//! Engine unit + integration tests.
//!
//! Coverage:
//!  (a) symbol row boundaries — end_row is the closing-brace line of a multiline fn.
//!  (b) card split / file-level fallback / deleted file / new file.
//!  (c) every card has a non-empty summary (B1 invariant).
//!  (d) tempfile-based mini Go repo end-to-end through build_review (real git2).

use super::cards::build_file_cards;
use super::gitdiff::{DiffLine, FileDiff, FileStatus, LineKind};
use super::model::{ReviewCard, T_ADD, T_CTX, T_DEL};
use super::symbols::{self, Lang, Symbol};

// ---------------------------------------------------------------------------
// (a) symbol row boundaries
// ---------------------------------------------------------------------------

#[test]
fn go_symbol_end_row_is_closing_brace_line() {
    // Rows (0-base):
    // 0: package main
    // 1: (blank)
    // 2: func Add(a, b int) int {
    // 3:     return a + b
    // 4: }
    let src = "package main\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
    let syms = symbols::extract(Lang::Go, src).unwrap().unwrap();
    let add = syms
        .iter()
        .find(|s| s.name == "Add")
        .expect("Add should be extracted");
    assert_eq!(add.start_row, 2, "start row = func line");
    assert_eq!(add.end_row, 4, "end row = closing brace line");
}

#[test]
fn go_extracts_method_and_type() {
    let src = "\
package main

type Session struct {
\ttoken string
}

func (s *Session) Validate() error {
\tif s.token == \"\" {
\t\treturn nil
\t}
\treturn nil
}
";
    let syms = symbols::extract(Lang::Go, src).unwrap().unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Session"), "type Session, got {names:?}");
    assert!(names.contains(&"Validate"), "method Validate, got {names:?}");
    let validate = syms.iter().find(|s| s.name == "Validate").unwrap();
    // func line is row 6; closing brace is row 11.
    assert_eq!(validate.start_row, 6);
    assert_eq!(validate.end_row, 11);
}

#[test]
fn rust_extracts_function_boundary() {
    let src = "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n";
    let syms = symbols::extract(Lang::Rust, src).unwrap().unwrap();
    let main = syms.iter().find(|s| s.name == "main").unwrap();
    assert_eq!(main.start_row, 0);
    assert_eq!(main.end_row, 3);
}

#[test]
fn java_extracts_class_and_method() {
    let src = "\
class Foo {
    int bar() {
        return 1;
    }
}
";
    let syms = symbols::extract(Lang::Java, src).unwrap().unwrap();
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Foo"), "class Foo, got {names:?}");
    assert!(names.contains(&"bar"), "method bar, got {names:?}");
}

#[test]
fn parser_error_returns_none() {
    // Unbalanced braces => has_error() => Ok(None) => file-level fallback.
    let src = "func broken( {\n";
    assert!(symbols::extract(Lang::Go, src).unwrap().is_none());
}

#[test]
fn lang_from_path_dispatch() {
    assert_eq!(Lang::from_path("a/b.go"), Some(Lang::Go));
    assert_eq!(Lang::from_path("a/b.java"), Some(Lang::Java));
    assert_eq!(Lang::from_path("a/b.rs"), Some(Lang::Rust));
    assert_eq!(Lang::from_path("a/b.kt"), Some(Lang::Kotlin)); // Kotlin => symbol-level
    assert_eq!(Lang::from_path("a/b.txt"), None);
    assert_eq!(Lang::from_path("Makefile"), None);
}

// ---------------------------------------------------------------------------
// (a.2) extract_with_refs — per-language reference capture (B1 asymmetry)
// ---------------------------------------------------------------------------

/// Collect every call ident attributed to a given symbol name.
fn calls_of<'a>(
    syms: &[Symbol],
    refs: &'a [symbols::SymbolRefs],
    name: &str,
) -> Vec<&'a str> {
    let idx = syms.iter().position(|s| s.name == name).expect("symbol present");
    refs[idx].calls.iter().map(|h| h.ident.as_str()).collect()
}

#[test]
fn rust_extract_with_refs_collects_calls_and_impls() {
    // Rust tags.scm: reference.call + reference.implementation, NO reference.type.
    let src = "\
struct Repo;

trait Store {
    fn save(&self);
}

impl Store for Repo {
    fn save(&self) {
        helper();
    }
}

fn helper() {}

fn run() {
    let r = Repo;
    r.save();
}
";
    let (syms, refs) = symbols::extract_with_refs(Lang::Rust, src)
        .unwrap()
        .expect("parses");
    // `run` calls `save` (field-expression call) — present in calls.
    let run_calls = calls_of(&syms, &refs, "run");
    assert!(run_calls.contains(&"save"), "run calls: {run_calls:?}");
    // `save` (the method) calls `helper`.
    let save_calls = calls_of(&syms, &refs, "save");
    assert!(save_calls.contains(&"helper"), "save calls: {save_calls:?}");

    // impl reference: the `impl Store for Repo` block attributes a `Store` impl ref.
    // It is attributed to whichever symbol encloses the impl header row (the method
    // `save` is the innermost def starting after the impl line, so the impl ref lands
    // on no symbol unless the impl header row sits inside one). Assert the IMPL bucket
    // somewhere carries `Store` OR `Repo` — i.e. impls are collected for Rust at all.
    let any_impl: Vec<&str> = refs
        .iter()
        .flat_map(|r| r.impls.iter().map(|h| h.ident.as_str()))
        .collect();
    assert!(
        any_impl.contains(&"Store") || any_impl.contains(&"Repo"),
        "Rust impl refs should be collected (B1): {any_impl:?}"
    );
    // Rust type_refs come ONLY from the header walk (tags.scm has none). `run`'s body
    // `let r = Repo;` is NOT a header type, so run has no type_refs from the header.
    let run_idx = syms.iter().position(|s| s.name == "run").unwrap();
    assert!(
        refs[run_idx].type_refs.iter().all(|h| h.in_header),
        "Rust type_refs must all be header-derived"
    );
}

#[test]
fn rust_header_type_refs_recovered_for_signature() {
    // B1 Rust gap: signature types are recovered via the header walk, not tags.scm.
    let src = "\
struct Money;

fn price(items: Money) -> Money {
    let _ = items;
    Money
}
";
    let (syms, refs) = symbols::extract_with_refs(Lang::Rust, src)
        .unwrap()
        .expect("parses");
    let price_idx = syms.iter().position(|s| s.name == "price").unwrap();
    let header_types: Vec<&str> = refs[price_idx]
        .type_refs
        .iter()
        .filter(|h| h.in_header)
        .map(|h| h.ident.as_str())
        .collect();
    assert!(
        header_types.contains(&"Money"),
        "signature type `Money` recovered: {header_types:?}"
    );
}

#[test]
fn go_extract_with_refs_collects_calls_and_types_no_impls() {
    // Go tags.scm: reference.call + reference.type, NO reference.implementation.
    let src = "\
package main

type Session struct {
\ttoken string
}

func (s *Session) Validate() error {
\thelper()
\treturn nil
}

func helper() {}

func use() Session {
\tvar s Session
\treturn s
}
";
    let (syms, refs) = symbols::extract_with_refs(Lang::Go, src)
        .unwrap()
        .expect("parses");
    // Validate calls helper.
    let v_calls = calls_of(&syms, &refs, "Validate");
    assert!(v_calls.contains(&"helper"), "Validate calls: {v_calls:?}");
    // Go has type_refs (reference.type) — `use` references the `Session` type.
    let any_type: Vec<&str> = refs
        .iter()
        .flat_map(|r| r.type_refs.iter().map(|h| h.ident.as_str()))
        .collect();
    assert!(any_type.contains(&"Session"), "Go type refs: {any_type:?}");
    // Go has NO impls bucket — must always be empty (B1 asymmetry).
    assert!(
        refs.iter().all(|r| r.impls.is_empty()),
        "Go must have empty impls (grammar has no reference.implementation)"
    );
}

#[test]
fn java_extract_with_refs_collects_calls_and_class_no_type() {
    // Java tags.scm: reference.call + reference.class + reference.implementation,
    // NO reference.type. `reference.call` sits on argument_list — the ident is @name.
    let src = "\
class Foo {
    void run() {
        helper();
        Bar b = new Bar();
    }
    void helper() {}
}
";
    let (syms, refs) = symbols::extract_with_refs(Lang::Java, src)
        .unwrap()
        .expect("parses");
    // run() calls helper() — the call ident comes from @name, not the argument_list.
    let run_calls = calls_of(&syms, &refs, "run");
    assert!(run_calls.contains(&"helper"), "run calls: {run_calls:?}");
    // Java type_refs come from reference.class (object creation `new Bar()`).
    let any_type: Vec<&str> = refs
        .iter()
        .flat_map(|r| r.type_refs.iter().map(|h| h.ident.as_str()))
        .collect();
    assert!(
        any_type.contains(&"Bar"),
        "Java reference.class -> type_refs: {any_type:?}"
    );
}

#[test]
fn java_impl_reference_from_implements() {
    // Java `implements` produces reference.implementation via type_list.
    let src = "\
interface Store {}
class Repo implements Store {
    void save() {}
}
";
    let (_syms, refs) = symbols::extract_with_refs(Lang::Java, src)
        .unwrap()
        .expect("parses");
    let any_impl: Vec<&str> = refs
        .iter()
        .flat_map(|r| r.impls.iter().map(|h| h.ident.as_str()))
        .collect();
    // The `implements Store` ref is attributed to whatever symbol encloses its row.
    // It may be the class `Repo` (the type_list sits on the class header row). Assert
    // the impl bucket is populated for Java at all.
    assert!(
        any_impl.contains(&"Store") || refs.iter().any(|r| !r.impls.is_empty()),
        "Java impl refs should be collected: {any_impl:?}"
    );
}

#[test]
fn extract_with_refs_parser_error_returns_none() {
    // Safe degradation: parser ERROR => Ok(None) => no symbols, no relations.
    let src = "func broken( {\n";
    assert!(symbols::extract_with_refs(Lang::Go, src).unwrap().is_none());
}

#[test]
fn symbol_owners_rust_impl_method() {
    let src = "\
struct Repo;
impl Repo {
    fn save(&self) {}
}
";
    let syms = symbols::extract(Lang::Rust, src).unwrap().unwrap();
    let owners = symbols::symbol_owners(Lang::Rust, src, &syms);
    let save_idx = syms.iter().position(|s| s.name == "save").unwrap();
    assert_eq!(owners[save_idx].as_deref(), Some("Repo"));
}

#[test]
fn symbol_owners_go_receiver() {
    let src = "\
package main
type Session struct{}
func (s *Session) Validate() {}
";
    let syms = symbols::extract(Lang::Go, src).unwrap().unwrap();
    let owners = symbols::symbol_owners(Lang::Go, src, &syms);
    let v_idx = syms.iter().position(|s| s.name == "Validate").unwrap();
    assert_eq!(owners[v_idx].as_deref(), Some("Session"));
}

#[test]
fn symbol_owners_java_class_method() {
    let src = "\
class Foo {
    int bar() { return 1; }
}
";
    let syms = symbols::extract(Lang::Java, src).unwrap().unwrap();
    let owners = symbols::symbol_owners(Lang::Java, src, &syms);
    let bar_idx = syms.iter().position(|s| s.name == "bar").unwrap();
    assert_eq!(owners[bar_idx].as_deref(), Some("Foo"));
}

// ---------------------------------------------------------------------------
// helpers for synthetic FileDiff construction
// ---------------------------------------------------------------------------

fn line(kind: LineKind, new: Option<u32>, old: Option<u32>, c: &str) -> DiffLine {
    DiffLine {
        kind,
        new_lineno: new,
        old_lineno: old,
        content: c.to_string(),
    }
}

fn sym(name: &str, start: usize, end: usize) -> Symbol {
    Symbol {
        name: name.to_string(),
        qualified: name.to_string(),
        start_row: start,
        end_row: end,
    }
}

fn assert_all_summaries_nonempty(cards: &[ReviewCard]) {
    for c in cards {
        assert!(
            !c.summary.is_empty(),
            "card {} has empty summary (B1 violated)",
            c.id
        );
        let first = c.summary.chars().next().unwrap();
        assert!(
            first.is_uppercase(),
            "summary must start with a capital: {:?}",
            c.summary
        );
    }
}

/// Core invariant: every card carries at least one add or del line. A ctx-only card
/// (e.g. an unchanged symbol pulled in by a hunk's surrounding context_lines) is a
/// "+0 −0" empty card and must never be produced.
fn assert_no_empty_change_cards(cards: &[ReviewCard]) {
    for c in cards {
        // Binary file cards legitimately have no lines (summary only); skip those.
        if c.lines.is_empty() {
            continue;
        }
        let adds = c.lines.iter().filter(|l| l.t == T_ADD).count();
        let dels = c.lines.iter().filter(|l| l.t == T_DEL).count();
        assert!(
            adds + dels >= 1,
            "card {} is ctx-only (+{} −{}) — empty change card: {:?}",
            c.id,
            adds,
            dels,
            c.lines.iter().map(|l| (l.t, l.c.as_str())).collect::<Vec<_>>()
        );
        // And the summary must not advertise a zero change.
        assert!(
            !c.summary.contains("+0 −0"),
            "card {} summary advertises a zero change: {:?}",
            c.id,
            c.summary
        );
    }
}

// ---------------------------------------------------------------------------
// (b) + (c) card split / fallback / deleted / new file
// ---------------------------------------------------------------------------

#[test]
fn two_changed_symbols_split_into_per_symbol_cards() {
    // File with two functions, each with a change => two symbol cards.
    // new-coord rows (0-base): foo at 0..=2, bar at 4..=6.
    let symbols = vec![sym("foo", 0, 2), sym("bar", 4, 6)];
    let file = FileDiff {
        new_path: "pkg/x.go".into(),
        old_path: "pkg/x.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "func foo() {"),
            line(LineKind::Add, Some(2), None, "\treturn 1"),
            line(LineKind::Ctx, Some(3), Some(2), "}"),
            line(LineKind::Ctx, Some(5), Some(4), "func bar() {"),
            line(LineKind::Add, Some(6), None, "\treturn 2"),
            line(LineKind::Ctx, Some(7), Some(5), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 2, "two symbol cards");
    assert_eq!(out[0].symbol, "foo");
    assert_eq!(out[1].symbol, "bar");
    assert_eq!(out[0].id, "pkg/x.go::foo");
    assert_eq!(out[1].id, "pkg/x.go::bar");
    assert_eq!(out[0].chapter, "x.go");
    assert_all_summaries_nonempty(&out);
}

#[test]
fn orphan_change_outside_symbols_becomes_file_card() {
    // Two changed symbols + an import change outside any symbol => 2 + 1 file card.
    let symbols = vec![sym("foo", 2, 4), sym("bar", 6, 8)];
    let file = FileDiff {
        new_path: "pkg/y.go".into(),
        old_path: "pkg/y.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // orphan import change at row 0 (new line 1)
            line(LineKind::Add, Some(1), None, "import \"fmt\""),
            // foo change
            line(LineKind::Ctx, Some(3), Some(3), "func foo() {"),
            line(LineKind::Add, Some(4), None, "\tfmt.Println()"),
            line(LineKind::Ctx, Some(5), Some(4), "}"),
            // bar change
            line(LineKind::Ctx, Some(7), Some(6), "func bar() {"),
            line(LineKind::Add, Some(8), None, "\treturn"),
            line(LineKind::Ctx, Some(9), Some(7), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 3, "foo, bar, file-level orphan");
    let file_card = out.iter().find(|c| c.id == "pkg/y.go::__file").unwrap();
    assert!(file_card.lines.iter().any(|l| l.c == "import \"fmt\""));
    assert_all_summaries_nonempty(&out);
}

#[test]
fn single_changed_symbol_collapses_to_whole_file_card() {
    let symbols = vec![sym("foo", 0, 2), sym("bar", 4, 6)];
    let file = FileDiff {
        new_path: "pkg/z.go".into(),
        old_path: "pkg/z.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "func foo() {"),
            line(LineKind::Add, Some(2), None, "\treturn 1"),
            line(LineKind::Ctx, Some(3), Some(2), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 1, "0-1 changed symbols => one whole-file card");
    assert_eq!(out[0].id, "pkg/z.go::__file");
    assert_eq!(out[0].symbol, "z.go");
    assert_all_summaries_nonempty(&out);
}

#[test]
fn unsupported_language_is_file_level_fallback() {
    // Empty symbol slice (as the engine passes for .kt) => one whole-file card.
    let file = FileDiff {
        new_path: "app/Main.kt".into(),
        old_path: "app/Main.kt".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Del, None, Some(1), "val x = 1"),
            line(LineKind::Add, Some(1), None, "val x = 2"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].symbol, "Main.kt");
    assert_all_summaries_nonempty(&out);
}

#[test]
fn deleted_file_emits_del_only_card_from_old_path() {
    // M8: a deleted file is a del-only file-level card on the OLD path.
    let file = FileDiff {
        new_path: "old/gone.go".into(), // resolved fallback == old for a delete
        old_path: "old/gone.go".into(),
        new_source: String::new(),
        old_source: "package old\n\nfunc gone() {}\n".into(),
        status: FileStatus::Deleted,
        is_binary: false,
        lines: vec![
            line(LineKind::Del, None, Some(1), "package old"),
            line(LineKind::Del, None, Some(2), ""),
            line(LineKind::Del, None, Some(3), "func gone() {}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].path, "old/gone.go");
    assert!(out[0].lines.iter().all(|l| l.t == T_DEL), "del-only");
    assert!(out[0].summary.starts_with("Removes"));
    assert_all_summaries_nonempty(&out);
}

#[test]
fn new_file_is_single_whole_file_card_even_with_many_symbols() {
    // M13: a brand-new file bypasses the per-symbol split.
    let symbols = vec![sym("foo", 0, 2), sym("bar", 3, 5), sym("baz", 6, 8)];
    let file = FileDiff {
        new_path: "pkg/new.go".into(),
        old_path: "pkg/new.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Added,
        is_binary: false,
        lines: (1..=9)
            .map(|i| line(LineKind::Add, Some(i), None, &format!("line {i}")))
            .collect(),
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 1, "new file => single whole-file card (M13)");
    assert!(out[0].summary.starts_with("Adds"));
    assert_all_summaries_nonempty(&out);
}

#[test]
fn binary_file_is_summary_only_card() {
    // M12: binary => card with no lines, summary only.
    let file = FileDiff {
        new_path: "assets/logo.png".into(),
        old_path: "assets/logo.png".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: true,
        lines: vec![],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    assert_eq!(out.len(), 1);
    assert!(out[0].lines.is_empty(), "binary card has no lines");
    assert!(out[0].summary.contains("binary"));
    assert_all_summaries_nonempty(&out);
}

#[test]
fn del_line_gutter_is_monotonic_new_coordinate() {
    // B2: a del line takes the preceding ctx line's new gutter number, never old.
    let file = FileDiff {
        new_path: "a.go".into(),
        old_path: "a.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(51), Some(51), "ctx a"),
            line(LineKind::Ctx, Some(52), Some(52), "ctx b"),
            line(LineKind::Del, None, Some(53), "removed"),
            line(LineKind::Add, Some(53), None, "added"),
            line(LineKind::Ctx, Some(54), Some(54), "ctx c"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    let lines = &out[0].lines;
    let nums: Vec<u32> = lines.iter().map(|l| l.n).collect();
    // 51, 52, [del->52], 53, 54  — never goes backwards.
    assert_eq!(nums, vec![51, 52, 52, 53, 54]);
    let mut prev = 0u32;
    for n in &nums {
        assert!(*n >= prev, "gutter must be monotonic: {nums:?}");
        prev = *n;
    }
    // And it must NOT be the old_lineno (53).
    let del = lines.iter().find(|l| l.t == T_DEL).unwrap();
    assert_eq!(del.n, 52, "del uses preceding new gutter, not old_lineno");
}

#[test]
fn del_anchored_to_innermost_symbol_via_preceding_ctx() {
    // M9: a del run inside a function attributes to that function via preceding ctx.
    let symbols = vec![sym("a", 0, 3), sym("b", 5, 9)];
    let file = FileDiff {
        new_path: "m.go".into(),
        old_path: "m.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // change in a
            line(LineKind::Ctx, Some(1), Some(1), "func a() {"),
            line(LineKind::Del, None, Some(2), "old a body"),
            line(LineKind::Add, Some(2), None, "new a body"),
            line(LineKind::Ctx, Some(3), Some(3), "}"),
            // change in b
            line(LineKind::Ctx, Some(6), Some(5), "func b() {"),
            line(LineKind::Del, None, Some(7), "old b body"),
            line(LineKind::Add, Some(7), None, "new b body"),
            line(LineKind::Ctx, Some(8), Some(8), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|c| c.symbol == "a").unwrap();
    let b = out.iter().find(|c| c.symbol == "b").unwrap();
    assert!(a.lines.iter().any(|l| l.c == "old a body" && l.t == T_DEL));
    assert!(b.lines.iter().any(|l| l.c == "old b body" && l.t == T_DEL));
    // No cross-contamination.
    assert!(!a.lines.iter().any(|l| l.c == "old b body"));
    assert_no_empty_change_cards(&out);
    assert_all_summaries_nonempty(&out);
}

#[test]
fn ctx_only_neighbors_of_a_change_do_not_become_cards() {
    // Regression (dearday: src/lib.rs init_tracing/error became "+0 −0" cards).
    // git emits context_lines(3) around every hunk, so a change confined to ONE
    // function spills ctx lines into the adjacent functions. Those ctx-only neighbors
    // must NOT be treated as changed symbols => exactly one card for the real change.
    //
    // Layout (new-coord rows, 0-base):
    //   prev   : 0..=4   (fully unchanged neighbor above; only its tail is ctx)
    //   middle : 6..=10  (the ONLY function with a real add)
    //   next   : 12..=16 (fully unchanged neighbor below; only its head is ctx)
    let symbols = vec![sym("prev", 0, 4), sym("middle", 6, 10), sym("next", 12, 16)];
    let file = FileDiff {
        new_path: "src/lib.rs".into(),
        old_path: "src/lib.rs".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // trailing 3 ctx lines of `prev` (the hunk's leading context)
            line(LineKind::Ctx, Some(3), Some(3), "    // prev body"),
            line(LineKind::Ctx, Some(4), Some(4), "    prev_call();"),
            line(LineKind::Ctx, Some(5), Some(5), "}"),
            // `middle` — the real change
            line(LineKind::Ctx, Some(7), Some(7), "fn middle() {"),
            line(LineKind::Add, Some(8), None, "    middle_new();"),
            line(LineKind::Ctx, Some(9), Some(8), "    middle_body();"),
            line(LineKind::Ctx, Some(10), Some(9), "}"),
            // leading 3 ctx lines of `next` (the hunk's trailing context)
            line(LineKind::Ctx, Some(13), Some(12), "fn next() {"),
            line(LineKind::Ctx, Some(14), Some(13), "    next_body();"),
            line(LineKind::Ctx, Some(15), Some(14), "    next_call();"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);

    // Only one symbol actually changed => 0/1 changed-symbol path => one whole-file
    // card. The crucial assertion is that `prev`/`next` never produced ctx-only cards.
    assert!(
        !out.iter().any(|c| c.symbol == "prev" || c.symbol == "next"),
        "ctx-only neighbors must not become cards: {:?}",
        out.iter().map(|c| (c.symbol.as_str(), c.summary.as_str())).collect::<Vec<_>>()
    );
    assert_no_empty_change_cards(&out);
    assert_all_summaries_nonempty(&out);
}

#[test]
fn changed_symbol_card_with_ctx_neighbors_excludes_neighbors() {
    // Two functions genuinely change, and a third (ctx-only neighbor) sits between
    // their hunks' context. We must get exactly 2 per-symbol cards (foo, bar) and the
    // ctx-only neighbor must be excluded — proving ctx alone never grants a card even
    // on the >=2-changed-symbols path. ctx is still preserved INSIDE the real cards.
    let symbols = vec![sym("foo", 0, 2), sym("mid", 4, 6), sym("bar", 8, 10)];
    let file = FileDiff {
        new_path: "pkg/n.go".into(),
        old_path: "pkg/n.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // foo: real change + its own ctx
            line(LineKind::Ctx, Some(1), Some(1), "func foo() {"),
            line(LineKind::Add, Some(2), None, "\tfoo_new()"),
            line(LineKind::Ctx, Some(3), Some(2), "}"),
            // mid: ctx-only (it is between the two hunks' context)
            line(LineKind::Ctx, Some(5), Some(4), "func mid() {"),
            line(LineKind::Ctx, Some(6), Some(5), "}"),
            // bar: real change + its own ctx
            line(LineKind::Ctx, Some(9), Some(8), "func bar() {"),
            line(LineKind::Add, Some(10), None, "\tbar_new()"),
            line(LineKind::Ctx, Some(11), Some(9), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);

    let names: Vec<&str> = out.iter().map(|c| c.symbol.as_str()).collect();
    assert_eq!(out.len(), 2, "exactly foo + bar, got {names:?}");
    assert!(names.contains(&"foo") && names.contains(&"bar"), "got {names:?}");
    assert!(!names.contains(&"mid"), "ctx-only neighbor must be excluded: {names:?}");
    // ctx is still preserved as display context inside the real card.
    let foo = out.iter().find(|c| c.symbol == "foo").unwrap();
    assert!(foo.lines.iter().any(|l| l.c == "func foo() {" && l.t == T_CTX));
    assert!(foo.lines.iter().any(|l| l.c == "\tfoo_new()" && l.t == T_ADD));
    assert_no_empty_change_cards(&out);
    assert_all_summaries_nonempty(&out);
}

#[test]
fn ctx_only_run_within_a_changed_symbol_is_dropped() {
    // A wide symbol changes in ONE place but git's context for an UNRELATED later hunk
    // (in a second symbol) lands a stray ctx run on the wide symbol via a coord gap.
    // The wide symbol is genuinely changed (one add), so it makes a card — but the
    // ctx-only run must be dropped, never emitted as a separate "+0 −0" run card.
    let symbols = vec![sym("wide", 0, 50), sym("other", 60, 62)];
    let file = FileDiff {
        new_path: "pkg/w.go".into(),
        old_path: "pkg/w.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // real change in `wide` near the top
            line(LineKind::Ctx, Some(10), Some(10), "ctx top"),
            line(LineKind::Add, Some(11), None, "added top"),
            line(LineKind::Ctx, Some(12), Some(11), "ctx top2"),
            // a stray ctx-only run still inside `wide` far below (coord jump => new run)
            line(LineKind::Ctx, Some(45), Some(44), "ctx far"),
            line(LineKind::Ctx, Some(46), Some(45), "ctx far2"),
            // real change in `other` so the >=2-symbol split path runs
            line(LineKind::Ctx, Some(61), Some(58), "func other() {"),
            line(LineKind::Add, Some(62), None, "added other"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);

    let wide_cards: Vec<_> = out.iter().filter(|c| c.symbol == "wide").collect();
    assert_eq!(
        wide_cards.len(),
        1,
        "the ctx-only run must be dropped => one wide card, got {:?}",
        out.iter().map(|c| (c.id.as_str(), c.summary.as_str())).collect::<Vec<_>>()
    );
    // The surviving wide card carries the real add (and its local ctx).
    assert!(wide_cards[0].lines.iter().any(|l| l.c == "added top" && l.t == T_ADD));
    assert!(!wide_cards[0].lines.iter().any(|l| l.c == "ctx far"));
    assert_no_empty_change_cards(&out);
    assert_all_summaries_nonempty(&out);
}

#[test]
fn disjoint_hunks_in_one_symbol_split_into_runs_without_gutter_jump() {
    // M11: a wide function changed in two places far apart — git emits two hunks
    // with the unchanged middle omitted (new-coord jumps 12 -> 40). The symbol must
    // split into one card per contiguous run so no card's gutter jumps backwards or
    // forwards (B2). A second symbol guarantees the per-symbol split path is taken.
    let symbols = vec![sym("wide", 0, 50), sym("other", 60, 62)];
    let file = FileDiff {
        new_path: "pkg/w.go".into(),
        old_path: "pkg/w.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            // hunk 1 of `wide` near the top (new lines 10..=12)
            line(LineKind::Ctx, Some(10), Some(10), "ctx top"),
            line(LineKind::Add, Some(11), None, "added top"),
            line(LineKind::Ctx, Some(12), Some(11), "ctx top2"),
            // hunk 2 of `wide` far below (new lines 40..=42) — git omitted 13..39
            line(LineKind::Ctx, Some(40), Some(38), "ctx bot"),
            line(LineKind::Add, Some(41), None, "added bot"),
            line(LineKind::Ctx, Some(42), Some(39), "ctx bot2"),
            // a change in `other` so the >=2-symbol split path runs
            line(LineKind::Ctx, Some(61), Some(58), "func other() {"),
            line(LineKind::Add, Some(62), None, "added other"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);

    let wide_cards: Vec<_> = out.iter().filter(|c| c.symbol == "wide").collect();
    assert_eq!(wide_cards.len(), 2, "two disjoint hunks => two cards, got {:?}",
        out.iter().map(|c| c.id.as_str()).collect::<Vec<_>>());
    // Run ids are stable + unique.
    assert_ne!(wide_cards[0].id, wide_cards[1].id);

    // No card's gutter ever jumps by more than 1 between consecutive non-del lines.
    for c in &out {
        let mut prev: Option<u32> = None;
        for l in &c.lines {
            if l.t == T_DEL {
                continue;
            }
            if let Some(p) = prev {
                assert!(
                    l.n <= p + 1,
                    "gutter jump in card {}: {} -> {} ({:?})",
                    c.id, p, l.n,
                    c.lines.iter().map(|x| x.n).collect::<Vec<_>>()
                );
            }
            prev = Some(l.n);
        }
    }
    assert_all_summaries_nonempty(&out);
}

#[test]
fn duplicate_symbol_names_get_stable_suffix() {
    // M3: two same-named symbols => stable @-suffix based on start_row position.
    let symbols = vec![sym("handler", 0, 2), sym("handler", 4, 6)];
    let file = FileDiff {
        new_path: "dup.rs".into(),
        old_path: "dup.rs".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "fn handler() {"),
            line(LineKind::Add, Some(2), None, "  a"),
            line(LineKind::Ctx, Some(3), Some(2), "}"),
            line(LineKind::Ctx, Some(5), Some(4), "fn handler() {"),
            line(LineKind::Add, Some(6), None, "  b"),
            line(LineKind::Ctx, Some(7), Some(5), "}"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    assert_eq!(out.len(), 2);
    let ids: Vec<&str> = out.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"dup.rs::handler@0"), "got {ids:?}");
    assert!(ids.contains(&"dup.rs::handler@1"), "got {ids:?}");
    // ids must be unique
    assert_ne!(out[0].id, out[1].id);
}

#[test]
fn lines_carry_no_trailing_newline_and_three_kinds_only() {
    let file = FileDiff {
        new_path: "k.go".into(),
        old_path: "k.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "ctx"),
            line(LineKind::Add, Some(2), None, "add"),
            line(LineKind::Del, None, Some(2), "del"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    for l in &out[0].lines {
        assert!(!l.c.ends_with('\n'));
        assert!([T_ADD, T_DEL, T_CTX].contains(&l.t));
    }
}

#[test]
fn serialized_card_has_exact_contract_keys() {
    // The front-end reads { id, chapter, symbol, path, status, summary, lines }
    // and each line as { n, t, c }. Lock those exact keys.
    let file = FileDiff {
        new_path: "s.go".into(),
        old_path: "s.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "ctx"),
            line(LineKind::Add, Some(2), None, "add"),
        ],
    };
    let mut out = Vec::new();
    build_file_cards(&file, &[], &mut out);
    let v = serde_json::to_value(&out[0]).unwrap();
    let obj = v.as_object().unwrap();
    for key in ["id", "chapter", "symbol", "path", "status", "summary", "lines"] {
        assert!(obj.contains_key(key), "card missing key {key}");
    }
    let line0 = &v["lines"][0];
    let lobj = line0.as_object().unwrap();
    assert_eq!(lobj.len(), 3, "line has exactly n/t/c");
    for key in ["n", "t", "c"] {
        assert!(lobj.contains_key(key), "line missing key {key}");
    }
    assert_eq!(v["status"], "pending");
}

#[test]
fn stage2_card_fields_serialize_as_camelcase_without_disturbing_stage1_keys() {
    // m1: the new ReviewCard fields must come out camelCase (clusterId / changeType /
    // aiSummary / qualified / kind) while the Stage-1 keys (chapter, summary, …) and
    // ReviewLine's n/t/c are untouched. The defaults are present and serializable.
    let file = FileDiff {
        new_path: "pkg/x.go".into(),
        old_path: "pkg/x.go".into(),
        new_source: String::new(),
        old_source: String::new(),
        status: FileStatus::Modified,
        is_binary: false,
        lines: vec![
            line(LineKind::Ctx, Some(1), Some(1), "func foo() {"),
            line(LineKind::Add, Some(2), None, "\treturn 1"),
            line(LineKind::Ctx, Some(3), Some(2), "}"),
            line(LineKind::Ctx, Some(5), Some(4), "func bar() {"),
            line(LineKind::Add, Some(6), None, "\treturn 2"),
            line(LineKind::Ctx, Some(7), Some(5), "}"),
        ],
    };
    let symbols = vec![sym("foo", 0, 2), sym("bar", 4, 6)];
    let mut out = Vec::new();
    build_file_cards(&file, &symbols, &mut out);
    let v = serde_json::to_value(&out[0]).unwrap();
    let obj = v.as_object().unwrap();
    // New fields, camelCased.
    for key in ["clusterId", "kind", "qualified", "changeType", "aiSummary"] {
        assert!(obj.contains_key(key), "card missing camelCase key {key}");
    }
    // chapter is preserved (App.jsx / ProgressSpine depend on it).
    assert_eq!(v["chapter"], "x.go");
    // Defaults: no cluster yet, function kind, qualified mirrors name. The "foo" card
    // is pure additions, so change_type derives to Added (camelCased to "added").
    assert_eq!(v["clusterId"], serde_json::Value::Null);
    assert_eq!(v["kind"], "function");
    assert_eq!(v["changeType"], "added");
    assert_eq!(v["qualified"], "foo");
    assert_eq!(v["aiSummary"], serde_json::Value::Null);
}

#[test]
fn review_data_serializes_stage2_fields_as_camelcase() {
    // m1: ReviewData's new fields are camelCase (clusterOrder / orderedCardIds /
    // headSha / baseSha / mergeSuggestions / splitSuggestions) and `cards`
    // is unchanged. A Stage-1-only build_review fills everything else with defaults.
    let data = super::model::ReviewData::default();
    let v = serde_json::to_value(&data).unwrap();
    let obj = v.as_object().unwrap();
    for key in [
        "cards",
        "clusters",
        "clusterOrder",
        "orderedCardIds",
        "unclustered",
        "headSha",
        "baseSha",
        "analysis",
        "mergeSuggestions",
        "splitSuggestions",
    ] {
        assert!(obj.contains_key(key), "ReviewData missing key {key}");
    }
    // Default analysis state is "idle" (Stage-1 only, no AI overlay yet).
    assert_eq!(v["analysis"], "idle");
    assert!(v["cards"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// (d) end-to-end through real git2 on a tempfile mini Go repo
// ---------------------------------------------------------------------------

#[test]
fn e2e_mini_go_repo_two_commits() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    // --- commit 1 on main: a Go file with two functions ---
    let path = dir.path().join("main.go");
    fs::write(
        &path,
        "package main\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n\nfunc Sub(a, b int) int {\n\treturn a - b\n}\n",
    )
    .unwrap();

    let base_oid = {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("main.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
    };

    // --- commit 2 on a branch: change BOTH functions => expect 2 symbol cards ---
    let base_commit = repo.find_commit(base_oid).unwrap();
    // Pin an explicit "main" branch (the default branch name is config-dependent).
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();

    fs::write(
        &path,
        "package main\n\nfunc Add(a, b int) int {\n\treturn a + b + 0\n}\n\nfunc Sub(a, b int) int {\n\treturn a - b - 0\n}\n",
    )
    .unwrap();

    {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("main.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "change both", &tree, &[&parent])
            .unwrap();
    }

    let repo_path = dir.path().to_str().unwrap();
    let data = super::build_review(repo_path, "main", "target").unwrap();

    // Two functions changed => two symbol cards.
    assert_eq!(data.cards.len(), 2, "two changed symbols => two cards");
    let names: Vec<&str> = data.cards.iter().map(|c| c.symbol.as_str()).collect();
    assert!(names.contains(&"Add"), "got {names:?}");
    assert!(names.contains(&"Sub"), "got {names:?}");

    // Each card has the actual added line and a non-empty summary.
    let add_card = data.cards.iter().find(|c| c.symbol == "Add").unwrap();
    assert!(add_card.lines.iter().any(|l| l.c.contains("a + b + 0") && l.t == T_ADD));
    assert_eq!(add_card.path, "main.go");
    assert_no_empty_change_cards(&data.cards);
    assert_all_summaries_nonempty(&data.cards);

    // Every line uses one of the three kinds, no trailing newlines.
    for card in &data.cards {
        for l in &card.lines {
            assert!([T_ADD, T_DEL, T_CTX].contains(&l.t));
            assert!(!l.c.ends_with('\n'));
        }
    }
}

#[test]
fn e2e_new_file_is_single_card() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    // commit 1: one file
    fs::write(dir.path().join("a.go"), "package main\n").unwrap();
    let base_oid = {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
    };

    // commit 2 on branch: add a brand-new file with several functions
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    fs::write(
        dir.path().join("b.go"),
        "package main\n\nfunc A() {}\n\nfunc B() {}\n\nfunc C() {}\n",
    )
    .unwrap();
    {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("b.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add b", &tree, &[&parent])
            .unwrap();
    }

    let data = super::build_review(dir.path().to_str().unwrap(), "main", "target").unwrap();
    // New file => single whole-file card (M13).
    let b_cards: Vec<_> = data.cards.iter().filter(|c| c.path == "b.go").collect();
    assert_eq!(b_cards.len(), 1, "new file => one card, got {b_cards:?}");
    assert!(b_cards[0].summary.starts_with("Adds"));
    assert_all_summaries_nonempty(&data.cards);
}

// ---------------------------------------------------------------------------
// (e) list_branches on a real tempfile mini repo
// ---------------------------------------------------------------------------

#[test]
fn list_branches_finds_branches_current_and_default() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    // One commit, then pin "main" and add a "feature" branch off it.
    fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let base_oid = {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap()
    };
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("feature", &base_commit, false).unwrap();
    // Park HEAD on "feature" so `current` is well-defined and not the default.
    repo.set_head("refs/heads/feature").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();

    let out = super::list_branches(dir.path().to_str().unwrap()).unwrap();

    // Both local branches present (init may also leave a default branch behind;
    // assert membership rather than exact length to stay config-independent).
    assert!(out.branches.iter().any(|b| b == "main"), "got {:?}", out.branches);
    assert!(out.branches.iter().any(|b| b == "feature"), "got {:?}", out.branches);
    // HEAD is on feature.
    assert_eq!(out.current.as_deref(), Some("feature"));
    // main exists => default base is main.
    assert_eq!(out.default.as_deref(), Some("main"));
    // current sorts first.
    assert_eq!(out.branches.first().map(String::as_str), Some("feature"));
}

#[test]
fn list_branches_errors_on_non_repo() {
    let dir = tempfile::tempdir().unwrap();
    // An empty temp dir is not a git repo => Err (surfaced as String to the UI).
    assert!(super::list_branches(dir.path().to_str().unwrap()).is_err());
}

// ---------------------------------------------------------------------------
// (f) analyze_relations end-to-end on a synthetic Rust repo (real git2)
// ---------------------------------------------------------------------------

#[test]
fn e2e_analyze_relations_rust_call_chain() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    // base: two free functions, one calls the other.
    let path = dir.path().join("svc.rs");
    fs::write(
        &path,
        "fn validate() -> bool {\n    true\n}\n\nfn create() -> bool {\n    validate()\n}\n",
    )
    .unwrap();
    let base_oid = {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("svc.rs")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap()
    };

    // target: change BOTH functions so both are changed symbols; create() still calls
    // validate() => a direct-call STRONG relation between the two changed cards.
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    fs::write(
        &path,
        "fn validate() -> bool {\n    false\n}\n\nfn create() -> bool {\n    validate() && true\n}\n",
    )
    .unwrap();
    {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("svc.rs")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "change both", &tree, &[&parent]).unwrap();
    }

    let a = super::analyze_relations(dir.path().to_str().unwrap(), "main", "target").unwrap();
    // Two changed symbols.
    assert_eq!(a.changed.len(), 2, "changed: {:?}", a.changed);
    // create() calls validate() (both changed) => exactly one strong pair.
    assert_eq!(a.hints.strong.len(), 1, "strong: {:?}", a.hints.strong);
    // The strong pair seeds them into ONE component.
    assert_eq!(a.seeds.len(), 1, "seeds: {:?}", a.seeds);
    assert_eq!(a.seeds[0].card_ids.len(), 2);
    // Every changed card is covered by a seed (nothing dropped).
    let covered: usize = a.seeds.iter().map(|s| s.card_ids.len()).sum();
    assert_eq!(covered, a.changed.len());
}

#[test]
fn e2e_analyze_relations_unsupported_lang_is_empty() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let base_oid = {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.txt")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap()
    };
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    fs::write(dir.path().join("a.txt"), "hello world\n").unwrap();
    {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.txt")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "edit", &tree, &[&parent]).unwrap();
    }

    // Unsupported language => no symbols, no relations, no seeds (safe degradation).
    let a = super::analyze_relations(dir.path().to_str().unwrap(), "main", "target").unwrap();
    assert!(a.changed.is_empty());
    assert!(a.hints.strong.is_empty() && a.hints.weak.is_empty());
    assert!(a.seeds.is_empty());
}

// ---------------------------------------------------------------------------
// (g) base-AST signals end-to-end (planning §2.1 base/head) — real git2.
// A surviving Go file where: `gone` is deleted, `create` gains a parameter (signature
// change), and `oldName`→`newName` is a rename (body identical). All three signals must
// surface through `analyze_relations.base_signals`.
// ---------------------------------------------------------------------------

#[test]
fn e2e_analyze_relations_base_signals() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    let path = dir.path().join("svc.go");
    // base: gone(), create(name), oldName(x) {body}, anchor() to keep the file ≥2 symbols.
    fs::write(
        &path,
        "package p\n\
\n\
func gone() int {\n\
\treturn 1\n\
}\n\
\n\
func create(name string) int {\n\
\treturn len(name)\n\
}\n\
\n\
func oldName(x int) int {\n\
\ty := x + 41\n\
\treturn y\n\
}\n\
\n\
func anchor() int {\n\
\treturn 7\n\
}\n",
    )
    .unwrap();
    let base_oid = {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("svc.go")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap()
    };

    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    // head: gone removed; create gains `age int`; oldName renamed to newName (same body);
    // anchor changed so the file has ≥2 changed symbols (per-symbol cards exist).
    fs::write(
        &path,
        "package p\n\
\n\
func create(name string, age int) int {\n\
\treturn len(name) + age\n\
}\n\
\n\
func newName(x int) int {\n\
\ty := x + 41\n\
\treturn y\n\
}\n\
\n\
func anchor() int {\n\
\treturn 8\n\
}\n",
    )
    .unwrap();
    {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("svc.go")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "base signals", &tree, &[&parent]).unwrap();
    }

    let a = super::analyze_relations(dir.path().to_str().unwrap(), "main", "target").unwrap();
    let bs = &a.base_signals;

    // deleted: `gone` (and NOT the renamed oldName).
    let del_names: Vec<&str> = bs.deleted.iter().map(|d| d.name.as_str()).collect();
    assert!(del_names.contains(&"gone"), "deleted: {del_names:?}");
    assert!(!del_names.contains(&"oldName"), "rename must consume the deletion: {del_names:?}");

    // rename: oldName → newName, and the `to_card_id` is a REAL changed-symbol card id.
    assert_eq!(bs.renames.len(), 1, "renames: {:?}", bs.renames);
    assert_eq!(bs.renames[0].from_name, "oldName");
    assert_eq!(bs.renames[0].to_name, "newName");
    assert!(
        a.changed.iter().any(|c| c.card_id == bs.renames[0].to_card_id),
        "rename to_card_id must be a real changed card: {} not in {:?}",
        bs.renames[0].to_card_id,
        a.changed.iter().map(|c| &c.card_id).collect::<Vec<_>>()
    );

    // signature change: create gained `age int`.
    let sc = bs
        .signature_changes
        .iter()
        .find(|s| s.name == "create")
        .expect("create signature change");
    assert!(sc.old_signature.contains("name string"), "old: {}", sc.old_signature);
    assert!(sc.new_signature.contains("age int"), "new: {}", sc.new_signature);
    assert!(
        a.changed.iter().any(|c| c.card_id == sc.card_id),
        "sig-change card_id must be a real changed card"
    );

    // The deleted signal must carry a synthetic, path-qualified id (not a head card id).
    assert!(bs.deleted.iter().all(|d| d.id.starts_with("deleted::svc.go::")));
}

#[test]
fn e2e_base_signals_determinism() {
    // Run analyze_relations twice on the same repo state ⇒ identical base_signals.
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();
    let path = dir.path().join("m.go");
    fs::write(
        &path,
        "package p\n\nfunc removed() int {\n\treturn 1\n}\n\nfunc keep() int {\n\treturn 2\n}\n",
    )
    .unwrap();
    let base_oid = {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("m.go")).unwrap();
        idx.write().unwrap();
        let tid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap()
    };
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    fs::write(&path, "package p\n\nfunc keep() int {\n\treturn 3\n}\n").unwrap();
    {
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("m.go")).unwrap();
        idx.write().unwrap();
        let tid = idx.write_tree().unwrap();
        let tree = repo.find_tree(tid).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "edit", &tree, &[&parent]).unwrap();
    }
    let p = dir.path().to_str().unwrap();
    let a1 = super::analyze_relations(p, "main", "target").unwrap();
    let a2 = super::analyze_relations(p, "main", "target").unwrap();
    assert_eq!(a1.base_signals, a2.base_signals, "base signals must be deterministic");
}
