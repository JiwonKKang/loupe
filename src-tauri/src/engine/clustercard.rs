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

use super::basesignals::{FileBaseSignals, RenamePair, SignatureChange};
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
    /// **Base-AST signal** (planning §2.1): symbols deleted from a file that still exists
    /// but whose other (surviving) symbols are in this seed. Informational only — these
    /// carry synthetic ids and are **not** clustering whitelist ids; the AI is told to keep
    /// the related surviving change aware of the deletion (prompt §base-signals).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_symbols: Vec<DeletedSymbolIn>,
    /// **Base-AST signal**: renames `from → to` whose `to` is a member card of this seed.
    /// Tells the AI "this symbol is the renamed old one" so a rename is one change, not a
    /// scattered delete+add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rename_pairs: Vec<RenamePairIn>,
    /// **Base-AST signal**: signature (header) changes of member cards of this seed,
    /// rendered as `old → new` before→after context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature_changes: Vec<SignatureChangeIn>,
}

/// A deleted-symbol signal as the AI sees it. Synthetic id (never a clustering whitelist id).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedSymbolIn {
    /// Synthetic id `"deleted::<path>::<name>"` — for display/debug, NOT a whitelist id.
    pub id: String,
    /// The deleted symbol's bare name.
    pub name: String,
    /// The base signature (header) of the deleted symbol.
    pub signature: String,
}

/// A rename signal as the AI sees it: the old name → the surviving (renamed) card.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePairIn {
    /// The base (old) symbol name that disappeared.
    pub from_name: String,
    /// The head changed-symbol **card id** the old symbol became (a real member card id).
    pub to_card_id: String,
    /// The head symbol name (display).
    pub to_name: String,
}

/// A signature-change signal as the AI sees it: a member card's `old → new` header.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureChangeIn {
    /// The head changed-symbol **card id** (a real member card id).
    pub card_id: String,
    /// Bare symbol name.
    pub name: String,
    /// `"<old> → <new>"` rendered before→after signature.
    pub change: String,
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
    /// **Per-card AI summary evidence (Stage-⑥)**: a *compressed* diff excerpt of this card —
    /// its added/removed lines, capped at [`SNIPPET_MAX_LINES`] (token defence). Drawn from
    /// the Stage-1 `ReviewCard.lines` (add/del only). Carried so the labelling call can write a
    /// one-sentence per-card summary grounded in the actual change. Empty for cards with no
    /// add/del lines (defensive); skipped from serialization when empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snippet: String,
    /// **Base-AST signal**: when set, this symbol was *renamed* from this old name (its
    /// body/signature matched a symbol deleted from the base). Inline mirror of
    /// `ClusterCardInput::rename_pairs` so the AI sees it on the symbol itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
    /// **Base-AST signal**: when set, this symbol's signature changed `"<old> → <new>"`.
    /// Inline mirror of `ClusterCardInput::signature_changes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_change: Option<String>,
}

/// Max diff lines carried in a card's [`ChangedSymbolIn::snippet`] (token defence). The
/// labelling call needs *enough* change-context to write a one-sentence per-card summary, not
/// the whole hunk; we keep the first add/del lines up to this cap. Small and fixed so big
/// cards can't blow the prompt budget.
pub const SNIPPET_MAX_LINES: usize = 12;

/// Build a compressed diff snippet from a Stage-1 card's lines: keep only add/del lines
/// (context omitted — the *change* is what matters), prefix each with `+`/`-`, and cap at
/// [`SNIPPET_MAX_LINES`]. Deterministic (line order preserved). Empty when the card has no
/// add/del lines (e.g. a pure-context fallback card) so serialization skips it.
fn snippet_from_lines(lines: &[crate::engine::model::ReviewLine]) -> String {
    use crate::engine::model::{T_ADD, T_DEL};
    let mut out: Vec<String> = Vec::with_capacity(SNIPPET_MAX_LINES);
    for line in lines {
        let marker = match line.t {
            T_ADD => '+',
            T_DEL => '-',
            _ => continue, // skip context — the change is the add/del lines.
        };
        out.push(format!("{marker}{}", line.c));
        if out.len() >= SNIPPET_MAX_LINES {
            break;
        }
    }
    out.join("\n")
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
    build_cluster_cards_with_signals(seeds, hints, changed, cards, &FileBaseSignals::default())
}

/// Like [`build_cluster_cards`] but also distributes the **base-AST signals** (deleted
/// symbols / renames / signature changes, planning §2.1) onto the seeds they belong to:
///  - a rename / signature change is attached to the seed that owns its `to_card_id` /
///    `card_id` member (and mirrored inline onto that `ChangedSymbolIn`);
///  - a deleted symbol (no card id) is attached to every seed that has a member card from
///    the *same file path* (so it never vanishes — "all changes visible", §3.1).
///
/// Signals naming a card id that is in no seed (defensive) are dropped from the cards but
/// remain in the `RelationAnalysis` sidecar.
pub fn build_cluster_cards_with_signals(
    seeds: &[Seed],
    hints: &RelationHints,
    changed: &[ChangedSymbol],
    cards: &[ReviewCard],
    signals: &FileBaseSignals,
) -> Vec<ClusterCardInput> {
    // card_id -> Stage-1 card (kind/change_type/summary live here).
    let card_by_id: BTreeMap<&str, &ReviewCard> =
        cards.iter().map(|c| (c.id.as_str(), c)).collect();
    // card_id -> relation-layer changed symbol (name/owner/path/is_test/refs).
    let changed_by_id: BTreeMap<&str, &ChangedSymbol> =
        changed.iter().map(|c| (c.card_id.as_str(), c)).collect();

    seeds
        .iter()
        .map(|seed| build_one(seed, hints, &card_by_id, &changed_by_id, signals))
        .collect()
}

fn build_one(
    seed: &Seed,
    hints: &RelationHints,
    card_by_id: &BTreeMap<&str, &ReviewCard>,
    changed_by_id: &BTreeMap<&str, &ChangedSymbol>,
    signals: &FileBaseSignals,
) -> ClusterCardInput {
    let member_ids: BTreeSet<&str> = seed.card_ids.iter().map(String::as_str).collect();

    // Base-AST signals whose head card id is a member of this seed (rename/sig change).
    let rename_by_to: BTreeMap<&str, &RenamePair> = signals
        .renames
        .iter()
        .filter(|r| member_ids.contains(r.to_card_id.as_str()))
        .map(|r| (r.to_card_id.as_str(), r))
        .collect();
    let sigchange_by_card: BTreeMap<&str, &SignatureChange> = signals
        .signature_changes
        .iter()
        .filter(|s| member_ids.contains(s.card_id.as_str()))
        .map(|s| (s.card_id.as_str(), s))
        .collect();

    let mut changed_symbols: Vec<ChangedSymbolIn> = Vec::with_capacity(seed.card_ids.len());
    let mut entrypoints: BTreeSet<String> = BTreeSet::new();
    let mut contracts: BTreeSet<String> = BTreeSet::new();
    let mut tests: BTreeSet<String> = BTreeSet::new();
    // Paths present in this seed — used to attach deleted-symbol signals by file.
    let mut seed_paths: BTreeSet<String> = BTreeSet::new();

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
        // Per-card AI-summary evidence: a compressed add/del excerpt of this card (capped).
        let snippet = card.map(|c| snippet_from_lines(&c.lines)).unwrap_or_default();
        let path = cs
            .map(|c| c.path.clone())
            .or_else(|| card.map(|c| c.path.clone()))
            .unwrap_or_default();
        if !path.is_empty() {
            seed_paths.insert(path.clone());
        }

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

        // Inline base-AST annotations for this symbol (rename / signature change).
        let renamed_from = rename_by_to.get(id.as_str()).map(|r| r.from_name.clone());
        let signature_change = sigchange_by_card
            .get(id.as_str())
            .map(|s| format!("{} → {}", s.old_signature, s.new_signature));

        changed_symbols.push(ChangedSymbolIn {
            card_id: id.clone(),
            name,
            kind,
            change_type,
            summary,
            snippet,
            renamed_from,
            signature_change,
        });
    }

    // Deleted symbols of any file this seed touches (no card id → attach by path).
    let deleted_symbols: Vec<DeletedSymbolIn> = signals
        .deleted
        .iter()
        .filter(|d| seed_paths.contains(&d.path))
        .map(|d| DeletedSymbolIn {
            id: d.id.clone(),
            name: d.name.clone(),
            signature: d.signature.clone(),
        })
        .collect();

    // Top-level mirrors (rename pairs / signature changes) for this seed, sorted stable.
    let mut rename_pairs: Vec<RenamePairIn> = rename_by_to
        .values()
        .map(|r| RenamePairIn {
            from_name: r.from_name.clone(),
            to_card_id: r.to_card_id.clone(),
            to_name: r.to_name.clone(),
        })
        .collect();
    rename_pairs.sort_by(|a, b| a.to_card_id.cmp(&b.to_card_id));
    let mut signature_changes: Vec<SignatureChangeIn> = sigchange_by_card
        .values()
        .map(|s| SignatureChangeIn {
            card_id: s.card_id.clone(),
            name: s.name.clone(),
            change: format!("{} → {}", s.old_signature, s.new_signature),
        })
        .collect();
    signature_changes.sort_by(|a, b| a.card_id.cmp(&b.card_id));

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
        deleted_symbols,
        rename_pairs,
        signature_changes,
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

// ===========================================================================
// File-level seeds (planning §4.3 Infra/Config + §4.4 경로/주제 기반)
// ===========================================================================
//
// Symbol-less changes (CI/CD, Cargo.*, Caddyfile, *.yaml/*.toml, Dockerfile,
// .gitignore, docs, generated files …) become Stage-1 *file-level* cards
// (`SymbolKind::File`, id `<path>::__file`). They carry no symbol relations, so
// `analyze_relations` never sees them and they never enter a strong-relation seed.
// Left out of the AI input entirely they all fall to "Unclustered" — read by the
// reviewer as *classification failure* when they are really just infra/config.
//
// `build_file_seed_cards` turns each such card into its **own singleton seed-card**
// so it DOES enter the clustering whitelist. The AI is told (prompts.rs) to group
// these by tool/purpose (CI, dependencies, caddy, …); truly unrelated ones still
// fall to Unclustered, but the common infra-PR case now forms real Infra clusters.

/// Build one synthetic singleton seed-card per **file-level** Stage-1 card that is
/// NOT already a member of a symbol seed. Each card carries a single `ChangedSymbolIn`
/// whose `card_id` is the file card's id (so it joins the whitelist) and whose `name`
/// is the file path (the AI groups on path/topic). The kind is guessed from the path
/// (`SymbolKind::Config`/`Migration`/`File`) and the algorithmic hint is `Infra`/`Contract`.
///
/// Deterministic: the input `cards` are already in stable Stage-1 order; we keep it and
/// assign `file-seed-<n>` ids by appearance. `already_seeded` excludes ids that the
/// symbol seeds (Stage-②) already cover, so a card is never double-fed.
pub fn build_file_seed_cards(
    cards: &[ReviewCard],
    already_seeded: &BTreeSet<String>,
) -> Vec<ClusterCardInput> {
    let mut out: Vec<ClusterCardInput> = Vec::new();
    for card in cards {
        // Only symbol-less *file-level* cards are candidates (symbol cards are already
        // in the whitelist via their seed). Skip anything a symbol seed already covers.
        if card.kind != SymbolKind::File || already_seeded.contains(card.id.as_str()) {
            continue;
        }
        let kind = file_symbol_kind(&card.path);
        let hint = file_type_hint(kind, &card.path);
        let n = out.len() + 1;
        out.push(ClusterCardInput {
            cluster_id: format!("file-seed-{n}"),
            algorithmic_type_hint: hint,
            entrypoint_candidates: Vec::new(),
            changed_symbols: vec![ChangedSymbolIn {
                card_id: card.id.clone(),
                // The PATH is the grouping key the AI sees (topic/tool lives in the path).
                name: card.path.clone(),
                kind,
                change_type: card.change_type,
                // Stat summary stays out of the labelling input (see steps.rs); here we
                // keep the short Stage-1 summary so the clustering call has minimal context.
                summary: card.summary.clone(),
                // Per-card AI-summary evidence: compressed add/del excerpt of this file card.
                snippet: snippet_from_lines(&card.lines),
                renamed_from: None,
                signature_change: None,
            }],
            relation_hints: RelationHints::default(),
            contracts_changed: Vec::new(),
            related_tests: Vec::new(),
            deleted_symbols: Vec::new(),
            rename_pairs: Vec::new(),
            signature_changes: Vec::new(),
        });
    }
    out
}

/// Classify a symbol-less file by path into a [`SymbolKind`] for the file seed:
///  - DB migrations / SQL / sqlx metadata → `Migration` (contract-shaped),
///  - build / CI / config / infra files (toml, yaml, json, Dockerfile, Caddyfile,
///    .github/, .sh, .gitignore, lockfiles …) → `Config`,
///  - everything else (docs, generated text, assets) → `File`.
fn file_symbol_kind(path: &str) -> SymbolKind {
    let p = path.to_ascii_lowercase();
    let base = p.rsplit('/').next().unwrap_or(&p);
    if p.contains("migration") || p.ends_with(".sql") || p.contains("/.sqlx/") {
        return SymbolKind::Migration;
    }
    let is_config = base == "dockerfile"
        || base == "caddyfile"
        || base == ".gitignore"
        || base == ".dockerignore"
        || base == "makefile"
        || base.starts_with("cargo.")
        || p.starts_with(".github/")
        || p.contains("/.github/")
        || p.ends_with(".toml")
        || p.ends_with(".yaml")
        || p.ends_with(".yml")
        || p.ends_with(".json")
        || p.ends_with(".sh")
        || p.ends_with(".caddy")
        || p.ends_with(".env")
        || p.ends_with(".conf")
        || p.ends_with(".ini")
        || p.ends_with(".lock");
    if is_config {
        SymbolKind::Config
    } else {
        SymbolKind::File
    }
}

/// Algorithmic cluster-kind hint for a file seed: a migration/SQL file is a `Contract`
/// change; any other config/infra/doc file is `Infra`. The AI may override (it decides
/// the final topic grouping), but this seeds the kind so a single-file infra change is
/// not mis-hinted as `Flow`.
fn file_type_hint(kind: SymbolKind, _path: &str) -> ClusterKind {
    match kind {
        SymbolKind::Migration => ClusterKind::Contract,
        _ => ClusterKind::Infra,
    }
}

#[cfg(test)]
#[path = "clustercard_tests.rs"]
mod tests;
