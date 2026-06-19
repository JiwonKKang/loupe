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
use crate::engine::basesignals::{DeletedSymbol, FileBaseSignals, RenamePair, SignatureChange};
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

// ---------------------------------------------------------------------------
// base-AST signal distribution onto cluster cards (planning §2.1 base/head)
// ---------------------------------------------------------------------------

fn sig_with(
    deleted: Vec<DeletedSymbol>,
    renames: Vec<RenamePair>,
    signature_changes: Vec<SignatureChange>,
) -> FileBaseSignals {
    FileBaseSignals {
        deleted,
        renames,
        signature_changes,
    }
}

#[test]
fn deleted_symbol_attached_to_seed_by_file_path() {
    // `kept` (card a) survives in svc.go; `gone` was deleted from the same file → the
    // deleted-symbol signal attaches to the seed that owns a card from svc.go.
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "kept", "svc.go", false)];
    let cards = vec![card("a", "kept", "svc.go", SymbolKind::Function, ChangeType::Modified)];
    let signals = sig_with(
        vec![DeletedSymbol {
            id: "deleted::svc.go::gone".into(),
            name: "gone".into(),
            path: "svc.go".into(),
            signature: "func gone() int".into(),
        }],
        vec![],
        vec![],
    );
    let out =
        build_cluster_cards_with_signals(&seeds, &RelationHints::default(), &changed, &cards, &signals);
    assert_eq!(out[0].deleted_symbols.len(), 1, "{:?}", out[0].deleted_symbols);
    assert_eq!(out[0].deleted_symbols[0].name, "gone");
    // The synthetic deleted id must NOT be a clustering whitelist id (not in changedSymbols).
    assert!(
        out[0].changed_symbols.iter().all(|s| s.card_id != "deleted::svc.go::gone"),
        "deleted id must not become a member card id"
    );
}

#[test]
fn deleted_symbol_in_other_file_not_attached() {
    // The deleted symbol lives in other.go; the seed only touches svc.go → not attached.
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "kept", "svc.go", false)];
    let cards = vec![card("a", "kept", "svc.go", SymbolKind::Function, ChangeType::Modified)];
    let signals = sig_with(
        vec![DeletedSymbol {
            id: "deleted::other.go::gone".into(),
            name: "gone".into(),
            path: "other.go".into(),
            signature: "func gone()".into(),
        }],
        vec![],
        vec![],
    );
    let out =
        build_cluster_cards_with_signals(&seeds, &RelationHints::default(), &changed, &cards, &signals);
    assert!(out[0].deleted_symbols.is_empty());
}

#[test]
fn rename_pair_attached_and_mirrored_inline() {
    // `newName` (card b) is the rename of `oldName` → renamePairs on the card AND
    // renamedFrom on the changed symbol.
    let seeds = vec![seed("seed-1", &["b"])];
    let changed = vec![changed("b", "newName", "svc.go", false)];
    let cards = vec![card("b", "newName", "svc.go", SymbolKind::Function, ChangeType::Modified)];
    let signals = sig_with(
        vec![],
        vec![RenamePair {
            from_name: "oldName".into(),
            to_card_id: "b".into(),
            to_name: "newName".into(),
            path: "svc.go".into(),
            basis: "body",
        }],
        vec![],
    );
    let out =
        build_cluster_cards_with_signals(&seeds, &RelationHints::default(), &changed, &cards, &signals);
    assert_eq!(out[0].rename_pairs.len(), 1);
    assert_eq!(out[0].rename_pairs[0].from_name, "oldName");
    assert_eq!(out[0].rename_pairs[0].to_card_id, "b");
    // inline mirror
    let s = &out[0].changed_symbols[0];
    assert_eq!(s.renamed_from.as_deref(), Some("oldName"));
}

#[test]
fn signature_change_attached_and_mirrored_inline() {
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "create", "svc.go", false)];
    let cards = vec![card("a", "create", "svc.go", SymbolKind::Method, ChangeType::Modified)];
    let signals = sig_with(
        vec![],
        vec![],
        vec![SignatureChange {
            card_id: "a".into(),
            name: "create".into(),
            path: "svc.go".into(),
            old_signature: "func create(name string) int".into(),
            new_signature: "func create(name string, age int) int".into(),
        }],
    );
    let out =
        build_cluster_cards_with_signals(&seeds, &RelationHints::default(), &changed, &cards, &signals);
    assert_eq!(out[0].signature_changes.len(), 1);
    assert!(out[0].signature_changes[0].change.contains("→"));
    // inline mirror on the symbol
    let s = &out[0].changed_symbols[0];
    assert!(s.signature_change.as_deref().unwrap().contains("age int"));
}

#[test]
fn signal_for_nonmember_card_not_attached() {
    // A rename whose `to_card_id` is not in this seed must not attach (defensive).
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "kept", "svc.go", false)];
    let cards = vec![card("a", "kept", "svc.go", SymbolKind::Function, ChangeType::Modified)];
    let signals = sig_with(
        vec![],
        vec![RenamePair {
            from_name: "old".into(),
            to_card_id: "zzz".into(), // not a member of seed-1
            to_name: "new".into(),
            path: "svc.go".into(),
            basis: "body",
        }],
        vec![],
    );
    let out =
        build_cluster_cards_with_signals(&seeds, &RelationHints::default(), &changed, &cards, &signals);
    assert!(out[0].rename_pairs.is_empty());
}

#[test]
fn build_cluster_cards_without_signals_is_unchanged() {
    // The signal-free entry point yields cards with empty base-signal fields (back-compat).
    let seeds = vec![seed("seed-1", &["a"])];
    let changed = vec![changed("a", "foo", "f.go", false)];
    let cards = vec![card("a", "foo", "f.go", SymbolKind::Function, ChangeType::Modified)];
    let out = build_cluster_cards(&seeds, &RelationHints::default(), &changed, &cards);
    assert!(out[0].deleted_symbols.is_empty());
    assert!(out[0].rename_pairs.is_empty());
    assert!(out[0].signature_changes.is_empty());
    assert!(out[0].changed_symbols[0].renamed_from.is_none());
    assert!(out[0].changed_symbols[0].signature_change.is_none());
}
