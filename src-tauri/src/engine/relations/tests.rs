//! Unit tests for Stage-② relation signals + strong-seed clustering.
//!
//! Pure functions only — no git, no tree-sitter, no AI. We construct `ChangedSymbol`s
//! directly so each rule (strong/weak/hub/self/seed) is isolated.

use super::*;
use crate::engine::symbols::{RefHit, SymbolRefs};

/// Build a `ChangedSymbol` with explicit call references (by ident).
fn cs(card_id: &str, name: &str, calls: &[&str]) -> ChangedSymbol {
    ChangedSymbol {
        card_id: card_id.to_string(),
        name: name.to_string(),
        owner: None,
        path: format!("{card_id}.rs"),
        is_test: false,
        refs: SymbolRefs {
            sym_idx: 0,
            calls: calls
                .iter()
                .map(|i| RefHit {
                    ident: i.to_string(),
                    row: 1,
                    in_header: false,
                })
                .collect(),
            ..Default::default()
        },
        imports: Vec::new(),
    }
}

fn has_pair(pairs: &[(String, String)], a: &str, b: &str) -> bool {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    pairs
        .iter()
        .any(|(x, y)| x == lo && y == hi)
}

// ---------------------------------------------------------------------------
// strong rules
// ---------------------------------------------------------------------------

#[test]
fn direct_call_is_strong() {
    // create_order() calls apply() — both changed → strong pair.
    let changed = vec![
        cs("c1", "create_order", &["apply"]),
        cs("c2", "apply", &[]),
    ];
    let h = compute_relation_hints(&changed);
    assert!(has_pair(&h.strong, "c1", "c2"), "strong: {:?}", h.strong);
    // No spurious weak (different files, no import).
    assert!(!has_pair(&h.weak, "c1", "c2"));
}

#[test]
fn call_to_unchanged_symbol_yields_no_pair() {
    // create_order() calls helper() but helper isn't in the changed set → no relation.
    let changed = vec![cs("c1", "create_order", &["helper"])];
    let h = compute_relation_hints(&changed);
    assert!(h.strong.is_empty(), "strong should be empty: {:?}", h.strong);
}

#[test]
fn signature_type_in_header_is_strong() {
    // Go/Java/Rust-recovered: a header type_ref naming another changed symbol → strong.
    let mut a = cs("req", "CreateOrderRequest", &[]);
    let mut b = cs("cmd", "create", &[]);
    b.refs.type_refs.push(RefHit {
        ident: "CreateOrderRequest".into(),
        row: 0,
        in_header: true,
    });
    a.path = "x.go".into();
    b.path = "y.go".into();
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.strong, "req", "cmd"), "strong: {:?}", h.strong);
}

#[test]
fn type_ref_not_in_header_is_not_signature_strong() {
    // A type use in the BODY (in_header=false) is not the signature-type signal.
    let mut b = cs("cmd", "create", &[]);
    b.refs.type_refs.push(RefHit {
        ident: "CreateOrderRequest".into(),
        row: 9,
        in_header: false,
    });
    let a = cs("req", "CreateOrderRequest", &[]);
    let h = compute_relation_hints(&[a, b]);
    assert!(
        !has_pair(&h.strong, "req", "cmd"),
        "body type use must not be signature-strong: {:?}",
        h.strong
    );
}

#[test]
fn impl_reference_is_strong() {
    // Rust/Java: impl/trait reference naming another changed symbol → strong.
    let mut a = cs("imp", "OrderRepoImpl", &[]);
    a.refs.impls.push(RefHit {
        ident: "OrderRepo".into(),
        row: 0,
        in_header: true,
    });
    let b = cs("trait", "OrderRepo", &[]);
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.strong, "imp", "trait"), "strong: {:?}", h.strong);
}

#[test]
fn same_class_helpers_are_strong() {
    // Two changed methods sharing a non-empty owner → strong (public/private helper).
    let mut a = cs("m1", "validate", &[]);
    let mut b = cs("m2", "calculate", &[]);
    a.owner = Some("OrderService".into());
    b.owner = Some("OrderService".into());
    a.path = "svc.rs".into();
    b.path = "svc.rs".into();
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.strong, "m1", "m2"), "strong: {:?}", h.strong);
}

#[test]
fn method_to_its_enclosing_changed_type_is_strong() {
    // A changed method (owner = ApiError) and the changed type ApiError itself → strong
    // (Repository↔Entity family: the type and its behaviour changed together).
    let mut method = cs("m1", "into_response", &[]);
    method.owner = Some("ApiError".into());
    method.path = "error.rs".into();
    let mut ty = cs("t1", "ApiError", &[]);
    ty.owner = None;
    ty.path = "error.rs".into();
    let h = compute_relation_hints(&[method, ty]);
    assert!(has_pair(&h.strong, "m1", "t1"), "strong: {:?}", h.strong);
    // Strong subsumes the same-file weak.
    assert!(!has_pair(&h.weak, "m1", "t1"));
}

#[test]
fn method_owner_matching_another_method_is_not_type_rule() {
    // owner "Svc" matches another *method* (owner=Some) not a type → the method↔type
    // rule must NOT fire here (it requires the target's owner to be None / the type).
    let mut a = cs("m1", "helper", &[]);
    a.owner = Some("Svc".into());
    let mut b = cs("m2", "Svc", &[]); // a method literally named "Svc" but owned by Outer
    b.owner = Some("Outer".into());
    let h = compute_relation_hints(&[a, b]);
    assert!(
        !has_pair(&h.strong, "m1", "m2"),
        "method↔type rule must require target owner=None: {:?}",
        h.strong
    );
}

#[test]
fn different_owners_are_not_class_helper_strong() {
    let mut a = cs("m1", "validate", &[]);
    let mut b = cs("m2", "calculate", &[]);
    a.owner = Some("OrderService".into());
    b.owner = Some("PaymentService".into());
    let h = compute_relation_hints(&[a, b]);
    assert!(!has_pair(&h.strong, "m1", "m2"));
}

#[test]
fn test_to_impl_is_strong_via_call() {
    // A test symbol whose call references the impl → strong (test→implementation).
    let mut t = cs("t1", "test_create_order", &["create_order"]);
    t.is_test = true;
    let i = cs("c1", "create_order", &[]);
    let h = compute_relation_hints(&[t, i]);
    assert!(has_pair(&h.strong, "t1", "c1"), "strong: {:?}", h.strong);
}

// ---------------------------------------------------------------------------
// weak rules
// ---------------------------------------------------------------------------

#[test]
fn same_file_is_weak() {
    let mut a = cs("a", "foo", &[]);
    let mut b = cs("b", "bar", &[]);
    a.path = "same.rs".into();
    b.path = "same.rs".into();
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.weak, "a", "b"), "weak: {:?}", h.weak);
    assert!(!has_pair(&h.strong, "a", "b"));
}

#[test]
fn import_only_is_weak() {
    // a imports `bar` (a changed symbol) but never calls/type-uses it → weak.
    let mut a = cs("a", "foo", &[]);
    a.imports = vec!["bar".into()];
    a.path = "a.rs".into();
    let mut b = cs("b", "bar", &[]);
    b.path = "b.rs".into();
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.weak, "a", "b"), "weak: {:?}", h.weak);
    assert!(!has_pair(&h.strong, "a", "b"));
}

#[test]
fn strong_subsumes_weak_same_file() {
    // Same file (weak) AND a direct call (strong) → the pair is strong only, not weak.
    let mut a = cs("a", "foo", &["bar"]);
    let mut b = cs("b", "bar", &[]);
    a.path = "same.rs".into();
    b.path = "same.rs".into();
    let h = compute_relation_hints(&[a, b]);
    assert!(has_pair(&h.strong, "a", "b"), "strong: {:?}", h.strong);
    assert!(
        !has_pair(&h.weak, "a", "b"),
        "strong must subsume weak: {:?}",
        h.weak
    );
}

// ---------------------------------------------------------------------------
// hub exclusion + self exclusion
// ---------------------------------------------------------------------------

#[test]
fn hub_names_are_excluded_from_relations() {
    // Both symbols call Logger (a hub). Logger is also a changed symbol here, but the
    // hub rule must drop it so no flow is glued through it.
    let a = cs("a", "create_order", &["Logger"]);
    let b = cs("b", "cancel_order", &["Logger"]);
    let logger = cs("hub", "Logger", &[]);
    let h = compute_relation_hints(&[a, b, logger]);
    assert!(
        !has_pair(&h.strong, "a", "hub"),
        "hub must not be a relation target: {:?}",
        h.strong
    );
    assert!(!has_pair(&h.strong, "b", "hub"));
    // And the two flows are not glued to each other either (no shared non-hub link).
    assert!(!has_pair(&h.strong, "a", "b"));
}

#[test]
fn self_reference_never_pairs() {
    // A recursive function calling itself must not pair with itself.
    let r = cs("rec", "factorial", &["factorial"]);
    let h = compute_relation_hints(&[r]);
    assert!(h.strong.is_empty(), "no self pair: {:?}", h.strong);
    assert!(h.weak.is_empty());
}

#[test]
fn hub_owner_excluded_from_class_helper() {
    let mut a = cs("m1", "info", &[]);
    let mut b = cs("m2", "warn", &[]);
    a.owner = Some("Logger".into());
    b.owner = Some("Logger".into());
    let h = compute_relation_hints(&[a, b]);
    assert!(
        !has_pair(&h.strong, "m1", "m2"),
        "hub owner must not create class-helper strong: {:?}",
        h.strong
    );
}

// ---------------------------------------------------------------------------
// determinism
// ---------------------------------------------------------------------------

#[test]
fn output_is_order_independent() {
    let a = cs("c1", "create_order", &["apply"]);
    let b = cs("c2", "apply", &[]);
    let h1 = compute_relation_hints(&[a.clone(), b.clone()]);
    let h2 = compute_relation_hints(&[b, a]);
    assert_eq!(h1, h2, "relation hints must be order-independent");
}

// ---------------------------------------------------------------------------
// ②.5 strong-seed connected components
// ---------------------------------------------------------------------------

fn ids(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

#[test]
fn seeds_are_strong_connected_components() {
    // c1—c2 (strong), c3—c4 (strong), c5 isolated → 3 seeds.
    let hints = RelationHints {
        strong: vec![
            ("c1".into(), "c2".into()),
            ("c3".into(), "c4".into()),
        ],
        weak: vec![],
    };
    let seeds = seed_clusters(&ids(&["c1", "c2", "c3", "c4", "c5"]), &hints);
    assert_eq!(seeds.len(), 3, "seeds: {seeds:?}");
    // The seed containing c1 also contains c2.
    let s1 = seeds.iter().find(|s| s.card_ids.contains(&"c1".to_string())).unwrap();
    assert_eq!(s1.card_ids, ids(&["c1", "c2"]));
    // c5 is a singleton seed.
    assert!(seeds.iter().any(|s| s.card_ids == ids(&["c5"])));
}

#[test]
fn transitive_strong_merges_into_one_seed() {
    // c1—c2, c2—c3 → all three in ONE component.
    let hints = RelationHints {
        strong: vec![
            ("c1".into(), "c2".into()),
            ("c2".into(), "c3".into()),
        ],
        weak: vec![],
    };
    let seeds = seed_clusters(&ids(&["c1", "c2", "c3"]), &hints);
    assert_eq!(seeds.len(), 1, "transitive closure: {seeds:?}");
    assert_eq!(seeds[0].card_ids, ids(&["c1", "c2", "c3"]));
}

#[test]
fn weak_relations_do_not_seed() {
    // Only a weak relation between c1 and c2 → they stay separate singleton seeds.
    let hints = RelationHints {
        strong: vec![],
        weak: vec![("c1".into(), "c2".into())],
    };
    let seeds = seed_clusters(&ids(&["c1", "c2"]), &hints);
    assert_eq!(seeds.len(), 2, "weak must not seed: {seeds:?}");
}

#[test]
fn all_isolated_yields_all_singletons() {
    let hints = RelationHints::default();
    let seeds = seed_clusters(&ids(&["a", "b", "c"]), &hints);
    assert_eq!(seeds.len(), 3);
    assert!(seeds.iter().all(|s| s.card_ids.len() == 1));
}

#[test]
fn seeds_are_deterministic_and_sorted() {
    let hints = RelationHints {
        strong: vec![("b".into(), "z".into())],
        weak: vec![],
    };
    // Same input in different array order → identical seeds.
    let s1 = seed_clusters(&ids(&["z", "a", "b"]), &hints);
    let s2 = seed_clusters(&ids(&["a", "b", "z"]), &hints);
    assert_eq!(s1, s2);
    // Seed ids are seed-1, seed-2, … in component-min order; "a" comes first.
    assert_eq!(s1[0].card_ids, ids(&["a"]));
    assert_eq!(s1[0].id, "seed-1");
    assert_eq!(s1[1].card_ids, ids(&["b", "z"]));
    assert_eq!(s1[1].id, "seed-2");
}
