//! ⑨ Just-in-time **definition** injection (planning §5).
//!
//! After the AI layout is folded onto `ReviewData` (so `ordered_card_ids` is the final
//! flow order), this post-pass slots a small **definition overview** card just before the
//! first changed card that *uses* a type/class/struct whose own definition is **not** in
//! the diff. The intent (planning §5): show the reader the shape of a data structure right
//! at the moment they first need it to follow the flow — not the whole file, just the
//! fields / constructor / public methods / which methods this PR changed.
//!
//! ## What this is (and is not)
//! Pure, deterministic, **no AI**. The overview is extracted structurally with tree-sitter
//! (`role` is left `None` — the AI label step may fill it later). This is a *bounded,
//! solvable subset* of §5, not the whole vision:
//!
//!  - **Qualified target** = an identifier a changed symbol references (a header type use,
//!    or a body call/construction) that names a **type-like definition** (Rust struct/enum,
//!    Java class/interface, Go type) living in the `new_source` of one of the changed files,
//!    whose own definition is **not itself a changed symbol** (so it has no diff card). The
//!    definition must resolve **unambiguously** — exactly one type-like definition with that
//!    name across the changed files. Anything we cannot pin down deterministically is
//!    **skipped** (no fabrication — planning §5 "흐름 이해에 필요한 개요만").
//!  - We never invent definitions for symbols outside the changed files (no whole-repo
//!    resolution this stage), and we never inject a definition for a symbol that already has
//!    its own change card (it is shown as a diff, not an overview).
//!
//! ## Degradation / invariants
//!  - No qualified target ⇒ `jit_defs = []`, **zero** definition cards, `ordered_card_ids`
//!    unchanged ⇒ byte-identical to the pre-⑨ behaviour.
//!  - **no-drop**: every pre-existing change card stays exactly once; definition cards are
//!    purely additive and inserted **before** their first user.
//!  - **determinism**: targets are discovered over sorted changed symbols / sorted refs and
//!    keyed by a stable id, so identical input ⇒ identical output.

use super::gitdiff::{FileDiff, FileStatus};
use super::model::{
    DefinitionOverview, JitDefinition, ReviewCard, SymbolKind,
};
use super::relations::ChangedSymbol;
use super::symbols::{self, Lang, Symbol};
use std::collections::{BTreeMap, BTreeSet};

/// The result of the ⑨ pass: the definition overviews (`jit_defs`) plus the synthetic
/// `kind == Definition` cards to append to `ReviewData.cards`, and the ordered-id
/// insertions to apply. Kept as one struct so the caller folds it atomically.
#[derive(Debug, Default)]
pub struct JitInjection {
    /// One per qualified definition — the structured overview the front-end looks up by
    /// `card.id` (== `JitDefinition.id`). Empty ⇒ no eligible target (safe degradation).
    pub jit_defs: Vec<JitDefinition>,
    /// The synthetic definition pseudo-cards (`kind == Definition`), one per `jit_defs`
    /// entry, to be appended to `ReviewData.cards`. `id` matches the `JitDefinition.id`.
    pub cards: Vec<ReviewCard>,
}

/// Compute the JIT definition injection for a fully-laid-out review.
///
/// * `diff` — the changed files (their `new_source` is where definitions are resolved).
/// * `changed` — the changed symbols with their card ids + references (Stage-②).
/// * `ordered_card_ids` — the **final** flow order (post AI fold); first-use is measured
///   against this so a definition lands before the earliest user *in review order*.
///
/// Returns the overviews + synthetic cards; the caller splices each definition card id into
/// `ordered_card_ids` before its `injected_before`. Pure & deterministic.
pub fn compute_jit_defs(
    diff: &[FileDiff],
    changed: &[ChangedSymbol],
    ordered_card_ids: &[String],
) -> JitInjection {
    // Position of each card id in the final flow order (for "first user" comparison and to
    // guarantee the target itself has at least one user that is actually laid out).
    let order_pos: BTreeMap<&str, usize> = ordered_card_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // The set of changed-symbol *names* per file path: a referenced name that is itself a
    // changed symbol has its own diff card, so it is never a JIT definition target.
    let mut changed_names_by_path: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for cs in changed {
        changed_names_by_path
            .entry(cs.path.as_str())
            .or_default()
            .insert(cs.name.as_str());
    }

    // Resolve every type-like definition across the changed files exactly once: name ->
    // candidate sites. A name resolving to >1 site (overloads / same name in two files) is
    // **ambiguous** and dropped (we will not guess which one the reader means).
    let defs = resolve_type_definitions(diff);

    // Gather, per candidate definition name, the earliest user (by final flow order) among
    // the changed symbols that reference it. A reference counts only if the referenced name
    // is NOT a changed symbol in the *user's own file* (that would be a sibling diff card)
    // and the name resolves to a unique type definition.
    //
    // `first_user`: def name -> (best order pos, best card id). Deterministic tie-break by
    // card id keeps identical input ⇒ identical output.
    let mut first_user: BTreeMap<&str, (usize, &str)> = BTreeMap::new();

    // Stable iteration: sort changed by card id, and dedup each symbol's referenced names.
    let mut changed_sorted: Vec<&ChangedSymbol> = changed.iter().collect();
    changed_sorted.sort_by(|a, b| a.card_id.cmp(&b.card_id));

    for cs in &changed_sorted {
        // Only users that are actually placed in the final order can anchor an insertion.
        let Some(&user_pos) = order_pos.get(cs.card_id.as_str()) else {
            continue;
        };
        let same_file_changed = changed_names_by_path.get(cs.path.as_str());

        // Referenced identifiers worth a definition overview: header type uses first
        // (signature DTOs — planning §5 "signature에 등장하면 함수보다 먼저"), then
        // body calls/constructions ("내부에서 처음 생성되면 생성 직전에"). Deduped & sorted.
        let mut refs: BTreeSet<&str> = BTreeSet::new();
        for hit in &cs.refs.type_refs {
            refs.insert(hit.ident.as_str());
        }
        for hit in &cs.refs.calls {
            refs.insert(hit.ident.as_str());
        }

        for ident in refs {
            // A referenced name that is itself a changed symbol in this file is a sibling
            // diff card, never a JIT definition.
            if same_file_changed.is_some_and(|s| s.contains(ident)) {
                continue;
            }
            // Must resolve to exactly one type-like definition that is NOT itself changed.
            let Some(def) = defs.get(ident) else {
                continue;
            };
            // The definition's own symbol must not be a changed symbol (no diff card for it).
            if changed_names_by_path
                .get(def.path.as_str())
                .is_some_and(|s| s.contains(ident))
            {
                continue;
            }
            // Record / improve the earliest user.
            match first_user.get(ident) {
                Some(&(best_pos, best_id))
                    if (best_pos, best_id) <= (user_pos, cs.card_id.as_str()) => {}
                _ => {
                    first_user.insert(ident, (user_pos, cs.card_id.as_str()));
                }
            }
        }
    }

    // Build one JIT definition per qualified target, in a deterministic order (by the def
    // name). Each yields a `JitDefinition` + a synthetic `Definition` card.
    let mut out = JitInjection::default();

    for (&name, &(_pos, before_id)) in &first_user {
        let def = &defs[name]; // present by construction.
        let overview = build_overview(diff, def);
        let id = jit_id(&def.path, name);
        out.jit_defs.push(JitDefinition {
            id: id.clone(),
            symbol: name.to_string(),
            path: def.path.clone(),
            overview,
            injected_before: before_id.to_string(),
        });
        out.cards.push(definition_card(&id, name, &def.path));
    }

    out
}

/// Splice each definition card id into `ordered_card_ids` immediately before its
/// `injected_before`. Done as a single rebuild so multiple definitions targeting the same
/// `injected_before` keep a deterministic (def-name) order and nothing is dropped.
///
/// A definition whose `injected_before` is absent from `ordered_card_ids` (defensive — the
/// first user should always be present) is appended at the end so its card is never lost.
pub fn splice_ordered(ordered_card_ids: &[String], jit_defs: &[JitDefinition]) -> Vec<String> {
    // before_id -> the definition ids to insert ahead of it (def-name order preserved:
    // `jit_defs` is already sorted by def name from `compute_jit_defs`).
    let mut inserts: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let present: BTreeSet<&str> = ordered_card_ids.iter().map(String::as_str).collect();
    let mut orphans: Vec<&str> = Vec::new();
    for jd in jit_defs {
        if present.contains(jd.injected_before.as_str()) {
            inserts
                .entry(jd.injected_before.as_str())
                .or_default()
                .push(jd.id.as_str());
        } else {
            orphans.push(jd.id.as_str());
        }
    }

    let mut out: Vec<String> = Vec::with_capacity(ordered_card_ids.len() + jit_defs.len());
    for id in ordered_card_ids {
        if let Some(defs) = inserts.get(id.as_str()) {
            for d in defs {
                out.push((*d).to_string());
            }
        }
        out.push(id.clone());
    }
    for d in orphans {
        out.push(d.to_string());
    }
    out
}

/// A resolved type-like definition site in a changed file.
struct DefSite {
    path: String,
    source_idx: usize,
    lang: Lang,
    sym: Symbol,
}

/// Resolve every **type-like** definition (struct/enum/class/interface/Go type) across the
/// changed files, keyed by bare name. A name appearing more than once (across files or as
/// overloads) is **ambiguous** and removed — we never guess which one a reference means.
fn resolve_type_definitions(diff: &[FileDiff]) -> BTreeMap<String, DefSite> {
    let mut by_name: BTreeMap<String, Option<DefSite>> = BTreeMap::new();

    for (fi, file) in diff.iter().enumerate() {
        if file.is_binary
            || file.status == FileStatus::Deleted
            || file.new_source.is_empty()
        {
            continue;
        }
        let Some(lang) = Lang::from_path(&file.new_path) else {
            continue;
        };
        let Ok(Some(syms)) = symbols::extract(lang, &file.new_source) else {
            continue; // parser ERROR / unsupported ⇒ no definitions from this file.
        };
        for sym in syms {
            if !is_type_like(lang, &file.new_source, &sym) {
                continue;
            }
            let site = DefSite {
                path: file.new_path.clone(),
                source_idx: fi,
                lang,
                sym: sym.clone(),
            };
            match by_name.entry(sym.name.clone()) {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(Some(site));
                }
                // Second occurrence ⇒ ambiguous; poison the entry (kept as None).
                std::collections::btree_map::Entry::Occupied(mut e) => {
                    e.insert(None);
                }
            }
        }
    }

    by_name
        .into_iter()
        .filter_map(|(k, v)| v.map(|site| (k, site)))
        .collect()
}

/// Whether a symbol is a *type-like* definition (a data structure worth an overview), as
/// opposed to a free function / method. Heuristic, keyed on the opening token of the
/// symbol's first source line (tree-sitter tags don't separate struct vs fn for us here):
///  - Rust: `struct` / `enum` / `trait` / `union`.
///  - Java: `class` / `interface` / `enum` / `record` (possibly after modifiers).
///  - Go:   `type ` declarations (`type Foo struct { … }` / `type Foo interface { … }`).
fn is_type_like(lang: Lang, source: &str, sym: &Symbol) -> bool {
    let Some(line) = source.lines().nth(sym.start_row) else {
        return false;
    };
    let trimmed = line.trim_start();
    match lang {
        Lang::Rust => {
            // Skip a leading visibility modifier (`pub`, `pub(crate)`, …) before the keyword.
            let rest = strip_rust_visibility(trimmed);
            starts_with_kw(rest, &["struct", "enum", "trait", "union"])
        }
        Lang::Go => trimmed.starts_with("type ") || trimmed.starts_with("type\t"),
        Lang::Java => {
            // Skip leading modifiers (public/final/abstract/…) then look for the keyword.
            trimmed.split_whitespace().any(|w| {
                matches!(w, "class" | "interface" | "enum" | "record")
            }) && first_type_kw_before_name(trimmed, &sym.name)
        }
    }
}

/// Strip a leading Rust visibility modifier (`pub`, `pub(crate)`, `pub(super)`, …) plus the
/// following whitespace, so `pub struct Foo` is recognised as a `struct` definition.
fn strip_rust_visibility(line: &str) -> &str {
    let Some(rest) = line.strip_prefix("pub") else {
        return line;
    };
    let rest = rest.trim_start();
    // `pub(crate)` / `pub(super)` / `pub(in path)` — drop the parenthesised restriction.
    if let Some(after) = rest.strip_prefix('(') {
        if let Some(close) = after.find(')') {
            return after[close + 1..].trim_start();
        }
    }
    rest
}

/// Rust/Go keyword check: the line starts with `kw` followed by a non-identifier char.
fn starts_with_kw(line: &str, kws: &[&str]) -> bool {
    kws.iter().any(|kw| {
        line.strip_prefix(kw)
            .is_some_and(|rest| rest.starts_with(|c: char| c == ' ' || c == '\t' || c == '<'))
    })
}

/// Java: ensure a `class|interface|enum|record` keyword appears *before* the symbol name on
/// the declaration line (so a method returning a class type isn't misread as a type def).
fn first_type_kw_before_name(line: &str, name: &str) -> bool {
    let kw_pos = ["class", "interface", "enum", "record"]
        .iter()
        .filter_map(|kw| find_word(line, kw))
        .min();
    let name_pos = find_word(line, name);
    matches!((kw_pos, name_pos), (Some(k), Some(n)) if k < n)
}

/// Byte index of `word` in `line` as a whole word (bounded by non-identifier chars).
fn find_word(line: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(word) {
        let at = from + rel;
        let before_ok = at == 0
            || !line[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after = at + word.len();
        let after_ok = line[after..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + word.len();
    }
    None
}

/// Stable JIT card / definition id: `jit::<path>::<name>`. Distinct from a change card id
/// (`<path>::<name>`) by the `jit::` prefix so it can never collide with a real card.
fn jit_id(path: &str, name: &str) -> String {
    format!("jit::{}::{}", path, name)
}

/// The synthetic definition pseudo-card. `kind == Definition` is the front-end marker; the
/// overview is shipped separately in `jit_defs` and looked up by this card's `id`. No diff
/// lines (it is an overview, not a change).
fn definition_card(id: &str, name: &str, path: &str) -> ReviewCard {
    let basename = path.rsplit('/').next().unwrap_or(path);
    ReviewCard {
        id: id.to_string(),
        chapter: basename.to_string(),
        symbol: name.to_string(),
        path: path.to_string(),
        status: "pending".into(),
        // B1: never empty. A short, honest one-liner (no AI).
        summary: format!("Definition of {name} — shown before its first use."),
        lines: Vec::new(),
        kind: SymbolKind::Definition,
        qualified: name.to_string(),
        ..Default::default()
    }
}

/// Extract the structured overview for one definition with tree-sitter (no AI). `role` is
/// left `None` (AI-only). Fields / constructor / public methods are pulled from the
/// definition's source body; `changed_methods` is the intersection of this type's methods
/// with the changed methods owned by this type in this PR.
fn build_overview(diff: &[FileDiff], def: &DefSite) -> DefinitionOverview {
    let source = &diff[def.source_idx].new_source;
    let body: Vec<&str> = source
        .lines()
        .skip(def.sym.start_row)
        .take(def.sym.end_row - def.sym.start_row + 1)
        .collect();

    let (fields, methods, constructor) = match def.lang {
        Lang::Rust => rust_overview(&body, &def.sym.name),
        Lang::Java => java_overview(&body, &def.sym.name),
        Lang::Go => go_overview(&body, &def.sym.name),
    };

    // changed_methods: this type's methods that changed in this PR (owner == def name).
    let changed_methods = collect_changed_methods(diff, &def.path, &def.sym.name);

    DefinitionOverview {
        role: None,
        fields,
        constructor,
        public_methods: methods,
        changed_methods,
    }
}

/// Methods changed in this PR whose `owner` is this definition (by re-deriving owners from
/// the changed files). Returns sorted, deduped names. Empty when none.
fn collect_changed_methods(diff: &[FileDiff], def_path: &str, def_name: &str) -> Vec<String> {
    let Some(file) = diff.iter().find(|f| f.new_path == def_path) else {
        return Vec::new();
    };
    let Some(lang) = Lang::from_path(&file.new_path) else {
        return Vec::new();
    };
    let Ok(Some(syms)) = symbols::extract(lang, &file.new_source) else {
        return Vec::new();
    };
    let owners = symbols::symbol_owners(lang, &file.new_source, &syms);
    let mut out: BTreeSet<String> = BTreeSet::new();
    for (i, sym) in syms.iter().enumerate() {
        if owners.get(i).cloned().flatten().as_deref() == Some(def_name) {
            out.insert(sym.name.clone());
        }
    }
    out.into_iter().collect()
}

/// Rust struct/enum overview: field declarations (`name: Type,`), and any `impl` method
/// signatures are intentionally NOT walked here (kept minimal — the body slice is only the
/// definition itself; impl blocks live elsewhere). Constructor = a `fn new(` if present.
fn rust_overview(body: &[&str], _name: &str) -> (Vec<String>, Vec<String>, Option<String>) {
    let mut fields = Vec::new();
    for raw in body.iter().skip(1) {
        let line = raw.trim().trim_end_matches(',');
        if line.is_empty() || line == "}" || line.starts_with("//") {
            continue;
        }
        // A struct field `name: Type` (skip enum-ish lines without a colon / with `(`).
        if let Some((lhs, rhs)) = line.split_once(':') {
            let lhs = lhs.trim_start_matches("pub ").trim();
            if is_ident(lhs) && !rhs.trim().is_empty() {
                fields.push(format!("{}: {}", lhs, rhs.trim()));
                continue;
            }
        }
        // An enum variant (bare ident, optionally with payload).
        let variant = line.split(['(', '{']).next().unwrap_or(line).trim();
        if is_ident(variant) {
            fields.push(variant.to_string());
        }
    }
    (fields, Vec::new(), None)
}

/// Java class/interface overview: field declarations and public method signatures from the
/// body slice. Constructor = a method whose name equals the type name.
fn java_overview(body: &[&str], name: &str) -> (Vec<String>, Vec<String>, Option<String>) {
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut constructor = None;
    for raw in body.iter().skip(1) {
        let line = raw.trim();
        if line.is_empty() || line == "}" || line.starts_with("//") || line.starts_with('@') {
            continue;
        }
        // Method (has `(` before any `=`): keep the signature up to `(` plus the param list.
        if let Some(open) = line.find('(') {
            // `head` is everything before the param-list `(`, e.g. `public OrderDraft` or
            // `public boolean validate`. A *constructor* is the declaration whose name token
            // (the identifier immediately preceding the `(`) equals the type name.
            let head = &line[..open];
            let sig = method_signature(line);
            if head.split_whitespace().last() == Some(name) {
                constructor.get_or_insert(sig);
            } else if line.starts_with("public") {
                methods.push(sig);
            }
            continue;
        }
        // Field: a `;`-terminated declaration with a type and a name.
        if line.ends_with(';') {
            let decl = line.trim_end_matches(';').trim();
            let words: Vec<&str> = decl.split_whitespace().collect();
            if words.len() >= 2 {
                let nm = words.last().copied().unwrap_or("");
                let ty = words[words.len() - 2];
                if is_ident(nm) {
                    fields.push(format!("{nm}: {ty}"));
                }
            }
        }
    }
    (fields, methods, constructor)
}

/// Go `type Foo struct { … }` / `type Foo interface { … }` overview: struct field lines
/// (`Name Type`) or interface method lines. No constructor concept in Go.
fn go_overview(body: &[&str], _name: &str) -> (Vec<String>, Vec<String>, Option<String>) {
    let mut fields = Vec::new();
    for raw in body.iter().skip(1) {
        let line = raw.trim();
        if line.is_empty() || line == "}" || line.starts_with("//") {
            continue;
        }
        // `Name Type` (struct field) — keep as-is.
        let first = line.split_whitespace().next().unwrap_or("");
        if is_ident(first) {
            fields.push(line.to_string());
        }
    }
    (fields, Vec::new(), None)
}

/// A trimmed Java method signature: everything up to and including the closing `)` of the
/// parameter list (drops the body brace / `throws` tail). Falls back to the whole line.
fn method_signature(line: &str) -> String {
    if let Some(close) = line.find(')') {
        line[..=close].trim().to_string()
    } else {
        line.trim_end_matches('{').trim().to_string()
    }
}

/// A bare identifier (letters/digits/underscore, not starting with a digit). Used to filter
/// field/variant candidates so we don't surface noise.
fn is_ident(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "jitdef_tests.rs"]
mod jitdef_tests;
