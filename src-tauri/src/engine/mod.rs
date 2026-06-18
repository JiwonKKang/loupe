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

/// Stage-③+④ entry point: refine the Stage-② analysis into AI cluster cards and run the
/// AI clustering (seed-correction) step. **Pure orchestration over an injected provider**
/// — no IPC, no cache (both deferred). Stage-1's `ReviewData` is untouched; this returns a
/// `ClusterResult` sidecar the orchestrator will fold into `ReviewData` in a later stage.
///
/// Pipeline (v2.1):
///  1. [`build_review`] → Stage-1 `cards` (the card-id source of truth / whitelist).
///  2. [`analyze_relations`] → strong-seed first-pass clusters + relation hints.
///  3. [`clustercard::build_cluster_cards`] → one refined `ClusterCardInput` per seed
///     (no raw diff; short summaries only — input-size defence).
///  4. [`ai::steps::cluster_step`] → AI merges/splits/places the seeds, verified against
///     the card-id whitelist (hallucination reject; omitted ids absorbed, no drop).
///
/// On AI failure (after the step's one retry) the `Err` propagates so a caller can fall
/// back; this function does not itself implement the layer-heuristic fallback (Stage-⑩).
pub async fn analyze_clusters(
    provider: &dyn ai::LlmProvider,
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<ai::steps::ClusterResult, EngineError> {
    let review = build_review(repo_path, base, target)?;
    let analysis = analyze_relations(repo_path, base, target)?;

    let cards = clustercard::build_cluster_cards(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
    );

    ai::steps::cluster_step(provider, &cards)
        .await
        .map_err(|e| EngineError::Parse(format!("AI clustering failed: {e}")))
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
