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
