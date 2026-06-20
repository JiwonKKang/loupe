//! Serializable IPC types — the single source of truth for the front-end contract.
//!
//! The front-end (`App.jsx`, `ReviewScreen.jsx`, `SummaryScreen.jsx`) expects the
//! short keys `n` / `t` / `c` on each line and the long keys on each card. These
//! types are the *only* thing the engine sends across IPC: `verdicts`, `threads`,
//! `spineItems`, `unresolved` are all derived on the front-end from `cards`.
//!
//! Stage-2 (AI clustering/ordering) *extends* this contract without breaking it:
//! `ReviewData` gains cluster fields, `ReviewCard` gains optional symbol metadata.
//! All new fields default to empty so the Stage-1 `build_review` can keep filling
//! `cards` exactly as before and leave the rest at `Default`. Per-struct
//! `#[serde(rename_all = "camelCase")]` is applied only to the *new* structs and to
//! `ReviewCard` (m1) so the new keys serialize as camelCase while the pre-existing
//! `n`/`t`/`c` (ReviewLine) and `cards` (ReviewData) keys are untouched.

use serde::{Deserialize, Serialize};

/// The whole IPC payload. Stage-1 fills `cards` (the diff-render contract, frozen);
/// Stage-2 overlays the cluster two-tier (`clusters` / `cluster_order` /
/// `ordered_card_ids` / …). Every Stage-2 field defaults to empty so a Stage-1-only
/// `build_review` returns a valid payload (`Default`-filled) without touching `cards`.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewData {
    /// [frozen] Stage-1 output exactly as before. The front-end reads `data.cards`.
    pub cards: Vec<ReviewCard>,
    /// [new] AI cluster metadata. Empty when only Stage-1 ran / on fallback.
    pub clusters: Vec<Cluster>,
    /// [new] Inter-cluster order; trailing `"__unclustered"` bucket last (§3.1).
    pub cluster_order: Vec<String>,
    /// [new] Full flatten order (front-end index source of truth).
    pub ordered_card_ids: Vec<String>,
    /// [new] Card ids that fell into the Unclustered bucket (§3.1 — always shown).
    pub unclustered: Vec<String>,
    /// [new] Head SHA — cache key / "same head == same order" marker.
    pub head_sha: String,
    /// [new] Base (merge-base) SHA — the 3-dot diff depends on it; cache key.
    pub base_sha: String,
    /// [new] Where the analysis stands (streaming / fallback signalling).
    pub analysis: AnalysisState,
    /// [new] §6.3 — display-only merge suggestions (never auto-applied).
    pub merge_suggestions: Vec<Suggestion>,
    /// [new] §6.3 — display-only split suggestions.
    pub split_suggestions: Vec<Suggestion>,
}

/// One review card = one symbol's contiguous change, or one file-level fallback.
///
/// The original Stage-1 fields (`id`/`chapter`/`symbol`/`path`/`status`/`summary`/
/// `lines`) keep their exact serialized keys. The Stage-2 fields are camelCased and
/// carry defaults so they are inert until Stage-2 fills them.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCard {
    /// Globally unique. Used by the front-end as React key / verdict key / thread key.
    /// Stable across re-ordering (M3) AND independent of `qualified` (m4): derived from
    /// `name + start_row`, so a later `qualified` normalization never changes a card id.
    pub id: String,
    /// Group label. ProgressSpine merges consecutive equal `chapter` into one section.
    /// Stage 1 = file basename. **Kept** for App.jsx/ProgressSpine compatibility (m1).
    pub chapter: String,
    /// Display title. e.g. "Session.Validate" (file-level / unsupported => basename).
    pub symbol: String,
    /// repo-relative path. `path.split('/').pop()` is the spine file label.
    pub path: String,
    /// Always "pending" (seed only — the front-end recomputes from verdicts).
    pub status: String,
    /// One sentence, starts with a capital. **Never empty** (B1 invariant): the
    /// front-end calls `summary.charAt(0)` with no guard. Stays statistical (Stage-1);
    /// the AI semantic summary lives in `ai_summary` so B1 is never at the AI's mercy.
    pub summary: String,
    pub lines: Vec<ReviewLine>,

    // ---- Stage-2 additions (optional / defaulted; inert until the AI layer fills them)
    /// [new] Owning cluster id. `None` when only Stage-1 ran.
    pub cluster_id: Option<String>,
    /// [new] §2.2 classification (default `Function`).
    pub kind: SymbolKind,
    /// [new] Qualified display name, e.g. "OrderService.calculatePrice". Today this is
    /// filled equal to `symbol`/`name`; real qualification is deferred to Stage ②. It is
    /// *not* part of the id key, so changing it later does not disturb caches (m4).
    pub qualified: String,
    /// [new] Added | Modified | Deleted (default `Modified`).
    pub change_type: ChangeType,
    /// [new] AI semantic summary, separate from the statistical `summary` (B1 split).
    pub ai_summary: Option<String>,
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

// ---------------------------------------------------------------------------
// Stage-2 cluster two-tier (v2-design §3)
// ---------------------------------------------------------------------------

/// AI cluster metadata: id -> title/summary/kind + the ordered card ids inside it.
/// `Deserialize` so the ⑦ cache can round-trip a stored `ClusterLayout` (which holds these).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    /// "cluster-1" (volatile label — excluded from the cache card-hash).
    pub id: String,
    /// AI title (B1-style: never empty; fallback "Changes").
    pub title: String,
    /// AI summary, 1–3 sentences.
    pub summary: String,
    /// Final classification (AI).
    pub kind: ClusterKind,
    /// Algorithmic hint fed to the AI as input — kept for debug/display.
    pub type_hint: ClusterKind,
    /// Intra-cluster order (AI `orderedSymbols` resolved to card ids).
    pub ordered_card_ids: Vec<String>,
}

/// Cluster classification (v2-design §3 / planning §4.3). `Deserialize` is needed because
/// the AI clustering step (Stage-④) parses this kebab-case enum out of the model output.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ClusterKind {
    #[default]
    Flow,
    Contract,
    DomainConcept,
    SharedFoundation,
    Infra,
}

/// §6.3 merge/split suggestion — display only, never auto-applied.
/// `Deserialize` so the ⑦ cache can round-trip a stored `ClusterLayout` (which holds these).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// "merge" | "split".
    pub kind: String,
    pub cluster_ids: Vec<String>,
    pub reason: String,
}

/// Where the Stage-2 analysis stands (streaming / fallback signalling).
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisState {
    /// Stage-1 only — no AI overlay yet (default).
    #[default]
    Idle,
    /// AI pipeline finished and verified.
    Done,
    /// AI failed/over budget — algorithmic fallback applied.
    Fallback,
    /// Some clusters resolved, still streaming.
    Partial,
}

/// §2.2 symbol classification for a review card.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    #[default]
    Function,
    Method,
    Class,
    Type,
    Interface,
    Enum,
    Dto,
    Test,
    Migration,
    Config,
    File,
}

/// How a symbol changed in this PR.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Added,
    #[default]
    Modified,
    Deleted,
}
