//! ⑨ JIT definition injection unit tests (planning §5).
//!
//! Pure functions only — synthetic `FileDiff` + `ChangedSymbol`s, no git / no AI. Covers:
//!  - a referenced-but-unchanged struct is qualified, its overview extracted, and its card
//!    spliced **before** its first user in the flow order;
//!  - degradation: no eligible target ⇒ empty result, order untouched;
//!  - a referenced name that is itself a changed symbol is **not** injected (has a diff card).

use super::*;
use crate::engine::gitdiff::{FileDiff, FileStatus};
use crate::engine::symbols::{RefHit, SymbolRefs};

/// A changed symbol with explicit header type-refs (the signature-DTO signal).
fn changed_with_type_refs(
    card_id: &str,
    name: &str,
    path: &str,
    type_refs: &[&str],
) -> ChangedSymbol {
    ChangedSymbol {
        card_id: card_id.to_string(),
        name: name.to_string(),
        owner: None,
        path: path.to_string(),
        is_test: false,
        refs: SymbolRefs {
            sym_idx: 0,
            type_refs: type_refs
                .iter()
                .map(|i| RefHit {
                    ident: i.to_string(),
                    row: 0,
                    in_header: true,
                })
                .collect(),
            ..Default::default()
        },
        imports: Vec::new(),
    }
}

fn rust_file(path: &str, source: &str) -> FileDiff {
    FileDiff {
        new_path: path.to_string(),
        old_path: path.to_string(),
        new_source: source.to_string(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: Vec::new(),
    }
}

/// A changed symbol with explicit body `calls` (the construction / body-call signal, e.g. a
/// Java `new OrderDraft(...)` lands in `type_refs`/`calls` depending on the grammar).
fn changed_with_calls(card_id: &str, name: &str, path: &str, calls: &[&str]) -> ChangedSymbol {
    ChangedSymbol {
        card_id: card_id.to_string(),
        name: name.to_string(),
        owner: None,
        path: path.to_string(),
        is_test: false,
        refs: SymbolRefs {
            sym_idx: 0,
            calls: calls
                .iter()
                .map(|i| RefHit {
                    ident: i.to_string(),
                    row: 0,
                    in_header: false,
                })
                .collect(),
            ..Default::default()
        },
        imports: Vec::new(),
    }
}

fn java_file(path: &str, source: &str) -> FileDiff {
    FileDiff {
        new_path: path.to_string(),
        old_path: path.to_string(),
        new_source: source.to_string(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: Vec::new(),
    }
}

// A Rust source: a struct `OrderDraft` (NOT changed) plus a function `create_order`
// (changed) whose signature references `OrderDraft`.
const RUST_SRC: &str = "\
pub struct OrderDraft {
    pub id: u64,
    pub coupon_id: Option<String>,
}

pub fn create_order(draft: OrderDraft) -> u64 {
    draft.id
}
";

#[test]
fn references_unchanged_struct_injects_definition_before_first_user() {
    let file = rust_file("src/order.rs", RUST_SRC);
    // Only `create_order` changed; `OrderDraft` did not.
    let changed = vec![changed_with_type_refs(
        "src/order.rs::create_order",
        "create_order",
        "src/order.rs",
        &["OrderDraft"],
    )];
    let order = vec!["src/order.rs::create_order".to_string()];

    let inj = compute_jit_defs(&[file], &changed, &order);

    assert_eq!(inj.jit_defs.len(), 1, "OrderDraft qualifies for a JIT def");
    let jd = &inj.jit_defs[0];
    assert_eq!(jd.symbol, "OrderDraft");
    assert_eq!(jd.path, "src/order.rs");
    assert_eq!(jd.injected_before, "src/order.rs::create_order");
    assert_eq!(jd.id, "jit::src/order.rs::OrderDraft");

    // Overview extracted the struct fields (tree-sitter body slice).
    let f = &jd.overview.fields;
    assert!(f.iter().any(|x| x.starts_with("id:")), "fields = {f:?}");
    assert!(
        f.iter().any(|x| x.starts_with("coupon_id:")),
        "fields = {f:?}"
    );
    assert!(jd.overview.role.is_none(), "role is AI-only (None here)");

    // The synthetic card is a `Definition` card with the same id and no diff lines.
    assert_eq!(inj.cards.len(), 1);
    let card = &inj.cards[0];
    assert_eq!(card.id, jd.id);
    assert_eq!(card.kind, SymbolKind::Definition);
    assert_eq!(card.symbol, "OrderDraft");
    assert!(card.lines.is_empty());
    assert!(!card.summary.is_empty(), "B1: summary never empty");

    // Splice: the definition lands immediately before its first user.
    let spliced = splice_ordered(&order, &inj.jit_defs);
    assert_eq!(
        spliced,
        vec![
            "jit::src/order.rs::OrderDraft".to_string(),
            "src/order.rs::create_order".to_string(),
        ],
    );
}

#[test]
fn no_eligible_target_is_a_noop() {
    // `create_order` references nothing resolvable (an unknown type not in any changed file).
    let file = rust_file("src/order.rs", RUST_SRC);
    let changed = vec![changed_with_type_refs(
        "src/order.rs::create_order",
        "create_order",
        "src/order.rs",
        &["SomethingExternal"],
    )];
    let order = vec!["src/order.rs::create_order".to_string()];

    let inj = compute_jit_defs(&[file], &changed, &order);
    assert!(inj.jit_defs.is_empty(), "no resolvable definition ⇒ empty");
    assert!(inj.cards.is_empty());

    // Splice with an empty def list is the identity (byte-identical order).
    let spliced = splice_ordered(&order, &inj.jit_defs);
    assert_eq!(spliced, order, "order unchanged on degradation");
}

#[test]
fn referenced_name_that_is_itself_changed_is_not_injected() {
    // Both `OrderDraft` (the struct) and `create_order` are changed symbols of the file: the
    // struct already has its own diff card, so no JIT definition is injected for it.
    let file = rust_file("src/order.rs", RUST_SRC);
    let changed = vec![
        changed_with_type_refs(
            "src/order.rs::create_order",
            "create_order",
            "src/order.rs",
            &["OrderDraft"],
        ),
        // The struct itself changed in this PR (so it is a sibling diff card).
        ChangedSymbol {
            card_id: "src/order.rs::OrderDraft".to_string(),
            name: "OrderDraft".to_string(),
            owner: None,
            path: "src/order.rs".to_string(),
            is_test: false,
            refs: SymbolRefs::default(),
            imports: Vec::new(),
        },
    ];
    let order = vec![
        "src/order.rs::OrderDraft".to_string(),
        "src/order.rs::create_order".to_string(),
    ];

    let inj = compute_jit_defs(&[file], &changed, &order);
    assert!(
        inj.jit_defs.is_empty(),
        "a changed struct keeps its diff card; no overview is injected"
    );
}

#[test]
fn definition_overview_lists_changed_methods_owned_by_the_type() {
    // A struct `Session` (unchanged) + a changed method `validate` in `impl Session`. The
    // overview's `changed_methods` should surface `validate`.
    let src = "\
pub struct Session {
    pub user_id: u64,
}

impl Session {
    pub fn validate(&self) -> bool {
        self.user_id != 0
    }
}

pub fn login(s: Session) -> bool {
    s.validate()
}
";
    let file = rust_file("src/session.rs", src);
    let changed = vec![
        // `login` references `Session` in its signature (header type-ref).
        changed_with_type_refs("src/session.rs::login", "login", "src/session.rs", &["Session"]),
        // `validate` changed; its owner (impl block type) is `Session`.
        ChangedSymbol {
            card_id: "src/session.rs::validate".to_string(),
            name: "validate".to_string(),
            owner: Some("Session".to_string()),
            path: "src/session.rs".to_string(),
            is_test: false,
            refs: SymbolRefs::default(),
            imports: Vec::new(),
        },
    ];
    let order = vec![
        "src/session.rs::login".to_string(),
        "src/session.rs::validate".to_string(),
    ];

    let inj = compute_jit_defs(&[file], &changed, &order);
    assert_eq!(inj.jit_defs.len(), 1, "Session qualifies (referenced, unchanged)");
    let jd = &inj.jit_defs[0];
    assert_eq!(jd.symbol, "Session");
    assert!(
        jd.overview.changed_methods.contains(&"validate".to_string()),
        "changed_methods = {:?}",
        jd.overview.changed_methods
    );
}

// A Java class `OrderDraft` (unchanged) constructed by a changed method `createOrder`. The
// overview must split a constructor (name == type) from a public method, and read the field.
const JAVA_SRC: &str = "\
public class OrderDraft {
    private long id;

    public OrderDraft(long id) {
        this.id = id;
    }

    public long total() {
        return id;
    }
}
";

#[test]
fn java_overview_separates_constructor_from_public_methods() {
    let file = java_file("src/OrderDraft.java", JAVA_SRC);
    // `createOrder` (changed, in another conceptual file but same path here for the test)
    // constructs `OrderDraft` — a `reference.class` → lands in the body refs.
    let changed = vec![changed_with_calls(
        "src/OrderDraft.java::createOrder",
        "createOrder",
        "src/OrderDraft.java",
        &["OrderDraft"],
    )];
    let order = vec!["src/OrderDraft.java::createOrder".to_string()];

    let inj = compute_jit_defs(&[file], &changed, &order);
    assert_eq!(inj.jit_defs.len(), 1, "OrderDraft qualifies (constructed, unchanged)");
    let ov = &inj.jit_defs[0].overview;

    // The constructor (name == type) is NOT mislabelled as a public method (regression: the
    // old `head.contains(\" name(\")` check could never fire because `head` excludes `(`).
    assert!(
        ov.constructor.as_deref().is_some_and(|c| c.contains("OrderDraft(")),
        "constructor = {:?}",
        ov.constructor
    );
    assert!(
        !ov.public_methods.iter().any(|m| m.contains("OrderDraft(")),
        "the constructor must not also appear as a public method: {:?}",
        ov.public_methods
    );
    assert!(
        ov.public_methods.iter().any(|m| m.contains("total(")),
        "public method `total` should be listed: {:?}",
        ov.public_methods
    );
    // The private field was read (`id: long`).
    assert!(
        ov.fields.iter().any(|f| f.starts_with("id:")),
        "fields = {:?}",
        ov.fields
    );
}
