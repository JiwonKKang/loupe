//! Stage-② — relation signals + strong-seed first-pass clustering.
//!
//! Pure, deterministic, **no AI**. Given the changed symbols of a review (each a card
//! with an id/name/kind/path plus the references collected by
//! [`symbols::extract_with_refs`]), compute a coarse `RelationHints { strong, weak }`
//! over **changed-symbol pairs only** (planning §4.4). These are *signals*, not a call
//! graph: bare-name matching, not resolved edges. The AI layer (later stage) consumes
//! these hints; this module never decides ordering.
//!
//! ## M5 (v2-critique) — keep it minimal, two grades only
//! Exactly **strong / weak**. No "medium". The only tuning knob is the hub name-set
//! exclusion (planning §4.4 "hub/공통 노드 제외 룰 — 품질의 80%를 결정"). **No fan-in
//! threshold, no top-K** — those are deferred (they over-fit to one repo). Determinism
//! comes from pure functions over sorted input, so it stays cache-consistent without
//! any parameters to tune.
//!
//! ## B1 (v2-critique) — asymmetric reference buckets
//! `SymbolRefs` buckets are filled asymmetrically per language (Rust: no `type_refs`
//! from tags.scm — recovered via the header walk; Go: no `impls`). A missing bucket
//! means "this grammar can't express that signal", so we simply find no pairs from it —
//! never a false "no relation". Strong rules degrade gracefully per language.

use super::symbols::SymbolRefs;
use std::collections::BTreeSet;

/// hub / common-node name-set excluded from relation evidence (planning §4.4). These
/// names tie unrelated flows together, so they are **never** a relation basis. Matched
/// case-insensitively against the referenced identifier (the *callee/type* name), not
/// the owning symbol. Kept as a fixed set (M5 — no fan-in tuning this stage).
const HUB_NAMES: &[&str] = &[
    "Logger",
    "log",
    "DateUtils",
    "StringUtils",
    "JsonUtils",
    "ErrorCode",
    "BaseResponse",
    "CommonException",
    "Objects",
    "Optional",
    "Arrays",
];

fn is_hub(name: &str) -> bool {
    HUB_NAMES.iter().any(|h| h.eq_ignore_ascii_case(name))
}

/// One changed symbol, as the relation/seed layer sees it. Built from a `ReviewCard`
/// (the `id` is the stable card id) plus the references attributed to that symbol by
/// `extract_with_refs`. The orchestrator pairs each changed card with its `SymbolRefs`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedSymbol {
    /// Stable card id (the relation/seed output speaks in these).
    pub card_id: String,
    /// Bare symbol name (the match key for bare-name relations).
    pub name: String,
    /// Enclosing class/type name when known (e.g. Rust `impl` block type, Java class),
    /// used for "same class helper" relations. `None` for free functions.
    pub owner: Option<String>,
    /// repo-relative path (used for the weak "same file" signal).
    pub path: String,
    /// True when this symbol is a test (test→impl strong relation).
    pub is_test: bool,
    /// References attributed to this symbol by `extract_with_refs` (B1 asymmetric).
    pub refs: SymbolRefs,
    /// Identifiers that appear only via an import/use statement attributed to this
    /// symbol's file (weak "import-only" signal). Empty when unknown.
    pub imports: Vec<String>,
}

/// The relation hints over changed-symbol pairs (planning §6.1 `relationHints`).
/// Each pair is `(card_id, card_id)` with `a < b` lexicographically so a pair is stored
/// once and the whole structure is order-independent (deterministic).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationHints {
    /// Same-cluster-likely: direct call, signature-type, same-class helper, test→impl.
    pub strong: Vec<(String, String)>,
    /// Barely evidence: import-only, util/logger use, same file.
    pub weak: Vec<(String, String)>,
}

/// Compute relation hints over the changed symbols. Pure & deterministic: the output is
/// a function of the input's content (sorted internally), independent of input order.
///
/// Rules (planning §4.4, distilled to strong/weak — M5):
///  - **strong**
///    - direct call: a symbol's `calls` references another changed symbol's `name`.
///    - signature type: a header-`type_refs` hit (Go/Java/Rust-recovered) names another
///      changed symbol → likely Request→Command→Entity contract relation.
///    - impl: an `impls` reference (Rust/Java) names another changed symbol (trait/iface
///      ↔ implementor).
///    - same-class helper: two changed symbols sharing a non-empty `owner` (public/
///      private helpers of the same class).
///    - test→impl: a test symbol whose `calls` reference another changed symbol's name.
///  - **weak**
///    - import-only: a symbol `imports` another changed symbol's name but does not call
///      / type-use it.
///    - same file: two changed symbols in the same `path` not already strong-linked.
///  - **hub exclusion**: any referenced identifier in `HUB_NAMES` is dropped before
///    matching (Logger, Optional, …) so common nodes never create relations.
///  - **self exclusion**: a symbol never relates to itself.
///  - changed-symbols only: matches are against the changed-symbol name set, never the
///    whole codebase (this is a *signal*, not a call graph).
pub fn compute_relation_hints(changed: &[ChangedSymbol]) -> RelationHints {
    let mut strong: BTreeSet<(String, String)> = BTreeSet::new();
    let mut weak: BTreeSet<(String, String)> = BTreeSet::new();

    // Index changed-symbol names → the card_ids carrying that bare name (a name may be
    // duplicated across overloads/files; relate to all carriers). Hub names excluded.
    let mut by_name: std::collections::BTreeMap<&str, Vec<&ChangedSymbol>> =
        std::collections::BTreeMap::new();
    for cs in changed {
        by_name.entry(cs.name.as_str()).or_default().push(cs);
    }

    // --- strong: reference-based (call / type / impl) ---
    for cs in changed {
        // calls → direct-call (or test→impl when the caller is a test) strong relation.
        for hit in &cs.refs.calls {
            link_ref(&cs.card_id, &hit.ident, &by_name, &mut strong);
        }
        // header type uses → signature-type strong relation.
        for hit in &cs.refs.type_refs {
            if hit.in_header {
                link_ref(&cs.card_id, &hit.ident, &by_name, &mut strong);
            }
        }
        // impl/trait references → impl strong relation.
        for hit in &cs.refs.impls {
            link_ref(&cs.card_id, &hit.ident, &by_name, &mut strong);
        }
    }

    // --- strong: same-class helper (shared non-empty owner) ---
    for (i, a) in changed.iter().enumerate() {
        let Some(owner_a) = a.owner.as_deref() else {
            continue;
        };
        if owner_a.is_empty() || is_hub(owner_a) {
            continue;
        }
        for b in &changed[i + 1..] {
            if b.owner.as_deref() == Some(owner_a) {
                insert_pair(&mut strong, &a.card_id, &b.card_id);
            }
        }
    }

    // --- strong: method ↔ its enclosing changed type (Repository↔Entity family) ---
    // A changed method whose `owner` is the *name* of a changed type/class in this PR is
    // strongly related to that type (the type and its behaviour changed together). This
    // catches e.g. a changed enum + its `impl IntoResponse for Enum { fn into_response }`
    // method, which neither call, signature-type, nor method-to-method same-owner links.
    for cs in changed {
        let Some(owner) = cs.owner.as_deref() else {
            continue;
        };
        if owner.is_empty() || is_hub(owner) {
            continue;
        }
        if let Some(targets) = by_name.get(owner) {
            for t in targets {
                // Only when the target is the type *itself* (a free-standing definition,
                // owner=None), not another method of the same class (covered above).
                if t.card_id != cs.card_id && t.owner.is_none() {
                    insert_pair(&mut strong, &cs.card_id, &t.card_id);
                }
            }
        }
    }

    // --- weak: import-only & same file ---
    for cs in changed {
        for imp in &cs.imports {
            if is_hub(imp) {
                continue;
            }
            // import-only is weak; if a strong link already exists it is *not* demoted
            // (strong wins — we just skip adding the weak duplicate below at emit time).
            link_ref(&cs.card_id, imp, &by_name, &mut weak);
        }
    }
    for (i, a) in changed.iter().enumerate() {
        for b in &changed[i + 1..] {
            if a.path == b.path {
                insert_pair(&mut weak, &a.card_id, &b.card_id);
            }
        }
    }

    // A pair that is strong must not also appear in weak (strong subsumes).
    for p in &strong {
        weak.remove(p);
    }

    RelationHints {
        strong: strong.into_iter().collect(),
        weak: weak.into_iter().collect(),
    }
}

/// Resolve a referenced identifier against the changed-symbol name index and, for every
/// distinct changed symbol carrying that name (≠ self, ≠ hub), insert the pair.
fn link_ref(
    from_id: &str,
    ident: &str,
    by_name: &std::collections::BTreeMap<&str, Vec<&ChangedSymbol>>,
    out: &mut BTreeSet<(String, String)>,
) {
    if is_hub(ident) {
        return;
    }
    if let Some(targets) = by_name.get(ident) {
        for t in targets {
            if t.card_id != from_id {
                insert_pair(out, from_id, &t.card_id);
            }
        }
    }
}

/// Insert a canonicalized (a < b) pair so the set is order-independent and self-pairs
/// are impossible.
fn insert_pair(out: &mut BTreeSet<(String, String)>, a: &str, b: &str) {
    if a == b {
        return;
    }
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    out.insert((lo.to_string(), hi.to_string()));
}

// ---------------------------------------------------------------------------
// ②.5 strong-seed first-pass clustering (v2.1 핵심)
// ---------------------------------------------------------------------------

/// A first-pass seed cluster: the algorithm's *proposal* of a change grouping, built
/// from **strong relations only** (v2.1). Each seed is one connected component of the
/// strong-relation graph; an isolated changed symbol is its own singleton seed. Weak
/// relations are deliberately **not** seed evidence (they would manufacture wrong
/// seeds; they remain AI judging material only). The AI layer is free to merge/split/
/// move these (they are anchors, not verdicts).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seed {
    /// Deterministic id `"seed-<n>"` (n = 1-based index in the sorted seed list).
    pub id: String,
    /// Card ids in this seed, sorted (stable output).
    pub card_ids: Vec<String>,
}

/// Build first-pass seed clusters from changed cards + their strong relations.
///
/// **strong-only connected components**: union-find over `hints.strong`. Every changed
/// card is a node (so isolated cards become singleton seeds — nothing is dropped, the
/// "all changes are visible" invariant). Weak relations are ignored here.
///
/// Deterministic: `all_card_ids` is sorted, union order is the sorted strong-pair order,
/// and seeds are emitted sorted by their smallest member — identical input ⇒ identical
/// output (cache-consistent, no parameters).
pub fn seed_clusters(all_card_ids: &[String], hints: &RelationHints) -> Vec<Seed> {
    // Stable node ordering.
    let mut ids: Vec<String> = all_card_ids.to_vec();
    ids.sort();
    ids.dedup();

    let index: std::collections::BTreeMap<&str, usize> =
        ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();

    let mut uf = UnionFind::new(ids.len());

    // Union in sorted strong-pair order for determinism. Pairs are already canonical
    // (a < b) from `compute_relation_hints`; sort defensively in case of other callers.
    let mut strong = hints.strong.clone();
    strong.sort();
    for (a, b) in &strong {
        if let (Some(&ia), Some(&ib)) = (index.get(a.as_str()), index.get(b.as_str())) {
            uf.union(ia, ib);
        }
        // A strong pair naming a card outside `all_card_ids` is ignored (defensive).
    }

    // Group ids by their component root.
    let mut groups: std::collections::BTreeMap<usize, Vec<String>> =
        std::collections::BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        groups.entry(uf.find(i)).or_default().push(id.clone());
    }

    // Order components by their smallest member for a stable, human-sensible order.
    let mut comps: Vec<Vec<String>> = groups.into_values().collect();
    for c in &mut comps {
        c.sort();
    }
    comps.sort_by(|a, b| a.first().cmp(&b.first()));

    comps
        .into_iter()
        .enumerate()
        .map(|(i, card_ids)| Seed {
            id: format!("seed-{}", i + 1),
            card_ids,
        })
        .collect()
}

/// Minimal union-find (path-compression + union-by-size). Local to keep the seed step
/// dependency-free (no petgraph) — M5 "no over-engineering".
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression.
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        // Union by size; tie-break to the smaller index as root for determinism.
        let (big, small) = if self.size[ra] != self.size[rb] {
            if self.size[ra] >= self.size[rb] {
                (ra, rb)
            } else {
                (rb, ra)
            }
        } else if ra < rb {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[small] = big;
        self.size[big] += self.size[small];
    }
}

#[cfg(test)]
mod tests;
