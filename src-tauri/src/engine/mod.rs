//! Stage-1 review engine. Reads a `base...target` git diff, extracts symbol
//! boundaries with tree-sitter, and maps "changed lines ∩ innermost symbol" into
//! `ReviewCard[]` for the front-end.
//!
//! Dependency direction: lib -> mod -> {gitdiff, symbols, cards} -> model.
//! `build_review` is Tauri-independent (a pure function) so it can be called
//! directly from `cargo test`.

mod branches;
mod cards;
mod gitdiff;
mod model;
mod symbols;

pub use branches::list_branches;
// Re-exported for downstream/testing consumers of the contract type.
#[allow(unused_imports)]
pub use branches::Branches;
pub use model::ReviewData;
// Re-exported for downstream/testing consumers of the contract types.
#[allow(unused_imports)]
pub use model::{ReviewCard, ReviewLine};

#[derive(Debug)]
pub enum EngineError {
    Git(git2::Error),
    Parse(String),
    Io(std::io::Error),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Git(e) => write!(f, "Git error: {}", e.message()),
            EngineError::Parse(s) => write!(f, "Parse error: {s}"),
            EngineError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<git2::Error> for EngineError {
    fn from(e: git2::Error) -> Self {
        EngineError::Git(e)
    }
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

/// Pure entry point: build the review payload from a repo + base/target refs.
pub fn build_review(
    repo_path: &str,
    base: &str,
    target: &str,
) -> Result<ReviewData, EngineError> {
    let diff = gitdiff::diff_three_dot(repo_path, base, target)?;

    let mut cards = Vec::new();
    for file in &diff {
        // Symbols only matter for non-binary, non-deleted, supported-language files.
        let symbols = if file.is_binary
            || file.status == gitdiff::FileStatus::Deleted
            || file.status == gitdiff::FileStatus::Added
        {
            Vec::new()
        } else {
            match symbols::Lang::from_path(&file.new_path) {
                // None (unsupported) or parser ERROR (Ok(None)) => empty symbol set
                // => file-level fallback inside build_file_cards.
                Some(lang) => match symbols::extract(lang, &file.new_source)? {
                    Some(syms) => syms,
                    None => Vec::new(),
                },
                None => Vec::new(),
            }
        };
        cards::build_file_cards(file, &symbols, &mut cards);
    }

    Ok(ReviewData { cards })
}

#[cfg(test)]
mod tests;
