//! The core mapping: `FileDiff` + `Vec<Symbol>` => `Vec<ReviewCard>`.
//!
//! Calls neither git2 nor tree-sitter directly, so the whole algorithm is unit-
//! testable against synthetic inputs.
//!
//! Card-split rule (stage 1, deliberately simple — no line-count heuristics):
//!   - If a file has >= 2 *changed symbols*: one card per changed symbol, plus one
//!     file-level card for any change outside a symbol.
//!   - If a file has 0 or 1 changed symbols: one whole-file card.
//!   - A brand-new file (all additions) is always one whole-file card (M13).
//!   - Binary file => one "binary change" file card, no lines (M12).
//!   - Deleted file => one del-only file card from old_source (M8).
//!   - Unsupported language / parser ERROR => one whole-file card (caller passes an
//!     empty symbol slice).

use super::gitdiff::{DiffLine, FileDiff, FileStatus, LineKind};
use super::model::{ChangeType, ReviewCard, ReviewLine, SymbolKind, T_ADD, T_CTX, T_DEL};
use super::symbols::Symbol;
use std::collections::BTreeMap;

const FILE_SYMBOL: &str = "__file";

/// Append the cards for one file into `out`.
pub fn build_file_cards(file: &FileDiff, symbols: &[Symbol], out: &mut Vec<ReviewCard>) {
    // M12: binary file => summary-only file card, no lines.
    if file.is_binary {
        out.push(binary_card(file));
        return;
    }

    // M8: deleted file => del-only file-level card recovered from the old blob.
    if file.status == FileStatus::Deleted {
        out.push(deleted_file_card(file));
        return;
    }

    // M13: a brand-new file is always read top-to-bottom as one card.
    if file.status == FileStatus::Added {
        out.push(whole_file_card(file));
        return;
    }

    // Attribute each diff line (not hunk — M10) to its innermost symbol. ctx lines
    // are attributed too (so a symbol card can render leading/trailing context), and
    // del lines anchor to the preceding ctx line's new coordinate (M9).
    let attribution = attribute_changes(&file.lines, symbols);

    // Which symbols actually have a *change*? A symbol is "changed" only if an add or
    // del line is attributed to it — never on ctx alone. git diff emits context_lines
    // (3) around every hunk, so an unchanged symbol adjacent to a real change would
    // otherwise be attributed via those ctx lines and become a spurious "+0 −0" card.
    // ctx still rides along as display context inside a genuinely-changed symbol's
    // card, it just never grants card-worthiness on its own (core invariant).
    let mut changed_symbol_idxs: Vec<usize> = attribution
        .iter()
        .zip(file.lines.iter())
        .filter_map(|(a, l)| match (a, l.kind) {
            (Some(idx), LineKind::Add | LineKind::Del) => Some(*idx),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    changed_symbol_idxs.sort_by_key(|&i| symbols[i].start_row);

    let has_orphan_change = attribution
        .iter()
        .zip(file.lines.iter())
        .any(|(a, l)| a.is_none() && l.kind != LineKind::Ctx);

    // Simple split rule: >= 2 changed symbols => per-symbol cards + orphan file card.
    if changed_symbol_idxs.len() >= 2 {
        // Track id suffixes by qualified name using a STABLE key (start_row) so a
        // later re-ordering cannot change a card's id (M3).
        let dup_counts = duplicate_name_counts(symbols, &changed_symbol_idxs);

        for &sym_idx in &changed_symbol_idxs {
            let sym = &symbols[sym_idx];
            // M11: a symbol whose changes fall into hunks separated by a gap (git
            // omitted the unchanged middle) becomes one card per contiguous run so
            // the gutter never jumps (B2). A single-run symbol keeps its plain id.
            push_symbol_cards(file, sym, sym_idx, &attribution, &dup_counts, out);
        }
        if has_orphan_change {
            out.push(orphan_file_card(file, &attribution));
        }
        return;
    }

    // 0 or 1 changed symbols => one whole-file card. Defensive: git only emits files
    // that actually changed, but never emit a "+0 −0" whole-file card if (somehow) the
    // file carries only ctx lines — the no-empty-change-card invariant holds here too.
    let file_has_change = file
        .lines
        .iter()
        .any(|l| matches!(l.kind, LineKind::Add | LineKind::Del));
    if file_has_change {
        out.push(whole_file_card(file));
    }
}

/// One changed symbol of a file, paired with the **same stable card id** Stage-1 would
/// mint for it. Used by the Stage-② relation/seed layer so relations speak in real card
/// ids (no duplicated id logic, no drift). `sym_idx` indexes the caller's `symbols`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedSymbolRef {
    pub sym_idx: usize,
    pub card_id: String,
}

/// Return the changed symbols of one file (add/del attributed, ctx never counts) with
/// their stable card ids — the exact ids `build_file_cards` would emit for the
/// per-symbol case (≥2 changed symbols). For a file with 0/1 changed symbols Stage-1
/// emits a whole-file card, so this still reports the changed symbol(s) with their
/// per-symbol id; the relation layer only ever forms *pairs* (needs ≥2), so a lone
/// changed symbol yields no intra-file relation regardless. Multi-run (M11) symbols use
/// the plain (first-run) id here — relations are per-symbol, not per-run.
///
/// File-level concerns (binary/deleted/added) carry no symbol relations and return empty.
pub fn changed_symbols_for_relations(file: &FileDiff, symbols: &[Symbol]) -> Vec<ChangedSymbolRef> {
    if file.is_binary
        || file.status == FileStatus::Deleted
        || file.status == FileStatus::Added
        || symbols.is_empty()
    {
        return Vec::new();
    }

    let attribution = attribute_changes(&file.lines, symbols);
    let mut changed_symbol_idxs: Vec<usize> = attribution
        .iter()
        .zip(file.lines.iter())
        .filter_map(|(a, l)| match (a, l.kind) {
            (Some(idx), LineKind::Add | LineKind::Del) => Some(*idx),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    changed_symbol_idxs.sort_by_key(|&i| symbols[i].start_row);

    let dup_counts = duplicate_name_counts(symbols, &changed_symbol_idxs);
    changed_symbol_idxs
        .into_iter()
        .map(|sym_idx| ChangedSymbolRef {
            sym_idx,
            card_id: stable_symbol_id(file, &symbols[sym_idx], &dup_counts),
        })
        .collect()
}

/// For each diff line, the index of the symbol it belongs to (None = outside any
/// symbol / orphan). ctx lines that are not adjacent to a change are still
/// attributed so that the symbol's contiguous range is known.
fn attribute_changes(lines: &[DiffLine], symbols: &[Symbol]) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity(lines.len());
    // Track the most recent ctx/add new-coordinate row so a del can anchor to it (M9).
    let mut prev_new_row: Option<usize> = None;

    // Precompute, for del lines with no preceding ctx, the *following* new row.
    let next_new_row = compute_next_new_rows(lines);

    for (i, line) in lines.iter().enumerate() {
        let row = match line.kind {
            LineKind::Add | LineKind::Ctx => line.new_lineno.map(|n| (n as usize).saturating_sub(1)),
            LineKind::Del => {
                // M9: anchor to preceding new coordinate; else following; else None.
                prev_new_row.or(next_new_row[i])
            }
        };
        let sym = row.and_then(|r| innermost_symbol(symbols, r));
        out.push(sym);

        if matches!(line.kind, LineKind::Add | LineKind::Ctx) {
            if let Some(n) = line.new_lineno {
                prev_new_row = Some((n as usize).saturating_sub(1));
            }
        }
    }
    out
}

/// For each line index, the new-coordinate row of the next add/ctx line at or
/// after it (used to anchor leading del runs that have no preceding ctx — M9).
fn compute_next_new_rows(lines: &[DiffLine]) -> Vec<Option<usize>> {
    let mut next = vec![None; lines.len()];
    let mut seen: Option<usize> = None;
    for i in (0..lines.len()).rev() {
        if matches!(lines[i].kind, LineKind::Add | LineKind::Ctx) {
            if let Some(n) = lines[i].new_lineno {
                seen = Some((n as usize).saturating_sub(1));
            }
        }
        next[i] = seen;
    }
    next
}

/// Innermost (narrowest) symbol whose inclusive row range contains `row`.
fn innermost_symbol(symbols: &[Symbol], row: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_width = usize::MAX;
    for (i, s) in symbols.iter().enumerate() {
        if s.start_row <= row && row <= s.end_row {
            let width = s.end_row - s.start_row;
            if width < best_width {
                best_width = width;
                best = Some(i);
            }
        }
    }
    best
}

/// Count duplicate names among the changed symbols, so we can append a stable
/// suffix. Keyed by the bare `name` (NOT `qualified`) so the id key is independent of
/// the deferred `qualified` normalization (m4) — when `qualified` later becomes
/// "Class.method", card ids (and therefore caches) stay put. Value = sorted list of
/// start_rows (the stable disambiguator — M3).
fn duplicate_name_counts(
    symbols: &[Symbol],
    changed: &[usize],
) -> BTreeMap<String, Vec<usize>> {
    let mut map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &i in changed {
        map.entry(symbols[i].name.clone())
            .or_default()
            .push(symbols[i].start_row);
    }
    for v in map.values_mut() {
        v.sort_unstable();
    }
    map
}

// ---------------------------------------------------------------------------
// Card builders
// ---------------------------------------------------------------------------

/// Emit one or more cards for a symbol. The symbol's attributed diff lines are
/// split into contiguous runs (M11): two hunks of the same symbol separated by a
/// gap (git omitted the unchanged middle, so the new-coordinate jumps) become
/// separate cards so no card's gutter ever jumps (B2). The common single-run case
/// keeps the plain id and one card.
fn push_symbol_cards(
    file: &FileDiff,
    sym: &Symbol,
    sym_idx: usize,
    attribution: &[Option<usize>],
    dup_counts: &BTreeMap<String, Vec<usize>>,
    out: &mut Vec<ReviewCard>,
) {
    // The diff-line indices attributed to this symbol, in order.
    let mut idxs: Vec<usize> = attribution
        .iter()
        .enumerate()
        .filter_map(|(i, a)| if *a == Some(sym_idx) { Some(i) } else { None })
        .collect();
    idxs.sort_unstable();
    if idxs.is_empty() {
        return;
    }

    // Keep only runs that carry a real change. A run that is ctx-only (e.g. trailing
    // context of an earlier hunk that landed in a separate contiguous run) must not
    // become a "+0 −0" card — the same invariant as at the symbol level, enforced per
    // run. ctx still rides along inside change-bearing runs as display context.
    let runs: Vec<Vec<usize>> = contiguous_runs(&file.lines, &idxs)
        .into_iter()
        .filter(|run| {
            run.iter()
                .any(|&i| matches!(file.lines[i].kind, LineKind::Add | LineKind::Del))
        })
        .collect();
    let multi = runs.len() > 1;

    for run in &runs {
        let lines = render_lines(&run.iter().map(|&i| file.lines[i].clone()).collect::<Vec<_>>());
        let (adds, dels) = count_add_del(&lines);
        let summary = symbol_summary(&sym.qualified, adds, dels);

        // Stable id: same name appearing twice gets an @<pos> suffix (M3). When a
        // single symbol splits into multiple runs, disambiguate runs by the first
        // line's new-coordinate gutter (stable, order-independent — M11/M3).
        let mut id = stable_symbol_id(file, sym, dup_counts);
        if multi {
            let gutter = lines.first().map(|l| l.n).unwrap_or(0);
            id = format!("{}#{}", id, gutter);
        }

        let (adds_n, dels_n) = count_add_del(&lines);
        out.push(ReviewCard {
            id,
            chapter: basename(&file.new_path),
            symbol: sym.qualified.clone(),
            path: file.new_path.clone(),
            status: "pending".into(),
            summary,
            lines,
            // Stage-2: qualified mirrors `symbol` for now (real normalization is ②);
            // it is NOT part of the id, so it can change later without moving caches.
            qualified: sym.qualified.clone(),
            change_type: line_change_type(adds_n, dels_n),
            kind: SymbolKind::Function,
            ..Default::default()
        });
    }
}

/// Split ordered diff-line indices into contiguous runs. A run breaks when the
/// diff array is non-adjacent (an intervening line belongs to another symbol) or
/// when the *new* coordinate jumps (a hunk gap git did not emit). Del lines do not
/// advance the new coordinate, so a del run stays attached to its anchor.
fn contiguous_runs(lines: &[DiffLine], idxs: &[usize]) -> Vec<Vec<usize>> {
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    // Last seen *new* coordinate within the current run.
    let mut last_new: Option<u32> = None;

    for &i in idxs {
        let l = &lines[i];
        let breaks = if let Some(&prev) = cur.last() {
            // Non-adjacent in the diff array => another symbol's line sits between.
            let array_gap = i != prev + 1;
            // New-coordinate jump for add/ctx (del has no new coord).
            let coord_gap = match (l.kind, l.new_lineno, last_new) {
                (LineKind::Del, _, _) => false,
                (_, Some(n), Some(prev_n)) => n > prev_n + 1,
                _ => false,
            };
            array_gap || coord_gap
        } else {
            false
        };

        if breaks {
            runs.push(std::mem::take(&mut cur));
            last_new = None;
        }
        if matches!(l.kind, LineKind::Add | LineKind::Ctx) {
            if let Some(n) = l.new_lineno {
                last_new = Some(n);
            }
        }
        cur.push(i);
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

/// Render all of a file's diff lines.
fn render_lines(lines: &[DiffLine]) -> Vec<ReviewLine> {
    let mut out = Vec::with_capacity(lines.len());
    // Monotonic gutter: a del line reuses the last emitted gutter number so the
    // column never jumps backwards (B2). Seed from the first line that has a new
    // coordinate.
    let mut last_gutter: u32 = lines
        .iter()
        .find_map(|l| l.new_lineno)
        .map(|n| n.saturating_sub(1))
        .unwrap_or(0);

    for line in lines {
        let (t, n) = match line.kind {
            LineKind::Add => {
                let n = line.new_lineno.unwrap_or(last_gutter + 1);
                (T_ADD, n)
            }
            LineKind::Ctx => {
                let n = line.new_lineno.unwrap_or(last_gutter + 1);
                (T_CTX, n)
            }
            LineKind::Del => {
                // B2: del carries the preceding gutter number (monotonic), never old_lineno.
                (T_DEL, last_gutter)
            }
        };
        if line.kind != LineKind::Del {
            last_gutter = n;
        }
        out.push(ReviewLine {
            n,
            t,
            c: line.content.clone(),
        });
    }
    out
}

/// Whole-file card: all changed lines + their context, single card.
fn whole_file_card(file: &FileDiff) -> ReviewCard {
    let lines = render_lines(&file.lines);
    let (adds, dels) = count_add_del(&lines);
    let summary = file_summary(&file.new_path, file.status, adds, dels);
    let name = basename(&file.new_path);
    ReviewCard {
        id: file_id(&file.new_path),
        chapter: name.clone(),
        symbol: name.clone(),
        path: file.new_path.clone(),
        status: "pending".into(),
        summary,
        lines,
        qualified: name,
        change_type: file_change_type(file.status),
        kind: SymbolKind::File,
        ..Default::default()
    }
}

/// File-level card for changes that fell outside any symbol (M4 orphan), when the
/// file was otherwise split per-symbol.
fn orphan_file_card(file: &FileDiff, attribution: &[Option<usize>]) -> ReviewCard {
    let orphan_lines: Vec<DiffLine> = file
        .lines
        .iter()
        .zip(attribution.iter())
        .filter_map(|(l, a)| if a.is_none() { Some(l.clone()) } else { None })
        .collect();
    let lines = render_lines(&orphan_lines);
    let (adds, dels) = count_add_del(&lines);
    let summary = file_summary(&file.new_path, file.status, adds, dels);
    let name = basename(&file.new_path);
    ReviewCard {
        id: file_id(&file.new_path),
        chapter: name.clone(),
        symbol: name.clone(),
        path: file.new_path.clone(),
        status: "pending".into(),
        summary,
        lines,
        qualified: name,
        change_type: file_change_type(file.status),
        kind: SymbolKind::File,
        ..Default::default()
    }
}

/// M8: a deleted file rendered as a del-only card from the old blob.
fn deleted_file_card(file: &FileDiff) -> ReviewCard {
    // Prefer the diff's own del lines (they carry old_lineno); if the diff omitted
    // them (e.g. very large), fall back to the whole old source.
    let lines: Vec<ReviewLine> = if file.lines.iter().any(|l| l.kind == LineKind::Del) {
        render_del_lines(&file.lines)
    } else {
        file.old_source
            .lines()
            .enumerate()
            .map(|(i, c)| ReviewLine {
                n: (i as u32) + 1,
                t: T_DEL,
                c: c.to_string(),
            })
            .collect()
    };
    let dels = lines.iter().filter(|l| l.t == T_DEL).count();
    let summary = format!(
        "Removes {}: −{} line{}.",
        basename(&file.old_path),
        dels,
        plural(dels)
    );
    let name = basename(&file.old_path);
    ReviewCard {
        id: file_id(&file.old_path),
        chapter: name.clone(),
        symbol: name.clone(),
        path: file.old_path.clone(),
        status: "pending".into(),
        summary,
        lines,
        qualified: name,
        change_type: ChangeType::Deleted,
        kind: SymbolKind::File,
        ..Default::default()
    }
}

/// Render only del lines with monotonic old-coordinate gutter numbers. Used for a
/// deleted file where there is no "new" side at all; here the gutter is the old
/// line number (the only sensible number) but still monotonic.
fn render_del_lines(lines: &[DiffLine]) -> Vec<ReviewLine> {
    lines
        .iter()
        .filter(|l| l.kind == LineKind::Del)
        .map(|l| ReviewLine {
            n: l.old_lineno.unwrap_or(0),
            t: T_DEL,
            c: l.content.clone(),
        })
        .collect()
}

/// M12: binary file card — summary only, no lines.
fn binary_card(file: &FileDiff) -> ReviewCard {
    let verb = match file.status {
        FileStatus::Added => "Adds binary file",
        FileStatus::Deleted => "Removes binary file",
        FileStatus::Modified => "Changes binary file",
    };
    let name = basename(&file.new_path);
    ReviewCard {
        id: file_id(&file.new_path),
        chapter: name.clone(),
        symbol: name.clone(),
        path: file.new_path.clone(),
        status: "pending".into(),
        summary: format!("{} {}.", verb, name),
        lines: Vec::new(),
        qualified: name,
        change_type: file_change_type(file.status),
        kind: SymbolKind::File,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stable card id from `path + name (+ @pos)`. Keyed by the bare `name` and the
/// symbol's `start_row` position, NEVER by `qualified` (m4): the deferred
/// qualified-name normalization (Stage ②) must not move existing ids or invalidate
/// caches. While `qualified == name` (Stage-1) this yields exactly the historical
/// ids, so the 26 existing tests are unaffected.
fn stable_symbol_id(
    file: &FileDiff,
    sym: &Symbol,
    dup_counts: &BTreeMap<String, Vec<usize>>,
) -> String {
    let base = format!("{}::{}", file.new_path, sym.name);
    match dup_counts.get(&sym.name) {
        // Suffix only when the same name appears more than once. The suffix is the
        // 0-base position of this symbol's start_row within the sorted start_rows
        // of its same-named siblings — a stable key invariant to card ordering (M3).
        Some(rows) if rows.len() > 1 => {
            let pos = rows.iter().position(|&r| r == sym.start_row).unwrap_or(0);
            format!("{}@{}", base, pos)
        }
        _ => base,
    }
}

fn file_id(path: &str) -> String {
    format!("{}::{}", path, FILE_SYMBOL)
}

/// Derive a symbol-card `change_type` from its add/del line counts: pure additions =>
/// Added, pure deletions => Deleted, otherwise Modified. (Stage-2 metadata only; the
/// statistical `summary` is unaffected.)
fn line_change_type(adds: usize, dels: usize) -> ChangeType {
    match (adds > 0, dels > 0) {
        (true, false) => ChangeType::Added,
        (false, true) => ChangeType::Deleted,
        _ => ChangeType::Modified,
    }
}

/// Map a file-level `FileStatus` onto the card `change_type`.
fn file_change_type(status: FileStatus) -> ChangeType {
    match status {
        FileStatus::Added => ChangeType::Added,
        FileStatus::Deleted => ChangeType::Deleted,
        FileStatus::Modified => ChangeType::Modified,
    }
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn count_add_del(lines: &[ReviewLine]) -> (usize, usize) {
    let adds = lines.iter().filter(|l| l.t == T_ADD).count();
    let dels = lines.iter().filter(|l| l.t == T_DEL).count();
    (adds, dels)
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// B1: summary is ALWAYS non-empty and starts with a capital letter.
fn symbol_summary(name: &str, adds: usize, dels: usize) -> String {
    format!(
        "Updates {}: +{} −{} line{}.",
        name,
        adds,
        dels,
        plural(adds + dels)
    )
}

fn file_summary(path: &str, status: FileStatus, adds: usize, dels: usize) -> String {
    let name = basename(path);
    match status {
        FileStatus::Added => format!("Adds {}: +{} line{}.", name, adds, plural(adds)),
        FileStatus::Deleted => format!("Removes {}: −{} line{}.", name, dels, plural(dels)),
        FileStatus::Modified => {
            format!("Updates {}: +{} −{} line{}.", name, adds, dels, plural(adds + dels))
        }
    }
}
