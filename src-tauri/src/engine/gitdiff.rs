//! git2 wrapping: a 3-dot (`base...target`) tree-to-tree diff, flattened into
//! per-file, per-line records. Knows only git2 — no tree-sitter, no serde.
//!
//! Output is a pure intermediate type (`FileDiff` / `DiffLine`) consumed by
//! `cards.rs`. The whole point of this layer is that `cards.rs` can be unit-tested
//! against synthetic `FileDiff` values without touching git2.

use super::EngineError;
use git2::{Diff, DiffFindOptions, DiffFormat, DiffOptions, Repository};
use std::path::Path;

/// One changed line within a file.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 1-base line number in the *new* file (None for `Del`).
    pub new_lineno: Option<u32>,
    /// 1-base line number in the *old* file (None for `Add`). Never exposed to the
    /// front-end — used only to slice `del` text out of the old blob.
    pub old_lineno: Option<u32>,
    /// Raw content, trailing newline/CR stripped, no +/- marker.
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Add,
    Del,
    Ctx,
}

/// One file's worth of changes.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    /// new-side path (== old_path unless renamed; for a delete this falls back to
    /// old_path so the card still has a usable path — M8).
    pub new_path: String,
    /// old-side path (== new_path unless renamed/added).
    pub old_path: String,
    /// Full text of the *new* file (target side), used by tree-sitter for symbol
    /// boundaries. Empty for a deleted or binary file.
    pub new_source: String,
    /// Full text of the *old* file (base side), used to recover `del` line text and
    /// to emit a deleted file as a del-only card (M8). Empty for an added file.
    pub old_source: String,
    pub status: FileStatus,
    pub is_binary: bool,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
}

/// The two SHAs that fully determine a 3-dot (`base...target`) diff — and therefore the
/// cache key (v2-critique M3). **`merge_base_sha` is the merge-base of (base, target), NOT
/// the base branch tip.** The 3-dot diff is `merge-base → target`, so the base branch tip
/// moving (while the merge-base stays put) does *not* change the diff; keying on the tip
/// would cause spurious cache misses. `head_sha` is the resolved target commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffShas {
    /// `merge_base(base, target)` — the actual base of the 3-dot diff (M3).
    pub merge_base_sha: String,
    /// Resolved `target` commit (the review's "head"; §8.2 cache unit = commit).
    pub head_sha: String,
}

/// Resolve `(merge_base_sha, head_sha)` for a `base...target` 3-dot range **without**
/// computing the diff (cheap — used by the cache layer to build the key before deciding
/// whether AI work is even needed). `merge_base_sha` is the merge-base (M3), not base tip.
pub fn resolve_shas(repo_path: &str, base: &str, target: &str) -> Result<DiffShas, EngineError> {
    let repo = Repository::open(repo_path)?;
    let base_oid = resolve_commit(&repo, base)?;
    let target_oid = resolve_commit(&repo, target)?;
    let merge_base_oid = repo.merge_base(base_oid, target_oid)?;
    Ok(DiffShas {
        merge_base_sha: merge_base_oid.to_string(),
        head_sha: target_oid.to_string(),
    })
}

/// Compute `base...target` (3-dot: target vs. their merge-base) as a list of
/// per-file diffs, renames detected, full old/new sources attached.
pub fn diff_three_dot(
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<Vec<FileDiff>, EngineError> {
    Ok(diff_three_dot_with_shas(repo_path, base, target)?.0)
}

/// Like [`diff_three_dot`] but also returns the [`DiffShas`] (merge-base + head) the diff
/// was computed against, so a caller (the cache layer) gets the exact cache-key SHAs from
/// the *same* repo open + merge-base resolution — no second `merge_base` round-trip and no
/// risk of the key drifting from the diff. `merge_base_sha` is the merge-base (M3).
pub fn diff_three_dot_with_shas(
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<(Vec<FileDiff>, DiffShas), EngineError> {
    let repo = Repository::open(repo_path)?;

    let base_oid = resolve_commit(&repo, base)?;
    let target_oid = resolve_commit(&repo, target)?;

    // 3-dot: diff target against the merge-base of (base, target).
    let merge_base_oid = repo.merge_base(base_oid, target_oid)?;
    let merge_base_commit = repo.find_commit(merge_base_oid)?;
    let target_commit = repo.find_commit(target_oid)?;

    let base_tree = merge_base_commit.tree()?;
    let target_tree = target_commit.tree()?;

    let mut opts = DiffOptions::new();
    // Full-file context: emit EVERY unchanged line as context so a card holds the whole file, not
    // just the hunks. The front-end folds the regions with no changes (collapsed by default) and
    // lets the reviewer expand any of them to read the surrounding code. (100k lines covers any
    // realistic source file; libgit2 just caps context at the available lines.)
    opts.context_lines(100_000);
    opts.include_typechange(true);

    let mut diff: Diff =
        repo.diff_tree_to_tree(Some(&base_tree), Some(&target_tree), Some(&mut opts))?;

    // Rename / copy detection so a renamed file is one card on the new path.
    let mut find_opts = DiffFindOptions::new();
    find_opts.renames(true).copies(true);
    diff.find_similar(Some(&mut find_opts))?;

    let files = build_file_diffs(&repo, &diff)?;
    let shas = DiffShas {
        merge_base_sha: merge_base_oid.to_string(),
        head_sha: target_oid.to_string(),
    };
    Ok((files, shas))
}

/// Resolve a ref / branch / SHA string to a commit Oid.
fn resolve_commit(repo: &Repository, rev: &str) -> Result<git2::Oid, EngineError> {
    let obj = repo.revparse_single(rev)?;
    let commit = obj.peel_to_commit()?;
    Ok(commit.id())
}

fn build_file_diffs(repo: &Repository, diff: &Diff) -> Result<Vec<FileDiff>, EngineError> {
    // First pass: discover the set of deltas (one per file) in order, with metadata.
    // We walk `diff.print(Patch)` to capture per-line origins together with the
    // delta index so we can group lines by file deterministically.
    let num_deltas = diff.deltas().len();
    let mut files: Vec<FileDiff> = Vec::with_capacity(num_deltas);

    for (idx, delta) in diff.deltas().enumerate() {
        let is_binary = is_delta_binary(&delta);
        let status = match delta.status() {
            git2::Delta::Added => FileStatus::Added,
            git2::Delta::Deleted => FileStatus::Deleted,
            _ => FileStatus::Modified,
        };

        let old_path = delta
            .old_file()
            .path()
            .map(path_to_string)
            .unwrap_or_default();
        let new_path = delta
            .new_file()
            .path()
            .map(path_to_string)
            .unwrap_or_default();

        // For a delete, new_path is empty — fall back to old_path so the card has a
        // usable path (M8). For an add, old_path is empty — fall back to new_path.
        let resolved_new = if new_path.is_empty() {
            old_path.clone()
        } else {
            new_path.clone()
        };
        let resolved_old = if old_path.is_empty() {
            new_path.clone()
        } else {
            old_path.clone()
        };

        let (new_source, old_source) = if is_binary {
            (String::new(), String::new())
        } else {
            (
                blob_text(repo, delta.new_file().id()),
                blob_text(repo, delta.old_file().id()),
            )
        };

        files.push(FileDiff {
            new_path: resolved_new,
            old_path: resolved_old,
            new_source,
            old_source,
            status,
            is_binary,
            lines: Vec::new(),
        });
        let _ = idx;
    }

    // Second pass: collect per-line records, attributed to the right delta index.
    // `diff.foreach`'s line callback gives us the line plus, via the closure
    // capture of the current delta, the file it belongs to. We instead use
    // `print` with the delta index resolved through `diff.get_delta`.
    let mut current_delta_idx: usize = 0;
    diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        // Find which file this delta corresponds to by matching new+old paths.
        // git2 yields deltas in the same order as `diff.deltas()`, so we advance a
        // cursor when the delta identity changes.
        let dnew = delta
            .new_file()
            .path()
            .map(path_to_string)
            .unwrap_or_default();
        let dold = delta
            .old_file()
            .path()
            .map(path_to_string)
            .unwrap_or_default();

        // Advance cursor to the file whose (old,new) raw paths match this delta.
        if !file_matches(&files, current_delta_idx, &dold, &dnew) {
            if let Some(found) = find_file_index(&files, &dold, &dnew) {
                current_delta_idx = found;
            }
        }

        let kind = match line.origin() {
            '+' => Some(LineKind::Add),
            '-' => Some(LineKind::Del),
            ' ' => Some(LineKind::Ctx),
            // 'F' (file header), 'H' (hunk header), '=','>','<' (binary/EOF) skipped.
            _ => None,
        };

        if let Some(kind) = kind {
            // from_utf8_lossy: never panic on non-UTF8 (M12 safety even for text).
            let raw = String::from_utf8_lossy(line.content());
            let content = raw.trim_end_matches(['\n', '\r']).to_string();
            if let Some(file) = files.get_mut(current_delta_idx) {
                file.lines.push(DiffLine {
                    kind,
                    new_lineno: line.new_lineno(),
                    old_lineno: line.old_lineno(),
                    content,
                });
            }
        }
        true
    })?;

    Ok(files)
}

/// Match a file slot against raw delta paths (empty-aware).
fn file_matches(files: &[FileDiff], idx: usize, dold: &str, dnew: &str) -> bool {
    match files.get(idx) {
        Some(f) => path_eq(&f.old_path, &f.new_path, dold, dnew),
        None => false,
    }
}

fn find_file_index(files: &[FileDiff], dold: &str, dnew: &str) -> Option<usize> {
    files
        .iter()
        .position(|f| path_eq(&f.old_path, &f.new_path, dold, dnew))
}

/// A delta's raw (possibly-empty) paths identify a file slot. Our slots store
/// fallback-resolved paths, so compare against both the stored value and the raw
/// (delete => dnew empty but slot.new_path == old).
fn path_eq(slot_old: &str, slot_new: &str, dold: &str, dnew: &str) -> bool {
    let old_ok = dold.is_empty() || dold == slot_old;
    let new_ok = dnew.is_empty() || dnew == slot_new;
    // At least one side must be a real, matching path to avoid two empties matching all.
    let any_real = (!dold.is_empty() && dold == slot_old) || (!dnew.is_empty() && dnew == slot_new);
    old_ok && new_ok && any_real
}

fn is_delta_binary(delta: &git2::DiffDelta) -> bool {
    delta.flags().contains(git2::DiffFlags::BINARY)
        || delta.old_file().is_binary()
        || delta.new_file().is_binary()
}

fn blob_text(repo: &Repository, oid: git2::Oid) -> String {
    if oid.is_zero() {
        return String::new();
    }
    match repo.find_blob(oid) {
        Ok(blob) => String::from_utf8_lossy(blob.content()).into_owned(),
        Err(_) => String::new(),
    }
}

fn path_to_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
