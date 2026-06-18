//! Stage-1 review engine. Reads a `base...target` git diff, extracts symbol
//! boundaries with tree-sitter, and maps "changed lines ∩ innermost symbol" into
//! `ReviewCard[]` for the front-end.
//!
//! Dependency direction: lib -> mod -> {gitdiff, symbols, cards} -> model.
//! `build_review` is Tauri-independent (a pure function) so it can be called
//! directly from `cargo test`.

pub mod ai;
mod branches;
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
pub use clustercard::{build_cluster_cards, ChangedSymbolIn, ClusterCardInput};

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

    Ok(RelationAnalysis {
        changed,
        hints,
        seeds,
    })
}

/// The final Stage-④+⑤+⑥ layout: ordered, labelled clusters + the flat order, ready to be
/// folded into `ReviewData` by the IPC orchestrator (a later stage). Stage-1's `ReviewData`
/// stays untouched — this is the AI sidecar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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

    let cards = clustercard::build_cluster_cards(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
    );

    run_cluster_pipeline(provider, &cards, &analysis.hints)
        .await
        .map_err(|e| EngineError::Parse(format!("AI cluster pipeline failed: {e}")))
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
            .unwrap_or_else(|| ("Changes".to_string(), "Changes in this cluster.".to_string()));
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
