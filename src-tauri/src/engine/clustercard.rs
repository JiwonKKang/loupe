//! Stage-③ — cluster card refinement (AI **input** preparation).
//!
//! Turns the Stage-② relation analysis (`Seed`s + `RelationHints` + `ChangedSymbol`s)
//! plus the Stage-1 `ReviewCard`s into `ClusterCardInput[]` — the *refined* card the AI
//! clustering step (Stage-④) consumes. **This is not raw diff** (planning §2.1/§6.1): the
//! AI never sees diff text here, only a compressed, symbol-level summary card.
//!
//! ## v2.1 — the unit of input is a *seed*, not a single symbol
//! The first-pass clustering unit is a **strong-seed** (v2.1 ②.5): one card per seed.
//! Each card carries the seed's changed symbols, the relation-hint pairs *within* the
//! seed, an algorithmic cluster-kind guess, entry-point candidates, contract-change
//! heuristics, and related tests. The AI receives M seed-cards (M ≪ N symbols) and is
//! told they are *proposals* it may freely merge/split/move (Stage-④ prompt).
//!
//! ## Identifiers are card ids (M4 whitelist key)
//! Every symbol is addressed by its **stable `card_id`**; `name` rides along for display
//! only. The whitelist/flatten key is the card id, so the AI must speak in card ids and
//! the verifier (`ai::verify`) checks every returned id against the card-id whitelist.
//!
//! ## Input-size defence
//! No symbol body is sent. Each changed symbol carries the short, already-computed
//! Stage-1 `summary` (statistical, B1-safe), never the raw hunk. Big PRs therefore stay
//! within token budget by construction (the diff never enters the prompt here).

use super::model::{ChangeType, ClusterKind, ReviewCard, SymbolKind};
use super::relations::{ChangedSymbol, RelationHints, Seed};
use std::collections::{BTreeMap, BTreeSet};
use serde::Serialize;

/// One refined cluster card = one seed's worth of AI input (planning §6.1, v2.1).
/// Serialized camelCase as the user-message JSON for the clustering call.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCardInput {
    /// The seed id (= the algorithm's first-pass grouping label, e.g. `"seed-1"`).
    pub cluster_id: String,
    /// Algorithmic cluster-kind guess from the seed's composition (AI may override).
    pub algorithmic_type_hint: ClusterKind,
    /// Path / route / annotation heuristics for the seed's entry points
    /// (controller/handler/route path names, `main`, …). Display + AI hint only.
    pub entrypoint_candidates: Vec<String>,
    /// The seed's changed symbols (card_id is the identity; name is display-only).
    pub changed_symbols: Vec<ChangedSymbolIn>,
    /// Relation-hint pairs **restricted to this seed's card ids** (strong/weak).
    pub relation_hints: RelationHints,
    /// Contract-change heuristics inside the seed (DTO fields, migration, config).
    pub contracts_changed: Vec<String>,
    /// Test symbols in the seed (test→impl), by name (display).
    pub related_tests: Vec<String>,
}

/// One changed symbol as the AI sees it. **card_id is the identity**; everything else is
/// context. No diff body — only the short Stage-1 `summary` (input-size defence).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedSymbolIn {
    /// Stable card id (the whitelist/flatten key — the AI must echo these).
    pub card_id: String,
    /// Bare symbol name (display only).
    pub name: String,
    /// Symbol classification (function/method/type/test/…).
    pub kind: SymbolKind,
    /// Added | Modified | Deleted.
    pub change_type: ChangeType,
    /// Short summary reused from the Stage-1 card (B1-safe; never the raw diff).
    pub summary: String,
}

/// Build the AI input cards from the Stage-② analysis + Stage-1 cards.
///
/// One `ClusterCardInput` per seed (v2.1). For each seed:
///  - gather its changed symbols (by card id) with kind/change_type/summary pulled from
///    the matching Stage-1 `ReviewCard` and the relation layer's `ChangedSymbol`;
///  - guess the cluster kind from the seed's composition (§4.3 heuristic);
///  - derive entry-point candidates, contract-change hints, and related tests from
///    path/name heuristics (never the diff body);
///  - restrict the relation hints to *this seed's* card-id pairs.
///
/// Deterministic: seeds and their members are already sorted; we never reorder. Cards or
/// changed-symbols missing from one side are skipped defensively (the relation layer and
/// Stage-1 are built from the same diff, so this is belt-and-braces only).
pub fn build_cluster_cards(
    seeds: &[Seed],
    hints: &RelationHints,
    changed: &[ChangedSymbol],
    cards: &[ReviewCard],
) -> Vec<ClusterCardInput> {
    // card_id -> Stage-1 card (kind/change_type/summary live here).
    let card_by_id: BTreeMap<&str, &ReviewCard> =
        cards.iter().map(|c| (c.id.as_str(), c)).collect();
    // card_id -> relation-layer changed symbol (name/owner/path/is_test/refs).
    let changed_by_id: BTreeMap<&str, &ChangedSymbol> =
        changed.iter().map(|c| (c.card_id.as_str(), c)).collect();

    seeds
        .iter()
        .map(|seed| build_one(seed, hints, &card_by_id, &changed_by_id))
        .collect()
}

fn build_one(
    seed: &Seed,
    hints: &RelationHints,
    card_by_id: &BTreeMap<&str, &ReviewCard>,
    changed_by_id: &BTreeMap<&str, &ChangedSymbol>,
) -> ClusterCardInput {
    let member_ids: BTreeSet<&str> = seed.card_ids.iter().map(String::as_str).collect();

    let mut changed_symbols: Vec<ChangedSymbolIn> = Vec::with_capacity(seed.card_ids.len());
    let mut entrypoints: BTreeSet<String> = BTreeSet::new();
    let mut contracts: BTreeSet<String> = BTreeSet::new();
    let mut tests: BTreeSet<String> = BTreeSet::new();

    for id in &seed.card_ids {
        let card = card_by_id.get(id.as_str());
        let cs = changed_by_id.get(id.as_str());

        // Prefer the relation layer's name (the bare symbol); fall back to the card's
        // symbol display, then the id itself (never panics on a missing side).
        let name = cs
            .map(|c| c.name.clone())
            .or_else(|| card.map(|c| c.symbol.clone()))
            .unwrap_or_else(|| id.clone());
        let kind = card.map(|c| c.kind).unwrap_or(SymbolKind::Function);
        let change_type = card.map(|c| c.change_type).unwrap_or(ChangeType::Modified);
        let summary = card.map(|c| c.summary.clone()).unwrap_or_default();
        let path = cs
            .map(|c| c.path.clone())
            .or_else(|| card.map(|c| c.path.clone()))
            .unwrap_or_default();

        // Entry-point heuristic: route/controller/handler path or a `main` symbol.
        if let Some(ep) = entrypoint_candidate(&path, &name) {
            entrypoints.insert(ep);
        }
        // Contract heuristic: DTO/request/response/migration/config/schema by kind+name.
        if let Some(c) = contract_candidate(kind, &name, &path) {
            contracts.insert(c);
        }
        // Related tests: a test symbol (by relation layer's `is_test` or kind).
        if cs.map(|c| c.is_test).unwrap_or(false) || kind == SymbolKind::Test {
            tests.insert(name.clone());
        }

        changed_symbols.push(ChangedSymbolIn {
            card_id: id.clone(),
            name,
            kind,
            change_type,
            summary,
        });
    }

    let relation_hints = restrict_hints(hints, &member_ids);
    let algorithmic_type_hint = guess_kind(&changed_symbols, &member_ids, &relation_hints, &entrypoints);

    ClusterCardInput {
        cluster_id: seed.id.clone(),
        algorithmic_type_hint,
        entrypoint_candidates: entrypoints.into_iter().collect(),
        changed_symbols,
        relation_hints,
        contracts_changed: contracts.into_iter().collect(),
        related_tests: tests.into_iter().collect(),
    }
}

/// Restrict the global relation hints to pairs whose *both* endpoints are in this seed.
/// (Cross-seed hints are not shown on a per-card basis; the AI sees per-seed evidence and
/// the seed list itself as the cross-seed signal.) Output stays sorted (deterministic).
fn restrict_hints(hints: &RelationHints, members: &BTreeSet<&str>) -> RelationHints {
    let keep = |pairs: &[(String, String)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .filter(|(a, b)| members.contains(a.as_str()) && members.contains(b.as_str()))
            .cloned()
            .collect()
    };
    RelationHints {
        strong: keep(&hints.strong),
        weak: keep(&hints.weak),
    }
}

/// Path/name entry-point heuristic (§6.1 `entrypointCandidates`). Pure name/path based —
/// no AST. Recognises common web-layer markers and a `main` entry symbol.
fn entrypoint_candidate(path: &str, name: &str) -> Option<String> {
    let p = path.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    let path_is_entry = p.contains("controller")
        || p.contains("handler")
        || p.contains("/routes/")
        || p.contains("/route/")
        || p.contains("/api/")
        || p.contains("/endpoints/")
        || p.contains("/router");
    let name_is_entry = n.ends_with("controller")
        || n.ends_with("handler")
        || n.ends_with("resource")
        || n == "main"
        || n.starts_with("handle");
    if path_is_entry || name_is_entry {
        // Display as "<name> (<path>)" so the AI sees both the symbol and where it lives.
        Some(format!("{name} ({path})"))
    } else {
        None
    }
}

/// Contract-change heuristic (§6.1 `contractsChanged`). A symbol is a contract change when
/// it is a DTO by *kind*, OR its *name* is contract-shaped (request/response/dto/payload/
/// command/event/schema), OR it lives on a *contract path* (migration/sql/config/proto).
/// Name/path/kind only — no diff body.
fn contract_candidate(kind: SymbolKind, name: &str, path: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    let p = path.to_ascii_lowercase();
    let is_dto_kind = matches!(kind, SymbolKind::Dto);
    let name_contract = n.ends_with("request")
        || n.ends_with("response")
        || n.ends_with("dto")
        || n.ends_with("payload")
        || n.ends_with("command")
        || n.ends_with("event")
        || n.ends_with("schema");
    let path_contract = p.contains("migration")
        || p.ends_with(".sql")
        || p.contains("/config")
        || p.ends_with("config.go")
        || p.ends_with("config.rs")
        || p.ends_with(".proto");
    if is_dto_kind || name_contract || path_contract {
        Some(name.to_string())
    } else {
        None
    }
}

/// Guess the cluster kind from a seed's composition (§4.3 — algorithm gives a *hint*, the
/// AI decides). Deliberately coarse, ordered by specificity:
///  - any contract symbol present and the seed is contract-heavy → `Contract`,
///  - a migration/config/infra path present → `Infra`,
///  - an entry-point present and a call/type chain (strong links) → `Flow`,
///  - a single new type/class with helpers and no entry point → `DomainConcept`,
///  - otherwise → `SharedFoundation` (touched by several, no single flow).
fn guess_kind(
    symbols: &[ChangedSymbolIn],
    _members: &BTreeSet<&str>,
    hints: &RelationHints,
    entrypoints: &BTreeSet<String>,
) -> ClusterKind {
    let n = symbols.len().max(1);
    let contract_n = symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Dto) || is_contract_name(&s.name))
        .count();
    let infra = symbols
        .iter()
        .any(|s| matches!(s.kind, SymbolKind::Config | SymbolKind::Migration));
    let added_type = symbols
        .iter()
        .any(|s| matches!(s.kind, SymbolKind::Type | SymbolKind::Class | SymbolKind::Interface)
            && s.change_type == ChangeType::Added);
    let has_flow = !entrypoints.is_empty() && !hints.strong.is_empty();

    if contract_n * 2 >= n && contract_n > 0 {
        ClusterKind::Contract
    } else if infra {
        ClusterKind::Infra
    } else if has_flow {
        ClusterKind::Flow
    } else if added_type {
        ClusterKind::DomainConcept
    } else {
        ClusterKind::SharedFoundation
    }
}

fn is_contract_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("request")
        || n.ends_with("response")
        || n.ends_with("dto")
        || n.ends_with("payload")
        || n.ends_with("command")
        || n.ends_with("event")
}

#[cfg(test)]
#[path = "clustercard_tests.rs"]
mod tests;
