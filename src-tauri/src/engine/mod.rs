//! Stage-1 review engine. Reads a `base...target` git diff, extracts symbol
//! boundaries with tree-sitter, and maps "changed lines ∩ innermost symbol" into
//! `ReviewCard[]` for the front-end.
//!
//! Dependency direction: lib -> mod -> {gitdiff, symbols, cards} -> model.
//! `build_review` is Tauri-independent (a pure function) so it can be called
//! directly from `cargo test`.

pub mod ai;
mod basesignals;
mod branches;
mod cache;
mod cards;
mod clustercard;
mod gitdiff;
mod model;
mod relations;
mod symbols;

pub use branches::list_branches;
// Re-exported for downstream/testing consumers of the contract type.
#[allow(unused_imports)]
pub use branches::Branches;
pub use model::ReviewData;
// Re-exported for downstream/testing consumers of the contract types.
#[allow(unused_imports)]
pub use model::{
    AnalysisState, ChangeType, Cluster, ClusterKind, DefinitionOverview, JitDefinition, ReviewCard,
    ReviewLine, Suggestion, SymbolKind,
};
// Stage-② relation/seed layer (pure, no AI). IPC wiring is deferred.
#[allow(unused_imports)]
pub use relations::{ChangedSymbol, RelationHints, Seed};
// Stage-③ cluster-card refinement (AI input prep, pure). IPC wiring is deferred.
#[allow(unused_imports)]
pub use clustercard::{
    build_cluster_cards, build_cluster_cards_with_signals, ChangedSymbolIn, ClusterCardInput,
    DeletedSymbolIn, RenamePairIn, SignatureChangeIn,
};
// Stage-② base-AST signals (deleted symbols / renames / signature changes). Pure, no AI.
#[allow(unused_imports)]
pub use basesignals::{DeletedSymbol, FileBaseSignals, RenamePair, SignatureChange};
// ⑦ SHA caching (planning §8.1/§8.2/§8.4; M2 Mutex<Connection>+WAL, M3 merge-base key).
#[allow(unused_imports)]
pub use cache::{card_hash, Cache, SCHEMA_VER};
#[allow(unused_imports)]
pub use gitdiff::DiffShas;

#[derive(Debug)]
pub enum EngineError {
    Git(git2::Error),
    Parse(String),
    Io(std::io::Error),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Git(e) => write!(f, "Git error: {}", e.message()),
            EngineError::Parse(s) => write!(f, "Parse error: {s}"),
            EngineError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<git2::Error> for EngineError {
    fn from(e: git2::Error) -> Self {
        EngineError::Git(e)
    }
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

/// Pure entry point: build the review payload from a repo + base/target refs.
pub fn build_review(
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<ReviewData, EngineError> {
    let diff = gitdiff::diff_three_dot(repo_path, base, target)?;

    let mut cards = Vec::new();
    for file in &diff {
        // Symbols only matter for non-binary, non-deleted, supported-language files.
        let symbols = if file.is_binary
            || file.status == gitdiff::FileStatus::Deleted
            || file.status == gitdiff::FileStatus::Added
        {
            Vec::new()
        } else {
            match symbols::Lang::from_path(&file.new_path) {
                // None (unsupported) or parser ERROR (Ok(None)) => empty symbol set
                // => file-level fallback inside build_file_cards.
                Some(lang) => match symbols::extract(lang, &file.new_source)? {
                    Some(syms) => syms,
                    None => Vec::new(),
                },
                None => Vec::new(),
            }
        };
        cards::build_file_cards(file, &symbols, &mut cards);
    }

    // Stage-1 fills only `cards`; every Stage-2 field is `Default` (empty) until the
    // AI overlay (`engine::ai` + later orchestrator) runs. The diff-render contract
    // (`cards`) is therefore unchanged for the front-end.
    Ok(ReviewData {
        cards,
        ..Default::default()
    })
}

/// Result of the Stage-② relation/seed analysis: the changed symbols (with their card
/// ids), the relation hints over changed-symbol pairs, and the strong-seed first-pass
/// clusters. **Pure algorithm — no AI.** Kept separate from `build_review`: Stage-1's
/// `ReviewData` contract is untouched; this is a sidecar the AI layer will consume later.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationAnalysis {
    /// One entry per changed symbol that the relation layer could place (card_id +
    /// name + owner + path). Ordered by card id for determinism.
    pub changed: Vec<relations::ChangedSymbol>,
    /// Strong/weak relation hints over changed-symbol pairs (planning §4.4, M5).
    pub hints: relations::RelationHints,
    /// strong-only connected-component seeds (v2.1 ②.5).
    pub seeds: Vec<relations::Seed>,
    /// Base-AST signals (deleted symbols / renames / signature changes) aggregated over
    /// all changed files (planning §2.1 base/head). Computed by parsing each file's
    /// `old_source` and diffing against the head symbols. Empty on new files / parser
    /// errors / no before→after differences (safe degradation).
    pub base_signals: basesignals::FileBaseSignals,
}

/// Stage-② entry point: from a repo + base/target refs, compute relation hints and
/// strong-seed first-pass clusters over the **changed symbols** of the diff. Separate
/// from [`build_review`] (Stage-1) and **does not** touch `ReviewData`. Pure &
/// deterministic given the repo state; no AI, no network, no cache.
///
/// Per changed file: extract symbols + references (`symbols::extract_with_refs`, B1
/// asymmetric buckets) and owners; align changed symbols to their Stage-1 card ids
/// (`cards::changed_symbols_for_relations`); build [`ChangedSymbol`]s; then
/// `compute_relation_hints` + `seed_clusters`. Files with parser ERROR / unsupported
/// language contribute no symbols and no relations (safe degradation).
pub fn analyze_relations(
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<RelationAnalysis, EngineError> {
    let diff = gitdiff::diff_three_dot(repo_path, base, target)?;

    let mut changed: Vec<relations::ChangedSymbol> = Vec::new();
    // Aggregate base-AST signals across all changed files (planning §2.1 base/head).
    let mut base_signals = basesignals::FileBaseSignals::default();

    for file in &diff {
        let Some(lang) = symbols::Lang::from_path(&file.new_path) else {
            continue; // unsupported language => no symbols/relations.
        };
        // extract_with_refs == None on parser ERROR/no-parse => safe degradation.
        let Some((syms, refs)) = symbols::extract_with_refs(lang, &file.new_source)? else {
            continue;
        };
        if syms.is_empty() {
            continue;
        }
        let owners = symbols::symbol_owners(lang, &file.new_source, &syms);

        // Changed symbols aligned to Stage-1 card ids (reuses cards.rs id logic).
        let changed_refs = cards::changed_symbols_for_relations(file, &syms);

        // Base-AST signals: parse `old_source` (base) and diff against the head symbols
        // (planning §2.1). The head side of a rename / signature change must be a *changed*
        // head symbol (so it carries a real card id); deleted symbols need no card id.
        // Skip when there is no base (added file) or a parse failure — `file_base_signals`
        // degrades to empty internally, but we also avoid the work on added/binary files.
        if !file.old_source.is_empty()
            && file.status == gitdiff::FileStatus::Modified
            && !file.is_binary
        {
            let head_changed: Vec<basesignals::HeadChanged> = changed_refs
                .iter()
                .map(|cr| basesignals::HeadChanged {
                    sym_idx: cr.sym_idx,
                    card_id: cr.card_id.as_str(),
                })
                .collect();
            let mut fsig = basesignals::file_base_signals(
                lang,
                &file.old_source,
                &file.new_source,
                &syms,
                &head_changed,
            );
            if basesignals::has_signals(&fsig) {
                basesignals::stamp_path(&mut fsig, &file.new_path);
                base_signals.deleted.extend(fsig.deleted);
                base_signals.renames.extend(fsig.renames);
                base_signals.signature_changes.extend(fsig.signature_changes);
            }
        }

        // Imports attributed file-wide (weak import-only signal). Best-effort, cheap.
        let imports = collect_imports(lang, &file.new_source);

        for cr in changed_refs {
            let sym = &syms[cr.sym_idx];
            changed.push(relations::ChangedSymbol {
                card_id: cr.card_id,
                name: sym.name.clone(),
                owner: owners.get(cr.sym_idx).cloned().flatten(),
                path: file.new_path.clone(),
                is_test: is_test_symbol(lang, &file.new_path, sym),
                refs: refs
                    .get(cr.sym_idx)
                    .cloned()
                    .unwrap_or_else(|| symbols::SymbolRefs {
                        sym_idx: cr.sym_idx,
                        ..Default::default()
                    }),
                imports: imports.clone(),
            });
        }
    }

    // Deterministic: sort changed symbols by card id before computing hints/seeds.
    changed.sort_by(|a, b| a.card_id.cmp(&b.card_id));

    let hints = relations::compute_relation_hints(&changed);
    let all_ids: Vec<String> = changed.iter().map(|c| c.card_id.clone()).collect();
    let seeds = relations::seed_clusters(&all_ids, &hints);

    // Deterministic aggregate order (files were visited in diff order; sort defensively).
    base_signals.deleted.sort_by(|a, b| a.id.cmp(&b.id));
    base_signals.renames.sort_by(|a, b| {
        a.to_card_id
            .cmp(&b.to_card_id)
            .then(a.from_name.cmp(&b.from_name))
    });
    base_signals
        .signature_changes
        .sort_by(|a, b| a.card_id.cmp(&b.card_id));

    Ok(RelationAnalysis {
        changed,
        hints,
        seeds,
        base_signals,
    })
}

/// The final Stage-④+⑤+⑥ layout: ordered, labelled clusters + the flat order, ready to be
/// folded into `ReviewData` by the IPC orchestrator (a later stage). Stage-1's `ReviewData`
/// stays untouched — this is the AI sidecar.
///
/// `Deserialize` is required so the ⑦ cache can round-trip a stored layout/fragment back
/// out of SQLite (`cache::Cache::get_layout` / `get_cluster`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterLayout {
    /// Clusters in inter-cluster order; each carries its title/summary/kind and the
    /// intra-cluster ordered card ids (`Cluster::ordered_card_ids`).
    pub clusters: Vec<Cluster>,
    /// Inter-cluster order = the ids of `clusters` in order (convenience / parity with
    /// `ReviewData.cluster_order`). The trailing `"__unclustered"` bucket is not listed
    /// here; it is rendered after all clusters from `unclustered`.
    pub cluster_order: Vec<String>,
    /// The full flatten order: every clustered card id (in cluster order, intra-order),
    /// then the unclustered bucket. The front-end's index source of truth.
    pub ordered_card_ids: Vec<String>,
    /// Card ids in the Unclustered bucket (§3.1 — always shown, never dropped).
    pub unclustered: Vec<String>,
    /// §6.3 display-only merge suggestions.
    pub merge_suggestions: Vec<Suggestion>,
    /// §6.3 display-only split suggestions.
    pub split_suggestions: Vec<Suggestion>,
}

/// Stage-③→④→⑤→⑥ entry point: refine the Stage-② analysis into AI cluster cards, then run
/// the full AI pipeline — clustering (seed correction) → ordering (intra/inter flow) →
/// title/summary labelling — and assemble the final [`ClusterLayout`]. **Pure
/// orchestration over an injected provider** — no IPC, no cache (both deferred). Stage-1's
/// `ReviewData` is untouched; this is the AI sidecar.
///
/// Pipeline (v2.1):
///  1. [`build_review`] → Stage-1 `cards` (the card-id source of truth / whitelist).
///  2. [`analyze_relations`] → strong-seed first-pass clusters + relation hints.
///  3. [`clustercard::build_cluster_cards`] → one refined `ClusterCardInput` per seed
///     (no raw diff; short summaries only — input-size defence).
///  4+5. **Small PR (≤ SMALL_PR_SYMBOLS): one combined call** clusters AND orders
///     ([`ai::steps::cluster_and_order_combined`], planning §4.1). **Big PR: two calls** —
///     [`ai::steps::cluster_step`] then [`ai::steps::order_step`]. Both are whitelist-
///     verified (hallucination reject; omitted ids absorbed; ordering = permutation).
///  6. [`ai::steps::label_step`] → ONE batched title/summary call for all clusters,
///     B1-safe + M4 token-checked (§6.2 / §8.4).
///
/// On AI failure (after each step's one retry) the `Err` propagates so a caller can fall
/// back; this function does not itself implement the layer-heuristic fallback (Stage-⑩).
pub async fn analyze_clusters(
    provider: &dyn ai::LlmProvider,
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<ClusterLayout, EngineError> {
    let review = build_review(repo_path, base, target)?;
    let analysis = analyze_relations(repo_path, base, target)?;

    let cards = clustercard::build_cluster_cards_with_signals(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
        &analysis.base_signals,
    );

    run_cluster_pipeline(provider, &cards, &analysis.hints)
        .await
        .map_err(|e| EngineError::Parse(format!("AI cluster pipeline failed: {e}")))
}

/// ⑦ **Cached** Stage-③→⑥ entry point (planning §8.1/§8.2/§8.4; v2-critique M2/M3).
///
/// Same result as [`analyze_clusters`] but with the SHA cache in front:
///  1. Resolve `(merge_base_sha, head_sha)` (M3 — merge-base, not base tip).
///  2. **Full-layout hit** (`review_layout` keyed by head): return immediately with **AI 0
///     calls** — the 5분→즉시 / §8.1 결정성 path (same head ⇒ byte-identical layout).
///  3. **Miss**: build the seed cards, hash each (`card_hash`, 부분 무효화 key), and split
///     them into cached seeds (`cluster_result` hit — reused, AI skipped even if head moved)
///     and uncached seeds (AI runs only for these). Per-seed fragments are merged into the
///     final layout, then every uncached fragment **and** the full layout are stored.
///
/// Stage-1's `ReviewData` is untouched (사이드카). The `cache_dir` is a parameter (tests pass
/// a tempdir; IPC will pass the app data dir later — deferred).
pub async fn analyze_clusters_cached(
    provider: &dyn ai::LlmProvider,
    cache: &cache::Cache,
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<ClusterLayout, EngineError> {
    // M3: the cache key base is the *merge-base* SHA (the actual 3-dot base), not base tip.
    let shas = gitdiff::resolve_shas(repo_path, base, target)?;

    // (2) Full-layout hit — same head ⇒ same order, AI 0 calls (§8.1 / §8.4).
    if let Some(layout) = cache.get_layout(repo_path, &shas.merge_base_sha, &shas.head_sha) {
        return Ok(layout);
    }

    // (3) Miss: prepare the seed cards (the per-seed 부분 무효화 unit).
    let review = build_review(repo_path, base, target)?;
    let analysis = analyze_relations(repo_path, base, target)?;
    let cards = clustercard::build_cluster_cards_with_signals(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
        &analysis.base_signals,
    );

    let layout = run_cluster_pipeline_cached(
        provider,
        cache,
        repo_path,
        &shas.merge_base_sha,
        &cards,
        &analysis.hints,
    )
    .await
    .map_err(|e| EngineError::Parse(format!("AI cluster pipeline failed: {e}")))?;

    // Store the assembled head layout so a re-open is the AI-0-call path.
    let _ = cache.put_layout(repo_path, &shas.merge_base_sha, &shas.head_sha, &layout);
    Ok(layout)
}

/// Run the pipeline **per seed card with the cache** (부분 무효화) and merge the per-seed
/// fragments into one layout. Each seed card is hashed; a `cluster_result` hit reuses its
/// stored fragment (no AI), a miss runs [`run_cluster_pipeline`] on just that one seed and
/// stores the fragment. Fragments are merged in seed (card) order — deterministic.
///
/// Running each seed independently is what makes a single seed's result reusable in
/// isolation when `head` moves: the AI's clustering for seed *i* depends only on seed *i*'s
/// content (`card_hash`), so an unchanged seed's fragment is bit-identical across heads.
async fn run_cluster_pipeline_cached(
    provider: &dyn ai::LlmProvider,
    cache: &cache::Cache,
    repo_path: &str,
    merge_base_sha: &str,
    cards: &[clustercard::ClusterCardInput],
    hints: &relations::RelationHints,
) -> Result<ClusterLayout, ai::LlmError> {
    let mut fragments: Vec<ClusterLayout> = Vec::with_capacity(cards.len());

    for card in cards {
        let hash = cache::card_hash(card);

        // 부분 무효화: a seed whose content (card_hash) is cached is reused — AI skipped.
        if let Some(fragment) = cache.get_cluster(repo_path, merge_base_sha, &hash) {
            fragments.push(fragment);
            continue;
        }

        // Miss: run the AI pipeline on this single seed, restricting the relation hints to
        // its members (so the per-seed run is self-contained / cache-stable).
        let one = std::slice::from_ref(card);
        let seed_hints = restrict_hints_to_cards(hints, one);
        let fragment = run_cluster_pipeline(provider, one, &seed_hints).await?;

        let _ = cache.put_cluster(repo_path, merge_base_sha, &hash, &fragment);
        fragments.push(fragment);
    }

    Ok(merge_fragments(fragments))
}

/// Restrict relation hints to pairs whose *both* endpoints are member card ids of `cards`
/// (so a single-seed pipeline run only sees its own intra-seed evidence). Deterministic.
fn restrict_hints_to_cards(
    hints: &relations::RelationHints,
    cards: &[clustercard::ClusterCardInput],
) -> relations::RelationHints {
    let members: std::collections::BTreeSet<&str> = cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.as_str()))
        .collect();
    let keep = |pairs: &[(String, String)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .filter(|(a, b)| members.contains(a.as_str()) && members.contains(b.as_str()))
            .cloned()
            .collect()
    };
    relations::RelationHints {
        strong: keep(&hints.strong),
        weak: keep(&hints.weak),
    }
}

/// Concatenate per-seed [`ClusterLayout`] fragments into one layout, preserving seed order
/// (deterministic — seeds are sorted).
///
/// Each fragment was AI-labelled independently, so two fragments can carry the **same**
/// cluster id (a model labels every single-seed run `"cluster-1"`). To avoid one swallowing
/// the other, cluster ids are **re-namespaced per fragment** (`s<frag>::<id>`) and every
/// reference to a cluster id — `cluster_order` and the suggestions' `cluster_ids` — is
/// remapped to match. Card-id lists (`ordered_card_ids`, `unclustered`) are concatenated
/// as-is (cards are disjoint across seeds, so they stay unique).
fn merge_fragments(fragments: Vec<ClusterLayout>) -> ClusterLayout {
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut cluster_order: Vec<String> = Vec::new();
    let mut ordered_card_ids: Vec<String> = Vec::new();
    let mut unclustered: Vec<String> = Vec::new();
    let mut merge_suggestions: Vec<Suggestion> = Vec::new();
    let mut split_suggestions: Vec<Suggestion> = Vec::new();

    for (fi, frag) in fragments.into_iter().enumerate() {
        // Per-fragment cluster-id remap so fragments can't collide on a shared label.
        let remap = |id: &str| format!("s{fi}::{id}");

        for mut c in frag.clusters {
            c.id = remap(&c.id);
            clusters.push(c);
        }
        for id in &frag.cluster_order {
            cluster_order.push(remap(id));
        }
        ordered_card_ids.extend(frag.ordered_card_ids);
        unclustered.extend(frag.unclustered);
        for mut s in frag.merge_suggestions {
            s.cluster_ids = s.cluster_ids.iter().map(|id| remap(id)).collect();
            merge_suggestions.push(s);
        }
        for mut s in frag.split_suggestions {
            s.cluster_ids = s.cluster_ids.iter().map(|id| remap(id)).collect();
            split_suggestions.push(s);
        }
    }

    ClusterLayout {
        clusters,
        cluster_order,
        ordered_card_ids,
        unclustered,
        merge_suggestions,
        split_suggestions,
    }
}

/// Run clustering → ordering → labelling over the prepared cluster cards and assemble the
/// final [`ClusterLayout`]. Separated from [`analyze_clusters`] so it can be unit-tested
/// with a mock provider on synthetic cards (no git / no network).
pub async fn run_cluster_pipeline(
    provider: &dyn ai::LlmProvider,
    cards: &[clustercard::ClusterCardInput],
    hints: &relations::RelationHints,
) -> Result<ClusterLayout, ai::LlmError> {
    use ai::steps;

    let whitelist = steps::whitelist_of(cards);

    // ④+⑤: small PR ⇒ one combined call; big PR ⇒ cluster then order (planning §4.1).
    let (clustering, ordering) = if steps::is_small_pr(cards) {
        steps::cluster_and_order_combined(provider, cards).await?
    } else {
        let clustering = steps::cluster_step(provider, cards).await?;
        let ordering = steps::order_step(provider, &clustering, hints, &whitelist).await?;
        (clustering, ordering)
    };

    // ⑥: batched title/summary over the ordered clusters.
    let label_inputs = build_label_inputs(&clustering, cards);
    let allowed_names = allowed_symbol_names(cards);
    let label_outcome = steps::label_step(provider, &label_inputs, &allowed_names).await?;

    Ok(assemble_layout(clustering, ordering, label_outcome, cards))
}

/// Build the labelling inputs (Stage-⑥) from the clustering result + the cluster cards:
/// one `LabelInput` per cluster carrying its kind and its changed symbols (name / kind /
/// change / short summary), resolved from the cards by card id.
fn build_label_inputs(
    clustering: &ai::steps::ClusterResult,
    cards: &[clustercard::ClusterCardInput],
) -> Vec<ai::steps::LabelInput> {
    use std::collections::BTreeMap;
    // card_id -> the changed-symbol context (name/kind/change/summary).
    let by_id: BTreeMap<&str, &clustercard::ChangedSymbolIn> = cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter())
        .map(|s| (s.card_id.as_str(), s))
        .collect();

    clustering
        .clusters
        .iter()
        .map(|c| ai::steps::LabelInput {
            cluster_id: c.cluster_id.clone(),
            kind: c.kind,
            changed_symbols: c
                .member_card_ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .map(|s| ai::steps::LabelSymbolIn {
                    name: s.name.clone(),
                    kind: s.kind,
                    change_type: s.change_type,
                    summary: s.summary.clone(),
                })
                .collect(),
        })
        .collect()
}

/// The bare-name whitelist for the M4 token check (Stage-⑥): every changed symbol's name.
fn allowed_symbol_names(cards: &[clustercard::ClusterCardInput]) -> std::collections::BTreeSet<String> {
    cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.name.clone()))
        .collect()
}

/// Fold the three AI outputs into the final [`ClusterLayout`]: order the clusters by the
/// ordering result's `clusterOrder`, attach each cluster's ordered card ids + title/
/// summary/kind, compute the flat `ordered_card_ids`, and pass through the suggestions.
fn assemble_layout(
    clustering: ai::steps::ClusterResult,
    ordering: ai::steps::OrderResult,
    labels: ai::steps::LabelOutcome,
    cards: &[clustercard::ClusterCardInput],
) -> ClusterLayout {
    use std::collections::BTreeMap;

    // cluster_id -> ordered card ids (from the ordering result).
    let order_by_cluster: BTreeMap<&str, &Vec<String>> = ordering
        .ordered_by_cluster
        .iter()
        .map(|oc| (oc.cluster_id.as_str(), &oc.card_ids))
        .collect();
    // cluster_id -> kind / type_hint (from the clustering result).
    let kind_by_cluster: BTreeMap<&str, ClusterKind> = clustering
        .clusters
        .iter()
        .map(|c| (c.cluster_id.as_str(), c.kind))
        .collect();
    // cluster_id -> (title, summary) from the labelling result.
    let label_by_cluster: BTreeMap<&str, (&str, &str)> = labels
        .labels
        .clusters
        .iter()
        .map(|l| (l.cluster_id.as_str(), (l.title.as_str(), l.summary.as_str())))
        .collect();
    // A card's algorithmic type-hint: take it from the first card whose seed kind we know.
    // (The clustering kind is the AI's final call; type_hint mirrors the algorithmic
    // guess that fed the cluster card. We reuse the AI kind as a safe default when the
    // per-cluster hint is not separable post-clustering.)
    let _ = cards;

    // Inter-cluster order: `clusterOrder` lists every clustered id exactly once (verifier
    // guarantees this). Any cluster missing from it (shouldn't happen) is appended.
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut ordered_ids: Vec<String> = Vec::new();
    for id in &ordering.cluster_order {
        if kind_by_cluster.contains_key(id.as_str()) && seen.insert(id.as_str()) {
            ordered_ids.push(id.clone());
        }
    }
    for c in &clustering.clusters {
        if seen.insert(c.cluster_id.as_str()) {
            ordered_ids.push(c.cluster_id.clone());
        }
    }

    let mut clusters: Vec<Cluster> = Vec::with_capacity(ordered_ids.len());
    let mut flat: Vec<String> = Vec::new();
    for id in &ordered_ids {
        let kind = kind_by_cluster.get(id.as_str()).copied().unwrap_or_default();
        let ordered_card_ids: Vec<String> = order_by_cluster
            .get(id.as_str())
            .map(|v| (*v).clone())
            .unwrap_or_default();
        flat.extend(ordered_card_ids.iter().cloned());
        let (title, summary) = label_by_cluster
            .get(id.as_str())
            .map(|(t, s)| (t.to_string(), s.to_string()))
            .unwrap_or_else(|| ("변경 사항".to_string(), "이 클러스터의 변경 사항입니다.".to_string()));
        clusters.push(Cluster {
            id: id.clone(),
            title,
            summary,
            kind,
            // type_hint mirrors the AI's final kind here (the per-seed algorithmic hint is
            // not preserved through clustering); kept for the debug/display contract.
            type_hint: kind,
            ordered_card_ids,
        });
    }

    // Unclustered bucket trails after all clusters (§3.1 — always shown).
    flat.extend(ordering.unclustered.iter().cloned());

    ClusterLayout {
        clusters,
        cluster_order: ordered_ids,
        ordered_card_ids: flat,
        unclustered: ordering.unclustered,
        merge_suggestions: labels.labels.merge_suggestions.iter().map(to_suggestion("merge")).collect(),
        split_suggestions: labels.labels.split_suggestions.iter().map(to_suggestion("split")).collect(),
    }
}

/// Map an AI suggestion (`SuggestionOut`) to the IPC `Suggestion` with a fixed kind label.
fn to_suggestion(kind: &'static str) -> impl Fn(&ai::steps::SuggestionOut) -> Suggestion {
    move |s| Suggestion {
        kind: kind.to_string(),
        cluster_ids: s.cluster_ids.clone(),
        reason: s.reason.clone(),
    }
}

/// Heuristic test detection (planning: test→impl strong relation). Name/path based and
/// deliberately coarse — language test conventions, no annotation parsing this stage:
///  - path contains a test marker (`_test.go`, `/test/`, `test_`, `Test.java`, `tests`),
///  - Go `func TestXxx`, Java `*Test`/`*Tests` class or a method starting with `test`,
///  - Rust functions in a file/path suggesting tests (we lack `#[test]` here).
fn is_test_symbol(lang: symbols::Lang, path: &str, sym: &symbols::Symbol) -> bool {
    let p = path.to_ascii_lowercase();
    let path_is_test = p.contains("/test/")
        || p.contains("/tests/")
        || p.ends_with("_test.go")
        || p.contains("test_")
        || p.contains("/__tests__/");
    let name = &sym.name;
    match lang {
        symbols::Lang::Go => path_is_test || name.starts_with("Test"),
        symbols::Lang::Java => {
            path_is_test
                || name.ends_with("Test")
                || name.ends_with("Tests")
                || name.starts_with("test")
        }
        symbols::Lang::Rust => path_is_test,
    }
}

/// Collect identifiers imported at file scope (weak "import-only" signal). Line-based and
/// intentionally shallow (no AST): the last path segment of each import/use is recorded.
/// Returns a deduped, sorted list. Best-effort — relations only use it as weak evidence.
fn collect_imports(lang: symbols::Lang, source: &str) -> Vec<String> {
    let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for raw in source.lines() {
        let line = raw.trim();
        let seg = match lang {
            symbols::Lang::Rust if line.starts_with("use ") => line
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .rsplit("::")
                .next(),
            symbols::Lang::Go if line.starts_with('"') && line.ends_with('"') => {
                line.trim_matches('"').rsplit('/').next()
            }
            symbols::Lang::Java if line.starts_with("import ") => line
                .trim_start_matches("import ")
                .trim_start_matches("static ")
                .trim_end_matches(';')
                .rsplit('.')
                .next(),
            _ => None,
        };
        if let Some(s) = seg {
            let s = s.trim().trim_matches('{').trim_matches('}').trim();
            // Skip wildcards / braces / empty.
            if !s.is_empty() && s != "*" && !s.contains(' ') {
                out.insert(s.to_string());
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod pipeline_tests;

#[cfg(test)]
#[path = "cache_pipeline_tests.rs"]
mod cache_pipeline_tests;

#[cfg(test)]
#[path = "cache_dearday_test.rs"]
mod cache_dearday_test;
