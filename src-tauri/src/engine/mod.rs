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
mod jitdef;
mod model;
mod progress;
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
    build_cluster_cards, build_cluster_cards_with_signals, build_file_seed_cards, ChangedSymbolIn,
    ClusterCardInput, DeletedSymbolIn, RenamePairIn, SignatureChangeIn,
};
// Stage-② base-AST signals (deleted symbols / renames / signature changes). Pure, no AI.
#[allow(unused_imports)]
pub use basesignals::{DeletedSymbol, FileBaseSignals, RenamePair, SignatureChange};
// ⑦ SHA caching (planning §8.1/§8.2/§8.4; M2 Mutex<Connection>+WAL, M3 merge-base key).
#[allow(unused_imports)]
pub use cache::{card_hash, Cache, SCHEMA_VER};
#[allow(unused_imports)]
pub use gitdiff::DiffShas;
// Streaming progress for the live AnalyzeScreen (cosmetic side-channel; lib.rs supplies a
// Tauri-emitting sink, tests pass the no-op `()`).
pub use progress::{Progress, ProgressCluster, ProgressSink};

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
    /// Per-card AI one-sentence summaries (Stage-⑥): `card_id -> 한국어 한 문장`. Cached with
    /// the layout so a cache hit restores them with **zero AI calls**. `fold_layout` copies
    /// each into the owning `ReviewCard.ai_summary`. A card absent from the map (label failed /
    /// the AI skipped it) keeps `ai_summary = None` (Optional — no B1 impact). `BTreeMap` for
    /// a deterministic, byte-stable serialization (§8.1).
    #[serde(default)]
    pub card_summaries: std::collections::BTreeMap<String, String>,
    /// ⑩ Fallback signal (planning §9): `true` when this layout was produced by the
    /// deterministic layer-heuristic fallback ([`build_fallback_layout`]) because the AI cluster
    /// pipeline failed, NOT by the AI. `fold_layout` reads it to set
    /// `analysis = AnalysisState::Fallback` (front-end shows the `'fallback'` banner) instead of
    /// `Done`. `#[serde(default)]` ⇒ `false` for any AI/cache-stored layout (which never sets it),
    /// so existing cached layouts deserialize as non-fallback (`Done`).
    #[serde(default)]
    pub fallback: bool,
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

    let cards = build_all_cluster_cards(&review, &analysis);

    // Non-streaming entry (no IPC sink): discard progress events.
    run_cluster_pipeline(provider, &cards, &analysis.hints, &())
        .await
        .map_err(|e| EngineError::Parse(format!("AI cluster pipeline failed: {e}")))
}

/// Build the **complete** AI clustering input: the symbol seed-cards (Stage-③, from the
/// strong-relation seeds) PLUS one synthetic singleton seed-card per **file-level** card
/// not already covered by a symbol seed (planning §4.3/§4.4 — Infra/Config topic
/// clustering). Without the file seeds, every symbol-less change (CI/CD, Cargo.*, Caddy,
/// yaml/toml, Dockerfile …) is outside the whitelist and the front-end renders it as a
/// "Unclustered" *failure* band; folding them in lets the AI group them by tool/topic.
///
/// `already_seeded` = every card id any symbol seed covers, so a card is never double-fed.
/// Deterministic: symbol cards keep their Stage-② order; file seeds follow in stable
/// Stage-1 card order. Pure — no IO.
fn build_all_cluster_cards(
    review: &ReviewData,
    analysis: &RelationAnalysis,
) -> Vec<clustercard::ClusterCardInput> {
    use std::collections::BTreeSet;
    let mut cards = clustercard::build_cluster_cards_with_signals(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
        &analysis.base_signals,
    );
    // Every card id any symbol seed already covers (so file seeds don't duplicate them).
    let already_seeded: BTreeSet<String> = cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.clone()))
        .collect();
    cards.extend(clustercard::build_file_seed_cards(&review.cards, &already_seeded));
    cards
}

/// ⑧ IPC entry point: the full Stage-1 + Stage-2 payload for the front-end.
///
/// Runs Stage-1 [`build_review`] (the `cards` diff-render contract, returned untouched), then
/// the ⑦-cached AI cluster pipeline ([`analyze_clusters_cached`]), and **folds** the resulting
/// [`ClusterLayout`] back onto the `ReviewData` the front-end already knows: it fills
/// `clusters` / `cluster_order` / `ordered_card_ids` / `unclustered` / `merge_suggestions` /
/// `split_suggestions`, stamps each card's `cluster_id`, records the `(head_sha, base_sha)`
/// cache markers, and sets `analysis = Done`. The `cards` themselves are never re-ordered —
/// `ordered_card_ids` carries the cluster flow order; the front-end flattens by it (m3 stable
/// id ⇒ same head = same order).
///
/// `cache_dir` is `<app_data_dir>/loupe` (the IPC layer passes it; tests can pass a tempdir).
/// The provider is injected (the IPC layer builds a `CliProvider` from the onboarding
/// setup-token). On AI failure the `Err` propagates so the caller can decide (the front-end
/// keeps showing the Stage-1 flat cards); this function does not itself apply the ⑩ fallback.
pub async fn analyze_review(
    provider: &dyn ai::LlmProvider,
    cache_dir: &std::path::Path,
    repo_path: &str,
    base: &str,
    target: &str,
    progress: &dyn ProgressSink,
) -> Result<ReviewData, EngineError> {
    // Stage-1: the diff-render cards (the card-id source of truth / whitelist).
    let mut review = build_review(repo_path, base, target)?;

    // Static prep is done (the cards exist) — tell the loader how many files changed.
    progress.emit(Progress::Static {
        files: distinct_file_count(&review),
    });

    // ⑦-cached AI cluster pipeline. The cache db lives under `<app_data_dir>/loupe`.
    let cache = cache::Cache::open_in_dir(cache_dir)
        .map_err(|e| EngineError::Parse(format!("cache open failed: {e}")))?;
    let layout = analyze_clusters_cached(provider, &cache, repo_path, base, target, progress).await?;

    // The 3-dot SHAs are the determinism markers the front-end shows / keys on.
    let shas = gitdiff::resolve_shas(repo_path, base, target)?;

    fold_layout(&mut review, layout, shas);

    // ⑨ JIT definition injection (planning §5): post-pass over the *final* `ordered_card_ids`
    // — slot a structured definition overview just before the first changed card that uses a
    // type/class/struct whose definition isn't itself in the diff. Pure & deterministic; on no
    // eligible target it is a no-op (jit_defs empty, order unchanged ⇒ identical behaviour).
    inject_jit_defs(&mut review, repo_path, base, target)?;
    Ok(review)
}

/// ⑨ Fold the JIT definition pass onto a fully-laid-out [`ReviewData`] (planning §5).
///
/// Recomputes the diff + changed symbols (pure, deterministic — the same inputs Stage-②
/// used) and resolves which referenced type/class/struct definitions are **not** themselves
/// changed but are needed to follow the flow. For each, a `kind == Definition` pseudo-card is
/// appended to `review.cards`, the structured overview is recorded in `review.jit_defs`, and
/// the card id is spliced into `review.ordered_card_ids` just before its first user.
///
/// Degradation: no eligible target ⇒ `jit_defs` stays empty, `cards`/`ordered_card_ids`
/// untouched (byte-identical to the pre-⑨ payload). Never drops or reorders existing cards.
fn inject_jit_defs(
    review: &mut ReviewData,
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<(), EngineError> {
    let diff = gitdiff::diff_three_dot(repo_path, base, target)?;
    let analysis = analyze_relations(repo_path, base, target)?;

    let mut injection =
        jitdef::compute_jit_defs(&diff, &analysis.changed, &review.ordered_card_ids);
    if injection.jit_defs.is_empty() {
        return Ok(()); // no eligible definition ⇒ no-op (identical to pre-⑨ behaviour).
    }

    // Stamp each definition pseudo-card with the cluster of the card it is injected *before*
    // (its first user). The ProgressSpine groups by `cluster_id` over consecutive items, so an
    // un-stamped definition would split its user's cluster run into a stray "기타 변경" group
    // sitting in the middle of the flow. Inheriting the user's cluster keeps the definition
    // visually attached to the change it explains — and `kind == Definition` still drives the
    // overview render, so this only affects spine grouping, not the panel. (`injected_before`
    // resolves to a real, already-laid-out card by construction; missing ⇒ leave unclustered.)
    let cluster_by_id: std::collections::HashMap<&str, Option<String>> = review
        .cards
        .iter()
        .map(|c| (c.id.as_str(), c.cluster_id.clone()))
        .collect();
    let before_by_def: std::collections::HashMap<&str, &str> = injection
        .jit_defs
        .iter()
        .map(|jd| (jd.id.as_str(), jd.injected_before.as_str()))
        .collect();
    for card in &mut injection.cards {
        if let Some(before) = before_by_def.get(card.id.as_str()) {
            if let Some(cid) = cluster_by_id.get(before) {
                card.cluster_id = cid.clone();
            }
        }
    }

    review.ordered_card_ids = jitdef::splice_ordered(&review.ordered_card_ids, &injection.jit_defs);
    review.cards.extend(injection.cards);
    review.jit_defs = injection.jit_defs;
    Ok(())
}

/// Distinct changed-file count, for the loader's "Scanning the diff · N files" line. Counts
/// unique card paths (a file with several changed symbols counts once).
fn distinct_file_count(review: &ReviewData) -> usize {
    review
        .cards
        .iter()
        .map(|c| c.path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// Fold a [`ClusterLayout`] onto a Stage-1 [`ReviewData`]: copy the cluster two-tier across,
/// stamp every card's `cluster_id` from the cluster that owns it, set the determinism markers
/// and `analysis = Done`. Pure (no IO) so it is unit-testable on synthetic input.
fn fold_layout(review: &mut ReviewData, layout: ClusterLayout, shas: gitdiff::DiffShas) {
    use std::collections::HashMap;

    // card_id -> owning cluster id (a card is in at most one cluster; unclustered ⇒ None).
    let mut owner: HashMap<&str, &str> = HashMap::new();
    for c in &layout.clusters {
        for cid in &c.ordered_card_ids {
            owner.insert(cid.as_str(), c.id.as_str());
        }
    }
    for card in &mut review.cards {
        card.cluster_id = owner.get(card.id.as_str()).map(|s| s.to_string());
        // Per-card AI one-sentence summary (Stage-⑥). `None` when the AI didn't produce one
        // for this card (label failed / skipped) — Optional, so B1 (statistical `summary`
        // non-empty) is unaffected.
        card.ai_summary = layout.card_summaries.get(card.id.as_str()).cloned();
    }

    review.clusters = layout.clusters;
    review.cluster_order = layout.cluster_order;
    review.ordered_card_ids = layout.ordered_card_ids;
    review.unclustered = layout.unclustered;
    review.merge_suggestions = layout.merge_suggestions;
    review.split_suggestions = layout.split_suggestions;
    review.head_sha = shas.head_sha;
    review.base_sha = shas.merge_base_sha;
    // ⑩ A layout flagged `fallback` came from the deterministic layer-heuristic builder (the AI
    // pipeline failed) — surface that to the front-end (`analysis === 'fallback'` banner) rather
    // than claiming a verified AI run. A normal AI/cache layout (`fallback == false`) ⇒ Done.
    review.analysis = if layout.fallback {
        AnalysisState::Fallback
    } else {
        AnalysisState::Done
    };
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
    progress: &dyn ProgressSink,
) -> Result<ClusterLayout, EngineError> {
    // M3: the cache key base is the *merge-base* SHA (the actual 3-dot base), not base tip.
    let shas = gitdiff::resolve_shas(repo_path, base, target)?;

    // (2) Full-layout hit — same head ⇒ same order, AI 0 calls (§8.1 / §8.4). No per-cluster
    // events fire (there is no AI work to stream); the loader transitions straight to done.
    if let Some(layout) = cache.get_layout(repo_path, &shas.merge_base_sha, &shas.head_sha) {
        return Ok(layout);
    }

    // (3) Miss: prepare the seed cards (the per-seed 부분 무효화 unit).
    let review = build_review(repo_path, base, target)?;
    let analysis = analyze_relations(repo_path, base, target)?;
    let cards = build_all_cluster_cards(&review, &analysis);

    // ⑩ AI failure → layer-heuristic fallback (planning §9). When the AI pipeline `Err`s
    // (call/timeout/parse failure, over-budget, or verify-then-retry exhausted), we do NOT
    // propagate the error: we degrade to the deterministic, "엉성하지만 항상 동작하는"
    // [`build_fallback_layout`] (strong seeds as clusters, layer-heuristic ordering). The
    // fallback layout is **never cached** (its `fallback` flag stays out of the cache; the next
    // open re-runs the AI so a transient failure isn't frozen into the SHA cache, §8.2 / parity
    // with `layout_is_cacheable`).
    let layout = match run_cluster_pipeline_cached(
        provider,
        cache,
        repo_path,
        &shas.merge_base_sha,
        &cards,
        &analysis.hints,
        progress,
    )
    .await
    {
        Ok(layout) => {
            // Store the assembled head layout so a re-open is the AI-0-call path — but ONLY if
            // its labels actually succeeded. A layout with a fallen-back cluster label is a
            // transient failure; caching it would freeze "변경 사항" and serve it on every re-open
            // (§ see `layout_is_cacheable`). On a fallen-back layout we skip the cache so the next
            // open re-runs.
            if layout_is_cacheable(&layout) {
                let _ = cache.put_layout(repo_path, &shas.merge_base_sha, &shas.head_sha, &layout);
            }
            layout
        }
        // The whole AI pipeline failed — degrade to the deterministic fallback (NOT cached).
        Err(_e) => build_fallback_layout(&cards),
    };
    Ok(layout)
}

/// Whether a freshly-produced layout is complete enough to **cache**. `label_one` never errors
/// the pipeline — on an AI/parse failure it returns the B1 fallback label
/// ([`ai::steps::FALLBACK_TITLE`] / [`ai::steps::FALLBACK_SUMMARY`]). That makes a *transiently
/// failed* labelling indistinguishable from a real one at the layout level, so without this
/// guard a one-off label failure (rate-limit, auth blip) would be cached and served forever
/// (SHA-cache hit ⇒ no re-run). We therefore refuse to cache a layout where ANY cluster's
/// title/summary is the fallback string — the next open re-runs and regenerates real labels.
/// (An empty PR has no clusters ⇒ cacheable; a genuine success has real titles ⇒ cacheable.)
fn layout_is_cacheable(layout: &ClusterLayout) -> bool {
    !layout.clusters.iter().any(|c| {
        c.title == ai::steps::FALLBACK_TITLE || c.summary == ai::steps::FALLBACK_SUMMARY
    })
}

/// Run the **whole-input** AI pipeline once with the cache in front (§8.2 캐싱).
///
/// Clustering is a *global* decision: the model must see **all** seed cards together to
/// merge/split/place them. Running the pipeline per seed (the previous design) atomized that
/// decision and made cross-seed merges structurally impossible — every seed became its own
/// cluster (or fell to unclustered), so the verified [`analyze_clusters`] merges never
/// appeared. We therefore run the pipeline over the **entire** card set, exactly like
/// [`analyze_clusters`], and cache the result at that grain.
///
/// The cache key is a single `cluster_result` row keyed by the **set hash** of all card
/// hashes ([`cards_set_hash`]). That hash is content-derived and head-independent, so the
/// "head moved but no seed content changed" case is still an AI-0-call hit (the property the
/// per-seed grain gave us). What we give up is *partial* invalidation — any seed content
/// change re-runs the whole pipeline — which is required for correctness, since the AI's
/// clustering of every seed depends on every other seed.
async fn run_cluster_pipeline_cached(
    provider: &dyn ai::LlmProvider,
    cache: &cache::Cache,
    repo_path: &str,
    merge_base_sha: &str,
    cards: &[clustercard::ClusterCardInput],
    hints: &relations::RelationHints,
    progress: &dyn ProgressSink,
) -> Result<ClusterLayout, ai::LlmError> {
    // Whole-input grain: one cache row for the entire clustering input. The key is the set
    // hash of every card's content hash (order-independent), reusing the `cluster_result`
    // table (§8.2). A hit ⇒ the AI is skipped even if `head` moved without touching content.
    let set_hash = cards_set_hash(cards);
    if let Some(layout) = cache.get_cluster(repo_path, merge_base_sha, &set_hash) {
        return Ok(layout);
    }

    // Miss: run the full pipeline over ALL cards at once (the global clustering decision),
    // converging on the same code path `analyze_clusters` uses. This is the path that streams
    // per-cluster review events to the loader.
    let layout = run_cluster_pipeline(provider, cards, hints, progress).await?;

    // Only cache a layout whose labels succeeded — a fallen-back label means a transient
    // failure that must not be frozen into the cache (see `layout_is_cacheable`).
    if layout_is_cacheable(&layout) {
        let _ = cache.put_cluster(repo_path, merge_base_sha, &set_hash, &layout);
    }
    Ok(layout)
}

/// The whole-input cache key: a SHA-256 over the **sorted** per-card content hashes
/// ([`cache::card_hash`]). Order-independent (cards are sorted before hashing) and
/// content-derived (head-independent), so the same set of seed contents always maps to the
/// same key regardless of seed discovery order or a head move that didn't touch content.
fn cards_set_hash(cards: &[clustercard::ClusterCardInput]) -> String {
    use sha2::{Digest, Sha256};
    let mut hashes: Vec<String> = cards.iter().map(cache::card_hash).collect();
    hashes.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"loupe-cards-set-v");
    hasher.update(cache::SCHEMA_VER.to_le_bytes());
    for h in &hashes {
        hasher.update(b"\0");
        hasher.update(h.as_bytes());
    }
    let digest = hasher.finalize();
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Run clustering → ordering → labelling over the prepared cluster cards and assemble the
/// final [`ClusterLayout`]. Separated from [`analyze_clusters`] so it can be unit-tested
/// with a mock provider on synthetic cards (no git / no network).
pub async fn run_cluster_pipeline(
    provider: &dyn ai::LlmProvider,
    cards: &[clustercard::ClusterCardInput],
    hints: &relations::RelationHints,
    progress: &dyn ProgressSink,
) -> Result<ClusterLayout, ai::LlmError> {
    use ai::steps;
    use futures_util::stream::{self, StreamExt};

    let whitelist = steps::whitelist_of(cards);

    // ④+⑤: small PR ⇒ one combined call; big PR ⇒ cluster then order (planning §4.1). As soon
    // as membership is known we tell the loader which clusters it will review (spinning) so the
    // queue rail can appear before the per-cluster reviews come back.
    let (clustering, ordering) = if steps::is_small_pr(cards) {
        let pair = steps::cluster_and_order_combined(provider, cards).await?;
        progress.emit(Progress::Clusters {
            clusters: progress_clusters(&pair.0, cards),
        });
        pair
    } else {
        let clustering = steps::cluster_step(provider, cards).await?;
        progress.emit(Progress::Clusters {
            clusters: progress_clusters(&clustering, cards),
        });
        let ordering = steps::order_step(provider, &clustering, hints, &whitelist).await?;
        (clustering, ordering)
    };

    // ⑥: review each cluster on its own AI call, bounded-concurrent, revealing each in the
    // loader the moment it finishes (`Reviewed`). Completion order is cosmetic — the final
    // layout is assembled from `clustering`/`ordering`, not from this order — so streaming the
    // labels per-cluster changes nothing about the result, only how the wait is shown.
    let label_inputs = build_label_inputs(&clustering, cards);
    let allowed_names = allowed_symbol_names(cards);
    let allowed_ref = &allowed_names;

    // cluster_id -> the set of member card ids the cluster actually owns. The per-card summary
    // fold whitelists each returned `cardId` against this so a hallucinated id is dropped (M4).
    let members_by_cluster: std::collections::BTreeMap<&str, std::collections::BTreeSet<&str>> =
        label_inputs
            .iter()
            .map(|inp| {
                (
                    inp.cluster_id.as_str(),
                    inp.changed_symbols.iter().map(|s| s.card_id.as_str()).collect(),
                )
            })
            .collect();

    let mut labels: Vec<ai::steps::ClusterLabel> = Vec::with_capacity(label_inputs.len());
    let mut suspicious: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    // Accumulated per-card AI summaries across all clusters (folded into the layout). A later
    // cluster never overwrites an earlier id (clusters partition the cards, so collisions
    // shouldn't happen; `entry`-keep makes it deterministic if they ever do).
    let mut card_summaries: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    // Build the per-cluster review futures eagerly (each borrows `provider`/`allowed_ref`), then
    // drive up to LABEL_CONCURRENCY at once. (Collecting the futures avoids a higher-ranked
    // lifetime error that a `.map(|inp| async move {…})` closure trips over.)
    let review_futs: Vec<_> = label_inputs
        .iter()
        .map(|inp| steps::label_one(provider, inp, allowed_ref))
        .collect();
    let mut reviews = stream::iter(review_futs).buffer_unordered(LABEL_CONCURRENCY);
    while let Some((label, bad)) = reviews.next().await {
        progress.emit(Progress::Reviewed {
            id: label.cluster_id.clone(),
            chapter: label.title.clone(),
        });
        if !bad.is_empty() {
            suspicious.insert(label.cluster_id.clone(), bad);
        }
        // Fold this cluster's per-card summaries: keep only ids that are real members of the
        // cluster (M4 whitelist) and non-empty summaries; first writer wins (determinism).
        if let Some(members) = members_by_cluster.get(label.cluster_id.as_str()) {
            for cs in &label.card_summaries {
                let s = cs.summary.trim();
                if !s.is_empty() && members.contains(cs.card_id.as_str()) {
                    card_summaries.entry(cs.card_id.clone()).or_insert_with(|| s.to_string());
                }
            }
        }
        labels.push(label);
    }

    let label_outcome = ai::steps::LabelOutcome {
        labels: ai::steps::LabelResult {
            clusters: labels,
            merge_suggestions: Vec::new(),
            split_suggestions: Vec::new(),
        },
        suspicious,
    };

    // All clusters reviewed → the final ordering/assembly pass.
    progress.emit(Progress::Final);
    Ok(assemble_layout(clustering, ordering, label_outcome, card_summaries, cards))
}

/// Max clusters reviewed concurrently in Stage-⑥. Each review is its own `claude` CLI call,
/// so this bounds how many subprocesses run at once (the rail fills in waves rather than all
/// at once on a large PR). Small enough to be gentle on the machine / rate limits.
const LABEL_CONCURRENCY: usize = 4;

/// Build the loader's provisional cluster list from the clustering result: each cluster's id,
/// a readable provisional chapter label (the real AI title arrives later per `Reviewed`), and
/// its member symbol display names.
fn progress_clusters(
    clustering: &ai::steps::ClusterResult,
    cards: &[clustercard::ClusterCardInput],
) -> Vec<ProgressCluster> {
    use std::collections::BTreeMap;
    let by_id: BTreeMap<&str, &clustercard::ChangedSymbolIn> = cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter())
        .map(|s| (s.card_id.as_str(), s))
        .collect();
    clustering
        .clusters
        .iter()
        .map(|c| ProgressCluster {
            id: c.cluster_id.clone(),
            chapter: provisional_chapter(&c.cluster_id),
            cards: c
                .member_card_ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .map(|s| s.name.clone())
                .collect(),
        })
        .collect()
}

/// Turn a cluster id slug (`auth-flow`, `error_type`) into a readable provisional chapter
/// (`Auth Flow`, `Error Type`) shown until the AI title replaces it.
fn provisional_chapter(id: &str) -> String {
    id.replace(['-', '_'], " ")
        .split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
                    // card_id + snippet feed the per-card AI summary (cardSummaries).
                    card_id: s.card_id.clone(),
                    name: s.name.clone(),
                    kind: s.kind,
                    change_type: s.change_type,
                    snippet: s.snippet.clone(),
                    // No statistical `summary` here: per-card line counts must not leak into
                    // the cluster summary (Issue A — cluster=intent, card=intent+snippet).
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
    card_summaries: std::collections::BTreeMap<String, String>,
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
        card_summaries,
        // AI-assembled layout — never the fallback. The ⑩ fallback builder sets this true.
        fallback: false,
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

// ===========================================================================
// ⑩ Layer-heuristic fallback layout (planning §9 — "엉성하지만 항상 동작")
// ===========================================================================
//
// When the AI cluster pipeline fails (call/timeout/parse, over-budget, verify-then-retry
// exhausted) we must NOT die — we degrade to a deterministic layout (§9.2: 1차 fallback =
// 파일경로 + 레이어 휴리스틱 정렬 controller→service→domain→repository→infra→test; DFS는 2차).
//
// Inputs are exactly the prepared `ClusterCardInput`s the AI would have received — each one
// already a **strong-seed** (or a file seed), with a stable `cluster_id` and its member
// `card_id`s. We reuse those seeds verbatim as the clusters (the strong relations the seeds
// encode are already "확실하게 묶을 수 있는" groupings — §9.3/v2.1), and only impose a
// deterministic order via the layer heuristic. No AI, no network, pure.
//
// Invariants kept (parity with the AI path):
//  - **no-drop / 결정성**: every whitelisted card id appears exactly once in `ordered_card_ids`.
//  - **fallback signal**: `fallback = true` ⇒ `fold_layout` sets `analysis = Fallback`.
//  - the layout is **not cacheable in practice** (the caller never caches a fallback), so a
//    later successful AI run regenerates the real layout.

/// Architectural layers in review-flow order (planning §1 / §9.2). A card's rank is the layer
/// it best matches by file path + symbol name; lower rank = earlier in the review flow.
/// `Other` (no match) sorts between `domain` and `repository` heuristics — see [`layer_rank`].
const LAYER_COUNT: usize = 7;

/// Map a (path, name) to a layer rank 0..LAYER_COUNT (planning §9.2 controller→…→test).
/// Pure name/path heuristic (lower-cased substring match), deliberately coarse — the goal is a
/// "그럴듯한" deterministic order, not a precise call graph. Order of the checks encodes
/// precedence (entrypoint markers win over generic ones):
///  0 controller/handler/route/api/endpoint/`main`/entrypoint
///  1 service/usecase/use_case/application/app-service
///  2 domain/model/entity/policy/aggregate/valueobject
///  3 repository/repo/dao/store/persistence/mapper
///  4 infra/config/client/gateway/adapter/migration/build/ci
///  5 test/spec/__tests__
///  6 everything else (Other) — trails the named layers but precedes nothing special
fn layer_rank(path: &str, name: &str) -> usize {
    let p = path.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    // test first: a test file/symbol is a test regardless of what else its path says.
    if p.contains("/test/")
        || p.contains("/tests/")
        || p.contains("/__tests__/")
        || p.contains("_test.")
        || p.contains("test_")
        || p.ends_with("_test.go")
        || p.contains(".test.")
        || p.contains(".spec.")
        || n.starts_with("test")
        || n.ends_with("test")
        || n.ends_with("tests")
        || n.ends_with("spec")
    {
        return 5;
    }
    let any = |hay: &str, needles: &[&str]| needles.iter().any(|x| hay.contains(x));
    // 0 — controller / handler / route / api / entrypoint.
    if any(&p, &["controller", "handler", "/routes/", "/route/", "/api/", "/endpoints/", "router"])
        || any(&n, &["controller", "handler"])
        || n == "main"
        || n.starts_with("handle")
    {
        return 0;
    }
    // 1 — service / usecase / application.
    if any(&p, &["service", "usecase", "use_case", "/application/", "app_service"])
        || any(&n, &["service", "usecase"])
    {
        return 1;
    }
    // 2 — domain / model / entity / policy.
    if any(&p, &["/domain/", "domain", "/model", "entity", "policy", "aggregate", "valueobject"])
        || any(&n, &["policy", "entity", "aggregate"])
    {
        return 2;
    }
    // 3 — repository / dao / store / persistence.
    if any(&p, &["repository", "/repo", "dao", "/store", "persistence", "mapper"])
        || any(&n, &["repository", "repo", "dao"])
    {
        return 3;
    }
    // 4 — infra / config / client / gateway / migration / build / ci.
    if any(
        &p,
        &[
            "infra", "/config", "config.", "client", "gateway", "adapter", "migration", ".sql",
            "/.github/", "dockerfile", "caddyfile", ".toml", ".yaml", ".yml",
        ],
    ) || any(&n, &["client", "gateway", "adapter", "config"])
    {
        return 4;
    }
    // 6 — unclassified (trails the named non-test layers, precedes tests already handled above).
    LAYER_COUNT - 1
}

/// A card's layer rank = the **minimum** layer rank over its member symbols (the earliest
/// review layer any of its members touches). Empty cards (defensive) rank `Other`.
fn card_layer_rank(card: &clustercard::ClusterCardInput) -> usize {
    card.changed_symbols
        .iter()
        .map(|s| layer_rank(card_path_of(card, s), &s.name))
        .min()
        .unwrap_or(LAYER_COUNT - 1)
}

/// A member symbol's path: the file-seed cards carry the path in `name`; symbol cards carry no
/// path on the `ChangedSymbolIn` (the path lives on the Stage-1 card, not here), so we fall back
/// to the symbol name for the path-based checks. Both feed `layer_rank` together with the name.
fn card_path_of<'a>(
    _card: &'a clustercard::ClusterCardInput,
    sym: &'a clustercard::ChangedSymbolIn,
) -> &'a str {
    // For file seeds the `name` *is* the path; for symbol seeds `name` is the bare symbol — in
    // both cases passing it as the "path" to `layer_rank` (alongside the name) is the cheapest
    // signal available here without re-threading the Stage-1 cards. The name-based checks in
    // `layer_rank` cover symbol cards; the path-based checks cover file cards.
    sym.name.as_str()
}

/// The deterministic layer-heuristic fallback (planning §9.2). Each input `ClusterCardInput` is
/// one strong seed (or file seed) → one cluster, ordered by layer then path; the card ids inside
/// each cluster are likewise layer-then-path sorted. Pure, no AI, no network.
///
/// `fallback = true` so `fold_layout` reports `AnalysisState::Fallback`. Every whitelisted card
/// id appears exactly once across the clusters (no-drop §3.1); there is no Unclustered bucket
/// here (every seed becomes a real cluster, however small) so nothing is lost.
fn build_fallback_layout(cards: &[clustercard::ClusterCardInput]) -> ClusterLayout {
    // Sort the seeds (clusters) by (min layer rank, representative path/name) for a stable,
    // review-sensible order. A seed's representative key is its lexicographically-smallest member
    // name so identical input ⇒ identical output (결정성).
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by(|&i, &j| {
        let (ci, cj) = (&cards[i], &cards[j]);
        card_layer_rank(ci)
            .cmp(&card_layer_rank(cj))
            .then_with(|| cluster_sort_key(ci).cmp(&cluster_sort_key(cj)))
            .then_with(|| ci.cluster_id.cmp(&cj.cluster_id))
    });

    let mut clusters: Vec<Cluster> = Vec::with_capacity(cards.len());
    let mut cluster_order: Vec<String> = Vec::with_capacity(cards.len());
    let mut flat: Vec<String> = Vec::new();
    // Guard the no-drop invariant: a card id is emitted at most once even if (defensively) it
    // appeared in two seeds.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for &i in &order {
        let card = &cards[i];

        // Intra-cluster order: members sorted by (layer rank, name) — same heuristic, finer grain.
        let mut members: Vec<&clustercard::ChangedSymbolIn> = card.changed_symbols.iter().collect();
        members.sort_by(|a, b| {
            layer_rank(card_path_of(card, a), &a.name)
                .cmp(&layer_rank(card_path_of(card, b), &b.name))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.card_id.cmp(&b.card_id))
        });
        let mut ordered_card_ids: Vec<String> = Vec::with_capacity(members.len());
        for m in members {
            if seen.insert(m.card_id.clone()) {
                ordered_card_ids.push(m.card_id.clone());
            }
        }
        // A seed whose every member was already emitted (defensive) contributes no cluster.
        if ordered_card_ids.is_empty() {
            continue;
        }
        flat.extend(ordered_card_ids.iter().cloned());

        clusters.push(Cluster {
            id: card.cluster_id.clone(),
            title: fallback_title(card),
            // Generic, honest summary — no AI ran, so we don't pretend to summarize intent.
            summary: FALLBACK_SUMMARY_KR.to_string(),
            kind: card.algorithmic_type_hint,
            type_hint: card.algorithmic_type_hint,
            ordered_card_ids,
        });
        cluster_order.push(card.cluster_id.clone());
    }

    ClusterLayout {
        clusters,
        cluster_order,
        ordered_card_ids: flat,
        // The fallback puts every seed into a cluster, so the Unclustered bucket is empty.
        unclustered: Vec::new(),
        merge_suggestions: Vec::new(),
        split_suggestions: Vec::new(),
        card_summaries: std::collections::BTreeMap::new(),
        // ⑩ the fallback signal — `fold_layout` turns this into `AnalysisState::Fallback`.
        fallback: true,
    }
}

/// The honest fallback cluster summary (no AI ran). Korean, parity with the labelling fallback.
const FALLBACK_SUMMARY_KR: &str = "자동 분석에 실패하여 파일 경로·레이어 기준으로 묶은 변경 사항입니다.";

/// A seed's deterministic sort key: its lexicographically smallest member name (display).
fn cluster_sort_key(card: &clustercard::ClusterCardInput) -> String {
    card.changed_symbols
        .iter()
        .map(|s| s.name.clone())
        .min()
        .unwrap_or_default()
}

/// Heuristic cluster title (no AI): the seed's representative symbol name, or the common file /
/// directory for a multi-member seed. Single member ⇒ that member's name; several members in one
/// file ⇒ the file's basename; otherwise the smallest member name. Never empty (B1 parity).
fn fallback_title(card: &clustercard::ClusterCardInput) -> String {
    let names: Vec<&str> = card.changed_symbols.iter().map(|s| s.name.as_str()).collect();
    match names.as_slice() {
        [] => FALLBACK_TITLE_KR.to_string(),
        [only] => basename(only).to_string(),
        _ => {
            // Several members: prefer a shared file basename (file seeds set name=path), else the
            // smallest member name as a stable representative.
            let basenames: std::collections::BTreeSet<&str> =
                names.iter().map(|n| basename(n)).collect();
            if basenames.len() == 1 {
                basenames.into_iter().next().unwrap_or(FALLBACK_TITLE_KR).to_string()
            } else {
                names.iter().min().copied().unwrap_or(FALLBACK_TITLE_KR).to_string()
            }
        }
    }
}

/// The last path segment of a `name`/path (`a/b/c.rs` → `c.rs`, a bare symbol → itself).
fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// Fallback title when a seed has no usable member name (defensive). Korean (B1 parity).
const FALLBACK_TITLE_KR: &str = "변경 사항";

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
