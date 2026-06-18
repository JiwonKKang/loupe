//! Local-branch enumeration for the onboarding range picker.
//!
//! Knows only git2 — returns a plain intermediate (`Branches`) that `lib.rs`
//! reshapes into the serde payload sent to the front-end. Kept separate from
//! `gitdiff` so opening the repo for "what can I compare?" is independent from
//! the heavier diff/tree-sitter path.

use super::EngineError;
use git2::{BranchType, Repository};

/// Result of scanning a repo's local branches.
pub struct Branches {
    /// All local branch shorthands, sorted with common bases first (see `rank`).
    pub branches: Vec<String>,
    /// HEAD shorthand, or `None` when detached.
    pub current: Option<String>,
    /// First of main/master/develop that actually exists — a sensible base default.
    pub default: Option<String>,
}

/// Preference order so the dropdown surfaces the usual base branches up top;
/// everything else falls back to case-insensitive alphabetical.
const PREFERRED: [&str; 3] = ["main", "master", "develop"];

/// Open `repo_path` and enumerate its local branches.
pub fn list_branches(repo_path: &str) -> Result<Branches, EngineError> {
    let repo = Repository::open(repo_path)?;

    let mut names: Vec<String> = Vec::new();
    for entry in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = entry?;
        if let Some(name) = branch.name()? {
            names.push(name.to_string());
        }
    }

    // current = HEAD shorthand; None when detached or unborn (no commits yet).
    let current = repo
        .head()
        .ok()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand().map(|s| s.to_string()));

    // default = first preferred base that exists in this repo.
    let default = PREFERRED
        .iter()
        .find(|p| names.iter().any(|n| n == *p))
        .map(|p| p.to_string());

    sort_branches(&mut names, current.as_deref());

    Ok(Branches {
        branches: names,
        current,
        default,
    })
}

/// Sort so the current branch is first, then main/master/develop, then the rest
/// case-insensitively. Stable + total so the dropdown order is deterministic.
fn sort_branches(names: &mut [String], current: Option<&str>) {
    names.sort_by(|a, b| {
        rank(a, current)
            .cmp(&rank(b, current))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
    });
}

/// Lower rank sorts earlier: current (0), preferred base (1..=3), other (100).
fn rank(name: &str, current: Option<&str>) -> u8 {
    if Some(name) == current {
        return 0;
    }
    PREFERRED
        .iter()
        .position(|p| *p == name)
        .map(|i| i as u8 + 1)
        .unwrap_or(100)
}
