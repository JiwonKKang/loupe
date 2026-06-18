//! Stage-③ cluster-card refinement tests.
//!
//! Pure functions only — we construct `Seed`/`RelationHints`/`ChangedSymbol`/`ReviewCard`
//! directly. The point of these tests is the *shape* contract (planning §6.1, v2.1):
//!  - one card per seed (seed is the input unit, not a single symbol),
//!  - identifiers are card ids; name rides along for display,
//!  - no raw diff — only the short Stage-1 summary,
//!  - relation hints restricted to the seed's own card-id pairs,
//!  - entry-point / contract / test heuristics fire on path/name only.

use super::*;
use crate::engine::model::{ChangeType, ClusterKind, ReviewCard, ReviewLine, SymbolKind, T_ADD};
use crate::engine::relations::{ChangedSymbol, RelationHints, Seed};
use crate::engine::symbols::SymbolRefs;

fn card(id: &str, symbol: &str, path: &str, kind: SymbolKind, ct: ChangeType) -> ReviewCard {
    ReviewCard {
        id: id.to_string(),
        chapter: "f".into(),
        symbol: symbol.to_string(),
        path: path.to_string(),
        status: "pending".into(),
        summary: format!("Updates {symbol}: +1 −0 line."),
        lines: vec![ReviewLine {
            n: 1,
            t: T_ADD,
            c: "x".into(),
        }],
        cluster_id: None,
        kind,
        qualified: symbol.to_string(),
        change_type: ct,
        ai_summary: None,
    }
}

fn changed(card_id: &str, name: &str, path: &str, is_test: bool) -> ChangedSymbol {
    ChangedSymbol {
        card_id: card_id.to_string(),
        name: name.to_string(),
        owner: None,
        path: path.to_string(),
        is_test,
        refs: SymbolRefs {
            sym_idx: 0,
            ..Default::default()
        },
        imports: Vec::new(),
    }
}

fn seed(id: &str, ids: &[&str]) -> Seed {
    Seed {
        id: id.to_string(),
        card_ids: ids.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn one_card_per_seed_unit_is_the_seed() {
    // Two seeds → exactly two cluster cards (v2.1: seed is the input unit).
    let seeds = vec![seed("seed-1", &["a", "b"]), seed("seed-2", &["c"])];
    let changed = vec![
        changed("a", "create", "svc.go", false),
        changed("b", "validate", "svc.go", false),
        changed("c", "Money", "money.go", false),
    ];
    let cards = vec![
        card("a", "create", "svc.go", SymbolKind::Function, ChangeType::Modified),
        card("b", "validate", "svc.go", SymbolKind::Function, ChangeType::Modified),
        card("c", "Money", "money.go", SymbolKind::Type, ChangeType::Added),
    ];
    let hints = RelationHints::default();
    let out = build_cluster_cards(&seeds, &hints, &changed, &cards);
    assert_eq!(out.len(), 2, "one card per seed");
    assert_eq!(out[0].cluster_id, "seed-1");
    assert_eq!(out[0].changed_symbols.len(), 2);
    assert_eq!(out[1].cluster_id, "seed-2");
    assert_eq!(out[1].changed_symbols.len(), 1);
}

#[test]
fn identifiers_are_card_ids_name_is_display_only() {
    let seeds = vec![seed("seed-1", &["card-x"])];
    let changed = vec![changed("card-x", "createOrder", "svc.go", false)];
    let cards = vec![card(
        "card-x",
        "createOrder",
        "svc.go",
        SymbolKind::Function,
        ChangeType::Modified,
    )];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    let s = &out[0].changed_symbols[0];
    assert_eq!(s.card_id, "card-x", "identity is the card id");
    assert_eq!(s.name, "createOrder", "name is carried for display");
}

#[test]
fn summary_is_reused_never_raw_diff() {
    // The card carries the Stage-1 statistical summary, NOT the diff line text ("x").
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "foo", "f.go", false)];
    let cards = vec![card("a", "foo", "f.go", SymbolKind::Function, ChangeType::Modified)];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    let s = &out[0].changed_symbols[0];
    assert_eq!(s.summary, "Updates foo: +1 −0 line.");
    // The raw diff line content ("x") must never appear anywhere in the card.
    let json = serde_json::to_string(&out[0]).unwrap();
    assert!(!json.contains("\"x\""), "raw diff must not enter the card: {json}");
}

#[test]
fn relation_hints_restricted_to_seed_member_pairs() {
    // Global hints include a cross-seed pair (a,c) and an intra-seed pair (a,b).
    // The seed-1 card must keep only (a,b).
    let seeds = vec![seed("seed-1", &["a", "b"]), seed("seed-2", &["c"])];
    let changed = vec![
        changed("a", "a", "f.go", false),
        changed("b", "b", "f.go", false),
        changed("c", "c", "g.go", false),
    ];
    let cards = vec![
        card("a", "a", "f.go", SymbolKind::Function, ChangeType::Modified),
        card("b", "b", "f.go", SymbolKind::Function, ChangeType::Modified),
        card("c", "c", "g.go", SymbolKind::Function, ChangeType::Modified),
    ];
    let hints = RelationHints {
        strong: vec![("a".into(), "b".into()), ("a".into(), "c".into())],
        weak: vec![],
    };
    let out = build_cluster_cards(&seeds, &hints, &changed, &cards);
    assert_eq!(out[0].relation_hints.strong, vec![("a".to_string(), "b".to_string())]);
    // seed-2 has a single member → no intra-seed pairs survive.
    assert!(out[1].relation_hints.strong.is_empty());
}

#[test]
fn entrypoint_candidate_from_controller_path() {
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "create", "order/OrderController.java", false)];
    let cards = vec![card(
        "a",
        "create",
        "order/OrderController.java",
        SymbolKind::Method,
        ChangeType::Modified,
    )];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    assert_eq!(out[0].entrypoint_candidates.len(), 1);
    assert!(out[0].entrypoint_candidates[0].contains("create"));
}

#[test]
fn contract_candidate_from_request_dto_name() {
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "CreateOrderRequest", "dto.go", false)];
    let cards = vec![card(
        "a",
        "CreateOrderRequest",
        "dto.go",
        SymbolKind::Type,
        ChangeType::Modified,
    )];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    assert_eq!(out[0].contracts_changed, vec!["CreateOrderRequest".to_string()]);
}

#[test]
fn related_tests_collected_from_is_test_flag() {
    let seeds = vec![seed("seed-1", &["t", "i"])];
    let changed = vec![
        changed("t", "TestCreate", "svc_test.go", true),
        changed("i", "create", "svc.go", false),
    ];
    let cards = vec![
        card("t", "TestCreate", "svc_test.go", SymbolKind::Function, ChangeType::Modified),
        card("i", "create", "svc.go", SymbolKind::Function, ChangeType::Modified),
    ];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    assert_eq!(out[0].related_tests, vec!["TestCreate".to_string()]);
}

#[test]
fn kind_hint_contract_when_contract_heavy() {
    let seeds = vec![seed("seed-1", &["a", "b"])];
    let changed = vec![
        changed("a", "CreateOrderRequest", "dto.go", false),
        changed("b", "OrderResponse", "dto.go", false),
    ];
    let cards = vec![
        card("a", "CreateOrderRequest", "dto.go", SymbolKind::Type, ChangeType::Modified),
        card("b", "OrderResponse", "dto.go", SymbolKind::Type, ChangeType::Modified),
    ];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    assert_eq!(out[0].algorithmic_type_hint, ClusterKind::Contract);
}

#[test]
fn kind_hint_flow_when_entrypoint_plus_strong_chain() {
    let seeds = vec![seed("seed-1", &["a", "b"])];
    let changed = vec![
        changed("a", "create", "OrderController.java", false),
        changed("b", "execute", "UseCase.java", false),
    ];
    let cards = vec![
        card("a", "create", "OrderController.java", SymbolKind::Method, ChangeType::Modified),
        card("b", "execute", "UseCase.java", SymbolKind::Method, ChangeType::Modified),
    ];
    let hints = RelationHints {
        strong: vec![("a".into(), "b".into())],
        weak: vec![],
    };
    let out = build_cluster_cards(&seeds, &hints, &changed, &cards);
    assert_eq!(out[0].algorithmic_type_hint, ClusterKind::Flow);
}

#[test]
fn deterministic_output_for_same_input() {
    let seeds = vec![seed("seed-1", &["a", "b"])];
    let changed = vec![
        changed("a", "foo", "f.go", false),
        changed("b", "bar", "f.go", false),
    ];
    let cards = vec![
        card("a", "foo", "f.go", SymbolKind::Function, ChangeType::Modified),
        card("b", "bar", "f.go", SymbolKind::Function, ChangeType::Modified),
    ];
    let h = RelationHints::default();
    let o1 = build_cluster_cards(&seeds, &h, &changed, &cards);
    let o2 = build_cluster_cards(&seeds, &h, &changed, &cards);
    assert_eq!(o1, o2);
}

#[test]
fn missing_card_side_degrades_without_panic() {
    // A seed references a card id that has no Stage-1 card / no changed symbol → it is
    // still emitted (id-only), nothing panics, nothing is dropped.
    let seeds = vec![seed("seed-1", &["ghost"])];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &[], &[]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].changed_symbols.len(), 1);
    assert_eq!(out[0].changed_symbols[0].card_id, "ghost");
    assert_eq!(out[0].changed_symbols[0].name, "ghost");
}
