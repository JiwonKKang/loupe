//! Stage-② extension — **base AST signals** (planning §2.1 "diff만 보지 않는다 — base/head
//! 양쪽 코드를 본다", §4.4 relation signals).
//!
//! Stage-1/Stage-② parse only the **head** (`new_source`) of each changed file. That makes
//! three before→after facts invisible, because they are only legible by comparing the
//! **base** (`old_source`) symbol set against the head symbol set:
//!
//!  1. **deleted symbol** — a symbol present in base but gone from head while the *file*
//!     survives (head-only parsing sees nothing; the del lines re-anchor onto whatever
//!     head symbol now occupies that coordinate, so the deletion "disappears").
//!  2. **rename** — a symbol `X` vanished from base and a symbol `Y` appeared in head whose
//!     **normalized body** is identical (or whose signature matches and body overlaps
//!     heavily). Without this, a rename scatters into "delete X + add Y" and lands in two
//!     clusters.
//!  3. **signature change** — a symbol with the *same* name in base and head whose header
//!     (signature line) text differs: a before→after contract change.
//!
//! This module is **pure** (no git, no AI): it takes a language + both sources + the
//! already-parsed head symbols, parses the base with `symbols::extract`, and diffs the two
//! symbol sets by **bare name** (qualified == name in Stage-1, planning m4). Everything is
//! deterministic — identical (base, head) ⇒ identical signals (cache-consistent).
//!
//! ## Heuristics kept deliberately minimal (M5 spirit)
//! Rename similarity is **normalized-body-hash equality OR (identical normalized signature
//! AND ≥ [`RENAME_BODY_OVERLAP`] of normalized body lines shared)** — no token-by-token
//! edit-distance, no tunable thresholds beyond that single overlap ratio. Signature change
//! is plain header-text inequality after whitespace/`pub`-noise normalization. Matching is
//! 1:1 greedy in a deterministic order, so renames never double-count a body.

use super::symbols::{self, Lang, Symbol};
use std::collections::{BTreeMap, BTreeSet};

/// Minimum fraction of a candidate's normalized body lines that must be shared with the
/// other side for a *signature-matched* (but not body-identical) pair to count as a rename.
/// Body-identical pairs (hash equal) are renames regardless of this. One ratio, no tuning
/// zoo (M5). 0.6 = "most of the body is the same code, just re-headed".
const RENAME_BODY_OVERLAP: f64 = 0.6;

/// A symbol present in the base but absent from the head (the *file* still exists). Carries
/// only what the AI needs as a signal — a name, an old signature, a synthetic stable id.
/// **Not a head card id**: never enters the clustering whitelist (it is informational).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedSymbol {
    /// Synthetic, stable id `"deleted::<path>::<name>"` — distinct from any head card id.
    pub id: String,
    /// Bare symbol name as it was in the base.
    pub name: String,
    /// repo-relative path of the file the symbol was deleted from.
    pub path: String,
    /// The base signature (header line, normalized) — display + AI context.
    pub signature: String,
}

/// A detected rename `from → to`. The `to` side **is** a real head changed-symbol card id
/// (the new code is in the diff), so the AI can keep it with the old symbol's flow. The
/// `from` side is the deleted base symbol (no head card id).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePair {
    /// The base (old) symbol name that disappeared.
    pub from_name: String,
    /// The head changed-symbol **card id** the old symbol became (real, whitelisted).
    pub to_card_id: String,
    /// The head symbol name (display).
    pub to_name: String,
    /// repo-relative path (same file for both sides — renames are within one file here).
    pub path: String,
    /// How the match was made: `"body"` (identical normalized body) or `"signature"`
    /// (identical signature + heavy body overlap). Display/debug only.
    pub basis: &'static str,
}

/// A symbol whose signature (header line) changed between base and head while keeping its
/// name. The `card_id` is the real head changed-symbol card (whitelisted).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureChange {
    /// The head changed-symbol **card id** (real, whitelisted).
    pub card_id: String,
    /// Bare symbol name (unchanged across the signature change).
    pub name: String,
    /// repo-relative path.
    pub path: String,
    /// Normalized base signature (before).
    pub old_signature: String,
    /// Normalized head signature (after).
    pub new_signature: String,
}

/// All base-AST signals for one file. Empty (all three vecs empty) on a new file (no base),
/// a parser error, an unsupported language, or simply no before→after differences.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBaseSignals {
    pub deleted: Vec<DeletedSymbol>,
    pub renames: Vec<RenamePair>,
    pub signature_changes: Vec<SignatureChange>,
}

impl FileBaseSignals {
    fn is_empty(&self) -> bool {
        self.deleted.is_empty() && self.renames.is_empty() && self.signature_changes.is_empty()
    }
}

/// A head changed symbol as this module needs to see it: its bare name and the real card id
/// Stage-1 minted for it. (The orchestrator builds these from `changed_symbols_for_relations`.)
#[derive(Debug, Clone)]
pub struct HeadChanged<'a> {
    /// Index into the head `symbols` slice (so we can read its header/body from source).
    pub sym_idx: usize,
    /// The real Stage-1 card id.
    pub card_id: &'a str,
}

/// Compute the base-AST signals for one file.
///
/// Inputs:
///  - `lang`, `old_source` (base), `new_source` (head),
///  - `head_syms`: the head symbols (already parsed by the caller — no re-parse),
///  - `head_changed`: which head symbols actually changed (by `sym_idx`) + their card ids.
///
/// Steps (all deterministic):
///  1. Parse the base with `symbols::extract`. On `None`/error/empty base ⇒ a deleted-set
///     of nothing and no renames; only signature changes need the base, so they too are
///     empty. (Safe degradation — planning: parser error ⇒ empty, never faked.)
///  2. **deleted** = base names not in the head name-set.
///  3. **added** = head changed names not in the base name-set (rename candidates).
///  4. **rename** = pair each deleted `X` with an added `Y` whose normalized body is equal
///     (or signature equal + body overlap ≥ ratio); 1:1 greedy, deterministic order. A
///     matched `X` is removed from `deleted`, a matched `Y` is not reported as "just added".
///  5. **signature change** = names in *both* base and head whose normalized header differs
///     (only for head symbols that actually changed — we have their card id).
pub fn file_base_signals(
    lang: Lang,
    old_source: &str,
    new_source: &str,
    head_syms: &[Symbol],
    head_changed: &[HeadChanged<'_>],
) -> FileBaseSignals {
    // A new file (no base) or an unparsable base yields no base signals (safe degradation).
    let base_syms = match symbols::extract(lang, old_source) {
        Ok(Some(s)) if !s.is_empty() => s,
        _ => return FileBaseSignals::default(),
    };

    let head_lines: Vec<&str> = new_source.lines().collect();
    let base_lines: Vec<&str> = old_source.lines().collect();

    // Name → indices, both sides (a name may repeat; we keep all).
    let base_by_name = index_by_name(&base_syms);
    let head_by_name = index_by_name(head_syms);

    // card id lookup for changed head symbols (by sym_idx).
    let card_by_head_idx: BTreeMap<usize, &str> =
        head_changed.iter().map(|h| (h.sym_idx, h.card_id)).collect();

    // --- deleted candidates: base names with no head symbol of that name. ---
    // (Whole base symbols, in deterministic start_row order.)
    let mut deleted_idx: Vec<usize> = base_syms
        .iter()
        .enumerate()
        .filter(|(_, s)| !head_by_name.contains_key(s.name.as_str()))
        .map(|(i, _)| i)
        .collect();
    deleted_idx.sort_by_key(|&i| (base_syms[i].start_row, base_syms[i].name.clone()));

    // --- added candidates: head names with no base symbol of that name. ---
    let mut added_idx: Vec<usize> = head_syms
        .iter()
        .enumerate()
        .filter(|(_, s)| !base_by_name.contains_key(s.name.as_str()))
        .map(|(i, _)| i)
        .collect();
    added_idx.sort_by_key(|&i| (head_syms[i].start_row, head_syms[i].name.clone()));

    // --- rename matching: 1:1 greedy over (deleted X, added Y) in deterministic order. ---
    let mut renames: Vec<RenamePair> = Vec::new();
    let mut matched_added: BTreeSet<usize> = BTreeSet::new();
    let mut matched_deleted: BTreeSet<usize> = BTreeSet::new();

    for &di in &deleted_idx {
        let dsym = &base_syms[di];
        let d_body = normalized_body(&base_lines, dsym);
        let d_sig = normalized_signature(&base_lines, dsym);
        let d_hash = body_hash(&d_body);

        // Pick the first (deterministic) unmatched added symbol that the heuristic accepts.
        let mut best: Option<(usize, &'static str)> = None;
        for &ai in &added_idx {
            if matched_added.contains(&ai) {
                continue;
            }
            let asym = &head_syms[ai];
            let a_body = normalized_body(&head_lines, asym);
            // (a) identical normalized body ⇒ rename (strongest, hash compare).
            if body_hash(&a_body) == d_hash && !d_body.is_empty() {
                best = Some((ai, "body"));
                break;
            }
            // (b) identical signature + heavy body overlap ⇒ rename.
            let a_sig = normalized_signature(&head_lines, asym);
            if a_sig == d_sig
                && !d_sig.is_empty()
                && body_overlap(&d_body, &a_body) >= RENAME_BODY_OVERLAP
            {
                // Keep scanning for a body-identical match first (preferred), but remember
                // this as a fallback. We don't break so a later exact-body match can win.
                if best.is_none() {
                    best = Some((ai, "signature"));
                }
            }
        }

        if let Some((ai, basis)) = best {
            let asym = &head_syms[ai];
            // Only emit a rename when the head side is a *changed* symbol (has a card id);
            // otherwise we have no whitelisted id to point the AI at.
            if let Some(&card_id) = card_by_head_idx.get(&ai) {
                renames.push(RenamePair {
                    from_name: dsym.name.clone(),
                    to_card_id: card_id.to_string(),
                    to_name: asym.name.clone(),
                    path: String::new(), // filled by the caller (it owns the path).
                    basis,
                });
                matched_added.insert(ai);
                matched_deleted.insert(di);
            }
        }
    }

    // --- deleted (final): deleted candidates not consumed by a rename match. ---
    let deleted: Vec<DeletedSymbol> = deleted_idx
        .iter()
        .filter(|di| !matched_deleted.contains(di))
        .map(|&di| {
            let s = &base_syms[di];
            DeletedSymbol {
                id: String::new(), // path-qualified id filled by the caller.
                name: s.name.clone(),
                path: String::new(),
                signature: normalized_signature(&base_lines, s),
            }
        })
        .collect();

    // --- signature changes: same name both sides, header text differs. ---
    // Only for head symbols that changed (we need a real card id), matched to a base symbol
    // of the same name. When a name repeats, pair by sorted position (deterministic).
    let mut signature_changes: Vec<SignatureChange> = Vec::new();
    for h in head_changed {
        let hsym = &head_syms[h.sym_idx];
        let Some(base_idxs) = base_by_name.get(hsym.name.as_str()) else {
            continue; // not in base ⇒ added/rename, handled above.
        };
        // Pair this head symbol against the base symbol of the same name at the same rank
        // among equal-named siblings (deterministic; usually a single pair).
        let head_rank = rank_among(head_syms, &head_by_name, h.sym_idx);
        let Some(&bi) = base_idxs.get(head_rank).or_else(|| base_idxs.first()) else {
            continue;
        };
        let old_sig = normalized_signature(&base_lines, &base_syms[bi]);
        let new_sig = normalized_signature(&head_lines, hsym);
        if old_sig != new_sig && !old_sig.is_empty() && !new_sig.is_empty() {
            signature_changes.push(SignatureChange {
                card_id: h.card_id.to_string(),
                name: hsym.name.clone(),
                path: String::new(),
                old_signature: old_sig,
                new_signature: new_sig,
            });
        }
    }
    signature_changes.sort_by(|a, b| a.card_id.cmp(&b.card_id));

    FileBaseSignals {
        deleted,
        renames,
        signature_changes,
    }
}

/// Stamp the per-file `path` (and synthetic deleted ids) onto a freshly-computed
/// `FileBaseSignals`. Kept separate so `file_base_signals` stays path-agnostic and unit-
/// testable, while the orchestrator owns the real `new_path`.
pub fn stamp_path(signals: &mut FileBaseSignals, path: &str) {
    for d in &mut signals.deleted {
        d.path = path.to_string();
        d.id = format!("deleted::{}::{}", path, d.name);
    }
    for r in &mut signals.renames {
        r.path = path.to_string();
    }
    for s in &mut signals.signature_changes {
        s.path = path.to_string();
    }
}

/// True when a file contributed any base signal.
pub fn has_signals(signals: &FileBaseSignals) -> bool {
    !signals.is_empty()
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Map bare name → the symbol indices carrying it, sorted by start_row (deterministic).
fn index_by_name(syms: &[Symbol]) -> BTreeMap<&str, Vec<usize>> {
    let mut m: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, s) in syms.iter().enumerate() {
        m.entry(s.name.as_str()).or_default().push(i);
    }
    for v in m.values_mut() {
        v.sort_by_key(|&i| syms[i].start_row);
    }
    m
}

/// The 0-based rank of `idx` among the symbols sharing its name (sorted by start_row).
fn rank_among(syms: &[Symbol], by_name: &BTreeMap<&str, Vec<usize>>, idx: usize) -> usize {
    by_name
        .get(syms[idx].name.as_str())
        .and_then(|v| v.iter().position(|&i| i == idx))
        .unwrap_or(0)
}

/// The symbol's signature = its **header line** (the `start_row` line), normalized: trim,
/// collapse internal whitespace, drop a trailing `{`, and strip leading `pub`/visibility
/// noise so a pure visibility change is not flagged as a signature change. Body-agnostic.
fn normalized_signature(lines: &[&str], sym: &Symbol) -> String {
    let raw = lines.get(sym.start_row).copied().unwrap_or("");
    let mut s = collapse_ws(raw);
    // Drop a trailing opening brace (and any trailing whitespace before it).
    if let Some(stripped) = s.strip_suffix('{') {
        s = stripped.trim_end().to_string();
    }
    // Strip a leading visibility keyword (Rust `pub`/`pub(crate)`, Java `public`/`private`/
    // `protected`) so visibility-only edits don't read as signature changes.
    for kw in ["pub(crate)", "pub(super)", "pub", "public", "private", "protected"] {
        if let Some(rest) = s.strip_prefix(kw) {
            if rest.starts_with(' ') || rest.is_empty() {
                s = rest.trim_start().to_string();
                break;
            }
        }
    }
    s
}

/// The symbol's **normalized body**: every line in `[start_row+1, end_row]`, each trimmed
/// and internal-whitespace-collapsed, blank/comment-only lines dropped. Returns the list of
/// kept lines (used both for an order-insensitive hash and for overlap). The header line is
/// excluded so a pure rename (header identifier changes, body identical) hashes equal.
fn normalized_body(lines: &[&str], sym: &Symbol) -> Vec<String> {
    let start = sym.start_row + 1;
    let end = sym.end_row.min(lines.len().saturating_sub(1));
    let mut out: Vec<String> = Vec::new();
    if start > end {
        return out;
    }
    for raw in &lines[start..=end] {
        let line = collapse_ws(raw);
        if line.is_empty() || is_comment_only(&line) || line == "}" || line == "{" {
            continue;
        }
        out.push(line);
    }
    out
}

/// Order-insensitive content hash of a normalized body (sorted lines joined). Two bodies
/// that are the same set of code lines hash equal even if a couple of lines were reordered
/// — a deliberately forgiving rename signal (M5: simple, not edit-distance).
fn body_hash(body: &[String]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut sorted = body.to_vec();
    sorted.sort();
    let mut h = DefaultHasher::new();
    sorted.len().hash(&mut h);
    for l in &sorted {
        l.hash(&mut h);
    }
    h.finish()
}

/// Fraction of `a`'s lines (as a multiset, approximated by a set) present in `b`. Symmetric
/// enough for our purpose: used only to confirm a signature-matched pair shares most of its
/// body. Empty `a` ⇒ 0.0 (an empty body can only rename via the empty-body short-circuit,
/// which we forbid for the hash path too).
fn body_overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let bset: BTreeSet<&String> = b.iter().collect();
    let shared = a.iter().filter(|l| bset.contains(l)).count();
    shared as f64 / a.len() as f64
}

/// Collapse all runs of ASCII whitespace to a single space and trim the ends.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True for a line that is only a comment (`//…`, `/*…`, `*…`, `#…`). Used to drop comments
/// from the body so a comment-only edit doesn't break a rename match.
fn is_comment_only(line: &str) -> bool {
    line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
        || line.starts_with("*/")
        || line.starts_with('#')
}

#[cfg(test)]
#[path = "basesignals_tests.rs"]
mod tests;
