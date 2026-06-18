//! tree-sitter symbol-boundary extraction. Knows only tree-sitter — no git.
//!
//! Input: a language + the *new* file source (`&str`). Output: `Vec<Symbol>` with
//! 0-base inclusive row ranges in the new-file coordinate system.
//!
//! Spike-confirmed API facts honoured here:
//!  - loader is `tree_sitter_go::LANGUAGE` (a `LanguageFn`); convert via `.into()`.
//!  - `QueryCursor::matches` is a `StreamingIterator`, not `std::iter::Iterator`.
//!  - capture names differ per language; `@name` is consistent across all three.
//!  - `end_position().row` is 0-base and is the row of the node's last byte.

use super::EngineError;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

/// One extracted definition (function / method / type / class / interface / …).
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    /// The `@name` text (bare identifier).
    pub name: String,
    /// Display name; same as `name` in stage 1 (qualification deferred — see note).
    pub qualified: String,
    /// 0-base inclusive start row (node.start_position().row).
    pub start_row: usize,
    /// 0-base inclusive end row (node.end_position().row — the row of the closing brace).
    pub end_row: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Go,
    Java,
    Rust,
}

impl Lang {
    /// Dispatch by file extension. `None` => unsupported (.kt and everything else)
    /// => file-level fallback card (no parsing).
    pub fn from_path(path: &str) -> Option<Lang> {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "go" => Some(Lang::Go),
            "java" => Some(Lang::Java),
            "rs" => Some(Lang::Rust),
            _ => None,
        }
    }

    fn language(self) -> tree_sitter::Language {
        match self {
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }

    fn tags_query(self) -> &'static str {
        match self {
            Lang::Go => tree_sitter_go::TAGS_QUERY,
            Lang::Java => tree_sitter_java::TAGS_QUERY,
            Lang::Rust => tree_sitter_rust::TAGS_QUERY,
        }
    }

    /// Capture names (without the leading `@`) that denote a *symbol boundary*.
    /// These differ by language — the spike confirmed the exact sets below.
    fn definition_captures(self) -> &'static [&'static str] {
        match self {
            // Go has no class/interface tags.
            Lang::Go => &["definition.function", "definition.method", "definition.type"],
            // Java has no top-level function tag.
            Lang::Java => &[
                "definition.class",
                "definition.interface",
                "definition.method",
            ],
            Lang::Rust => &[
                "definition.function",
                "definition.method",
                "definition.class",
                "definition.interface",
                "definition.macro",
                "definition.module",
            ],
        }
    }

    /// B1 (v2-critique, verified against tags.scm) — the `@reference.*` capture set is
    /// **asymmetric per language**. A single code path cannot pull `.type`/`.class`
    /// from every grammar; each language only carries the captures listed below, so the
    /// corresponding `SymbolRefs` bucket is left *empty* (not faked) where a language
    /// lacks a capture. The asymmetry is intentional and load-bearing:
    ///
    ///  - Rust: `reference.call`, `reference.implementation`  (NO `.type`, NO `.class`)
    ///          → `calls` + `impls` populated; `type_refs` always empty.
    ///  - Go:   `reference.call`, `reference.type`            (NO `.class`, NO `.impl`)
    ///          → `calls` + `type_refs` populated; `impls` always empty.
    ///  - Java: `reference.call`, `reference.class`,
    ///          `reference.implementation`                    (NO `.type`)
    ///          → `calls` + `impls` populated; `type_refs` from `.class` (object
    ///            creation / superclass / extends are the only "type usage" Java tags).
    ///
    /// Returned as `(call_caps, type_ref_caps, impl_caps)` capture-name slices so the
    /// extraction loop can bucket each reference `@name` by which kind capture co-occurs
    /// in its match. `RelationHints` consumers must treat a missing bucket as "this
    /// language can't express this signal", never as "no such relation".
    fn reference_captures(self) -> RefCaptureMap {
        match self {
            Lang::Rust => RefCaptureMap {
                call: &["reference.call"],
                // Rust grammar has no `reference.type`; signature/type relations are
                // recovered separately (see `rust_type_refs`), not from tags.scm.
                type_ref: &[],
                impl_: &["reference.implementation"],
            },
            Lang::Go => RefCaptureMap {
                call: &["reference.call"],
                type_ref: &["reference.type"],
                // Go grammar has no implementation/class reference tag.
                impl_: &[],
            },
            Lang::Java => RefCaptureMap {
                call: &["reference.call"],
                // Java has no `reference.type`; `reference.class` (object creation /
                // superclass / `extends`) is the closest "type usage" signal.
                type_ref: &["reference.class"],
                impl_: &["reference.implementation"],
            },
        }
    }
}

/// Per-language mapping of which `@reference.*` capture names feed each `SymbolRefs`
/// bucket (B1). An empty slice means "this grammar cannot express this reference kind"
/// — the bucket is then always empty for that language (documented asymmetry).
struct RefCaptureMap {
    call: &'static [&'static str],
    type_ref: &'static [&'static str],
    impl_: &'static [&'static str],
}

/// A single reference occurrence (call / type-use / impl) attributed to a symbol.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefHit {
    /// The bare identifier text of the referenced name (e.g. `apply`, `OrderItem`).
    pub ident: String,
    /// 0-base row of the reference site.
    pub row: usize,
    /// True when the reference sits in the symbol's *header* (signature / first line),
    /// i.e. a likely type/parameter relation rather than a body call. Header refs are
    /// what feed signature-type strong relations.
    pub in_header: bool,
}

/// References attributed to one symbol (by index into the parallel `Vec<Symbol>`).
///
/// The three buckets are populated **asymmetrically per language** (B1, see
/// `reference_captures`): a bucket a grammar cannot express is left empty rather than
/// faked. `RelationHints` must read each bucket as "signal present" vs "language can't
/// express", never as a graph edge.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolRefs {
    /// Index of the owning symbol in the returned `Vec<Symbol>`.
    pub sym_idx: usize,
    /// Direct call references (`reference.call`) — all three languages.
    pub calls: Vec<RefHit>,
    /// Type usages: Go `reference.type`, Java `reference.class`. **Empty for Rust**
    /// (the grammar has no such tag; recovered via `rust_type_refs`).
    pub type_refs: Vec<RefHit>,
    /// Impl/trait relations: Rust `reference.implementation`, Java
    /// `reference.implementation`. **Empty for Go** (grammar has no such tag).
    pub impls: Vec<RefHit>,
}

/// Extract symbols from the new-file source. On a parser ERROR (`has_error`) the
/// caller is told via `Ok(None)` to fall back to a file-level card; never panics.
pub fn extract(lang: Lang, source: &str) -> Result<Option<Vec<Symbol>>, EngineError> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.language())
        .map_err(|e| EngineError::Parse(format!("set_language failed: {e}")))?;

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Err(EngineError::Parse("parse returned None".into())),
    };

    if tree.root_node().has_error() {
        // Parser ERROR => file-level fallback (signal with None).
        return Ok(None);
    }

    let language = lang.language();
    let query = Query::new(&language, lang.tags_query())
        .map_err(|e| EngineError::Parse(format!("query compile failed: {e}")))?;

    // Pre-resolve the capture indices we care about.
    let def_capture_names = lang.definition_captures();
    let name_idx = query.capture_index_for_name("name");

    let src_bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src_bytes);

    let mut symbols: Vec<Symbol> = Vec::new();

    // StreamingIterator: must use while-let + .next(), not for/map.
    while let Some(m) = matches.next() {
        // Find the definition node (the symbol boundary) and the @name node within
        // this single match.
        let mut def_node = None;
        let mut name_text: Option<String> = None;

        for cap in m.captures {
            let cap_name = query.capture_names()[cap.index as usize];
            if def_capture_names.contains(&cap_name) {
                def_node = Some(cap.node);
            }
            if Some(cap.index) == name_idx {
                if let Ok(t) = cap.node.utf8_text(src_bytes) {
                    name_text = Some(t.to_string());
                }
            }
        }

        if let Some(node) = def_node {
            let name = name_text.unwrap_or_else(|| "?".to_string());
            symbols.push(Symbol {
                qualified: name.clone(),
                name,
                start_row: node.start_position().row,
                end_row: node.end_position().row,
            });
        }
    }

    // Deterministic order: by start_row, then by narrower range first.
    symbols.sort_by(|a, b| {
        a.start_row
            .cmp(&b.start_row)
            .then((a.end_row - a.start_row).cmp(&(b.end_row - b.start_row)))
            .then(a.name.cmp(&b.name))
    });

    Ok(Some(symbols))
}

/// Like [`extract`], but in the **same `TAGS_QUERY` pass** also collects reference
/// captures and attributes each to its enclosing (innermost) symbol — zero extra
/// parses. Returns `(symbols, refs)` where `refs[i].sym_idx` indexes into `symbols`.
///
/// B1 (v2-critique): the reference buckets are filled **asymmetrically per language**
/// via `Lang::reference_captures` — Rust has no `type_refs`, Go has no `impls`. A bucket
/// a grammar can't express stays empty (never faked). For Rust, signature *type* relations
/// (absent from tags.scm) are recovered with a small, targeted `type_identifier` /
/// `scoped_type_identifier` walk over each symbol header — kept deliberately minimal.
///
/// Safety/degradation: same contract as [`extract`] — parser ERROR or no parse =>
/// `Ok(None)` so the caller falls back to a file-level card with **no** relations.
pub fn extract_with_refs(
    lang: Lang,
    source: &str,
) -> Result<Option<(Vec<Symbol>, Vec<SymbolRefs>)>, EngineError> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.language())
        .map_err(|e| EngineError::Parse(format!("set_language failed: {e}")))?;

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Err(EngineError::Parse("parse returned None".into())),
    };

    if tree.root_node().has_error() {
        // Parser ERROR => file-level fallback, and crucially **no relations** (safe
        // degradation: refs left empty rather than half-parsed).
        return Ok(None);
    }

    let language = lang.language();
    let query = Query::new(&language, lang.tags_query())
        .map_err(|e| EngineError::Parse(format!("query compile failed: {e}")))?;

    let def_capture_names = lang.definition_captures();
    let ref_caps = lang.reference_captures();
    let name_idx = query.capture_index_for_name("name");

    let src_bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src_bytes);

    let mut symbols: Vec<Symbol> = Vec::new();
    // Raw reference hits in source order, bucketed by kind, before symbol attribution.
    let mut raw_calls: Vec<RawRef> = Vec::new();
    let mut raw_type_refs: Vec<RawRef> = Vec::new();
    let mut raw_impls: Vec<RawRef> = Vec::new();

    while let Some(m) = matches.next() {
        // One match may be a *definition* (has a def capture) or a *reference* (has a
        // reference.* capture). Both carry a `@name`. We classify by which captures the
        // match contains; per-language `ref_caps` decides the bucket.
        let mut def_node = None;
        let mut name_node = None;
        let mut name_text: Option<String> = None;
        let mut is_call = false;
        let mut is_type_ref = false;
        let mut is_impl = false;

        for cap in m.captures {
            let cap_name = query.capture_names()[cap.index as usize];
            if def_capture_names.contains(&cap_name) {
                def_node = Some(cap.node);
            }
            if ref_caps.call.contains(&cap_name) {
                is_call = true;
            }
            if ref_caps.type_ref.contains(&cap_name) {
                is_type_ref = true;
            }
            if ref_caps.impl_.contains(&cap_name) {
                is_impl = true;
            }
            if Some(cap.index) == name_idx {
                name_node = Some(cap.node);
                if let Ok(t) = cap.node.utf8_text(src_bytes) {
                    name_text = Some(t.to_string());
                }
            }
        }

        if let Some(node) = def_node {
            // A definition match (symbol boundary) — same as `extract`.
            let name = name_text.clone().unwrap_or_else(|| "?".to_string());
            symbols.push(Symbol {
                qualified: name.clone(),
                name,
                start_row: node.start_position().row,
                end_row: node.end_position().row,
            });
            continue;
        }

        // A reference match. Its identifier is the `@name` text; its row is the name
        // node's row (Java's `reference.call` sits on a sibling argument_list, so we
        // always anchor on `@name`, never the reference-capture node).
        if let (Some(text), Some(nnode)) = (name_text, name_node) {
            let row = nnode.start_position().row;
            // A bare Go `type_identifier` matches BOTH the type-def `@name` and the
            // standalone `(type_identifier) @reference.type`; the def arm above already
            // consumed the definition match, so here we only see genuine references.
            if is_call {
                raw_calls.push(RawRef {
                    ident: text.clone(),
                    row,
                    block: None,
                });
            }
            if is_type_ref {
                raw_type_refs.push(RawRef {
                    ident: text.clone(),
                    row,
                    block: None,
                });
            }
            if is_impl {
                // An impl/trait ref names a TYPE on the block header (Rust `impl ... {}`,
                // Java `class X implements Y`/`extends Y`). That header row is usually
                // OUTSIDE any extracted symbol, so we also record the enclosing block
                // range and attribute the impl ref to the methods *inside* the block —
                // that is exactly the trait↔implementor relation we want.
                let block = enclosing_impl_block(nnode).map(|n| {
                    (n.start_position().row, n.end_position().row)
                });
                raw_impls.push(RawRef {
                    ident: text,
                    row,
                    block,
                });
            }
        }
    }

    // Deterministic symbol order (same as `extract`) BEFORE attribution so `sym_idx`
    // is stable and matches a later `extract` call.
    symbols.sort_by(|a, b| {
        a.start_row
            .cmp(&b.start_row)
            .then((a.end_row - a.start_row).cmp(&(b.end_row - b.start_row)))
            .then(a.name.cmp(&b.name))
    });

    // Attribute each raw reference to its innermost enclosing symbol.
    let mut refs: Vec<SymbolRefs> = (0..symbols.len())
        .map(|i| SymbolRefs {
            sym_idx: i,
            ..Default::default()
        })
        .collect();

    attribute_refs(&symbols, &raw_calls, &mut refs, |r| &mut r.calls);
    attribute_refs(&symbols, &raw_type_refs, &mut refs, |r| &mut r.type_refs);
    attribute_impls(&symbols, &raw_impls, &mut refs);

    // B1 Rust gap: tags.scm gives Rust no `reference.type`, so signature type relations
    // (the v2 §4.4 strong signal: Request→Command→Entity) would be invisible. Recover
    // them with a targeted header walk — bounded to each symbol's first line(s), never a
    // whole-file re-scan (kept minimal per the critique).
    if lang == Lang::Rust {
        rust_type_refs(&tree, src_bytes, &symbols, &mut refs);
    }

    Ok(Some((symbols, refs)))
}

/// Best-effort enclosing type/class name for each symbol, parallel to `symbols`.
///
/// Used for the "same-class helper" strong relation (planning §4.4). Derived by walking
/// each symbol's start node up to its nearest type-defining ancestor:
///  - Rust: the `impl_item`'s `type:` identifier (methods live in `declaration_list`).
///  - Java: the enclosing `class_declaration` / `interface_declaration` name.
///  - Go:   the method `receiver` type (`func (s *Session) M()` → `Session`).
///
/// `None` when the symbol is free-standing (top-level function, the type itself, …).
/// Reuses the same deterministic `symbols` ordering as `extract`/`extract_with_refs`, so
/// the i-th owner matches the i-th symbol. Returns `None`-filled vec on parse error.
pub fn symbol_owners(lang: Lang, source: &str, symbols: &[Symbol]) -> Vec<Option<String>> {
    let mut parser = Parser::new();
    if parser.set_language(&lang.language()).is_err() {
        return vec![None; symbols.len()];
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return vec![None; symbols.len()],
    };
    let src_bytes = source.as_bytes();

    symbols
        .iter()
        .map(|s| owner_of(lang, &tree, src_bytes, s))
        .collect()
}

/// Find the enclosing type name for one symbol by locating the smallest node at the
/// symbol's start position and walking ancestors to the nearest type definition.
fn owner_of(
    lang: Lang,
    tree: &tree_sitter::Tree,
    src_bytes: &[u8],
    sym: &Symbol,
) -> Option<String> {
    let point = tree_sitter::Point {
        row: sym.start_row,
        column: 0,
    };
    let mut node = tree
        .root_node()
        .descendant_for_point_range(point, point)?;

    loop {
        let parent = node.parent()?;
        match (lang, parent.kind()) {
            // Rust: method inside `impl Type { ... }` → the impl's `type:` field.
            (Lang::Rust, "impl_item") => {
                if let Some(t) = parent.child_by_field_name("type") {
                    return t
                        .utf8_text(src_bytes)
                        .ok()
                        .map(|s| s.rsplit("::").next().unwrap_or(s).to_string());
                }
            }
            // Java: method/field inside a class/interface → its `name:` field.
            (Lang::Java, "class_declaration" | "interface_declaration") => {
                if let Some(n) = parent.child_by_field_name("name") {
                    return n.utf8_text(src_bytes).ok().map(str::to_string);
                }
            }
            // Go: a method declaration carries its receiver type directly.
            (Lang::Go, "method_declaration") => {
                return go_receiver_type(parent, src_bytes);
            }
            _ => {}
        }
        node = parent;
    }
}

/// Extract the receiver type name from a Go `method_declaration` node, stripping any
/// leading `*` (pointer receiver). `func (s *Session) M()` → `Session`.
fn go_receiver_type(method: tree_sitter::Node, src_bytes: &[u8]) -> Option<String> {
    let receiver = method.child_by_field_name("receiver")?;
    // The receiver is a parameter_list; find the first type_identifier within it.
    let mut cursor = receiver.walk();
    let mut stack = vec![receiver];
    while let Some(n) = stack.pop() {
        if n.kind() == "type_identifier" {
            return n.utf8_text(src_bytes).ok().map(str::to_string);
        }
        for c in n.children(&mut cursor) {
            stack.push(c);
        }
    }
    None
}

/// A reference occurrence before it is attributed to a symbol. `block` (impl refs only)
/// is the enclosing impl/inheritance block's inclusive row range, used to attribute the
/// ref to the methods inside that block when the header row sits outside every symbol.
#[derive(Debug, Clone)]
struct RawRef {
    ident: String,
    row: usize,
    block: Option<(usize, usize)>,
}

/// Walk up from an impl/trait reference's `@name` node to the nearest block node whose
/// member symbols the relation applies to: Rust `impl_item`, Java `class_declaration` /
/// `interface_declaration` (for `implements`/`extends`).
fn enclosing_impl_block(name_node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    let mut node = name_node;
    loop {
        let parent = node.parent()?;
        match parent.kind() {
            "impl_item" | "class_declaration" | "interface_declaration" => {
                return Some(parent);
            }
            _ => node = parent,
        }
    }
}

/// Attribute each raw reference to the innermost symbol whose row range contains it,
/// pushing a `RefHit` into the bucket chosen by `pick`. `in_header` is true when the
/// reference sits on the symbol's start row (its signature line).
fn attribute_refs(
    symbols: &[Symbol],
    raws: &[RawRef],
    refs: &mut [SymbolRefs],
    pick: impl Fn(&mut SymbolRefs) -> &mut Vec<RefHit>,
) {
    for raw in raws {
        if let Some(sym_idx) = innermost_symbol_at(symbols, raw.row) {
            let in_header = raw.row == symbols[sym_idx].start_row;
            pick(&mut refs[sym_idx]).push(RefHit {
                ident: raw.ident.clone(),
                row: raw.row,
                in_header,
            });
        }
    }
}

/// Attribute impl/trait references (B1: Rust/Java only). If the impl ref's row is inside
/// a symbol, attribute there (the common case for, e.g., a method named in a `where`
/// bound). Otherwise — the usual case, the ref names the trait/superclass on the block
/// header outside every symbol — attribute it to **every symbol contained in the block**
/// (the impl's member methods): that is the trait↔implementor relation. If neither
/// applies (no block, no enclosing symbol), the ref is dropped (defensive).
fn attribute_impls(symbols: &[Symbol], raws: &[RawRef], refs: &mut [SymbolRefs]) {
    for raw in raws {
        if let Some(sym_idx) = innermost_symbol_at(symbols, raw.row) {
            let in_header = raw.row == symbols[sym_idx].start_row;
            refs[sym_idx].impls.push(RefHit {
                ident: raw.ident.clone(),
                row: raw.row,
                in_header,
            });
            continue;
        }
        if let Some((bs, be)) = raw.block {
            for (i, s) in symbols.iter().enumerate() {
                // Symbols fully contained in the block are this impl's members.
                if s.start_row >= bs && s.end_row <= be {
                    refs[i].impls.push(RefHit {
                        ident: raw.ident.clone(),
                        row: raw.row,
                        in_header: true,
                    });
                }
            }
        }
    }
}

/// Innermost (narrowest) symbol whose inclusive row range contains `row`. Mirrors the
/// attribution rule in `cards.rs::innermost_symbol`.
fn innermost_symbol_at(symbols: &[Symbol], row: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_width = usize::MAX;
    for (i, s) in symbols.iter().enumerate() {
        if s.start_row <= row && row <= s.end_row {
            let width = s.end_row - s.start_row;
            if width < best_width {
                best_width = width;
                best = Some(i);
            }
        }
    }
    best
}

/// B1 Rust-only: recover signature *type* relations the Rust tags.scm cannot express.
/// For each symbol whose header (start row) carries `type_identifier` /
/// `scoped_type_identifier` nodes (parameter/return types, field types), record them as
/// `type_refs` with `in_header = true`. Deliberately narrow: we only walk nodes that
/// *start on the symbol's start row*, so this is a header-only signal, not a body scan.
fn rust_type_refs(
    tree: &tree_sitter::Tree,
    src_bytes: &[u8],
    symbols: &[Symbol],
    refs: &mut [SymbolRefs],
) {
    use std::collections::HashSet;
    // Collect (ident, row) of type identifiers appearing on each symbol's start row.
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut stack = vec![root];
    // Dedup per (sym_idx, ident) so a multi-token type on the header isn't double-added.
    let mut seen: HashSet<(usize, String)> = HashSet::new();

    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if kind == "type_identifier" || kind == "scoped_type_identifier" {
            let row = node.start_position().row;
            // Only header-row type uses (signature), and only inside a symbol.
            if let Some(sym_idx) = innermost_symbol_at(symbols, row) {
                if row == symbols[sym_idx].start_row {
                    if let Ok(text) = node.utf8_text(src_bytes) {
                        // For scoped types (`crate::Foo`) keep the final segment as the
                        // bare name so it matches a changed symbol's bare `name`.
                        let ident = text.rsplit("::").next().unwrap_or(text).to_string();
                        if seen.insert((sym_idx, ident.clone())) {
                            refs[sym_idx].type_refs.push(RefHit {
                                ident,
                                row,
                                in_header: true,
                            });
                        }
                    }
                }
            }
        }
        // Walk children.
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}
