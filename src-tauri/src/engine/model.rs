//! Serializable IPC types — the single source of truth for the front-end contract.
//!
//! The front-end (`App.jsx`, `ReviewScreen.jsx`, `SummaryScreen.jsx`) expects the
//! short keys `n` / `t` / `c` on each line and the long keys on each card. These
//! types are the *only* thing the engine sends across IPC: `verdicts`, `threads`,
//! `spineItems`, `unresolved` are all derived on the front-end from `cards`.

use serde::Serialize;

/// The whole IPC payload. The front-end reads `data.cards` only.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ReviewData {
    pub cards: Vec<ReviewCard>,
}

/// One review card = one symbol's contiguous change, or one file-level fallback.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ReviewCard {
    /// Globally unique. Used by the front-end as React key / verdict key / thread key.
    /// Stable across re-ordering (M3): never embeds the card's position.
    pub id: String,
    /// Group label. ProgressSpine merges consecutive equal `chapter` into one section.
    /// Stage 1 = file basename.
    pub chapter: String,
    /// Display title. e.g. "Session.Validate" (file-level / unsupported => basename).
    pub symbol: String,
    /// repo-relative path. `path.split('/').pop()` is the spine file label.
    pub path: String,
    /// Always "pending" (seed only — the front-end recomputes from verdicts).
    pub status: String,
    /// One sentence, starts with a capital. **Never empty** (B1 invariant): the
    /// front-end calls `summary.charAt(0)` with no guard.
    pub summary: String,
    pub lines: Vec<ReviewLine>,
}

/// One rendered diff line.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ReviewLine {
    /// Gutter number. Monotonic *new*-coordinate number for every kind, including
    /// `del` (B2): a `del` line carries the gutter number of the preceding context
    /// line so the gutter never jumps backwards. `old_lineno` is never exposed.
    pub n: u32,
    /// "add" | "del" | "ctx" — exactly these three (front-end contract).
    pub t: &'static str,
    /// Raw code text (leading tabs preserved; no trailing newline; no +/- marker).
    pub c: String,
}

pub const T_ADD: &str = "add";
pub const T_DEL: &str = "del";
pub const T_CTX: &str = "ctx";
