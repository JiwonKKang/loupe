//! Streaming progress for the analysis pipeline (the front-end `AnalyzeScreen`).
//!
//! On a cache miss the AI pipeline runs for minutes; the front-end shows a live screen that
//! mirrors the real stages — static prep, per-cluster review (the queue rail fills as each
//! cluster's AI review finishes), then the final ordering pass. The engine reports those
//! milestones through a [`ProgressSink`] so it stays decoupled from Tauri: tests pass the
//! no-op `()` sink, and `lib.rs` passes a sink that re-emits each event over the
//! `analyze://progress` Tauri channel.
//!
//! These events are a *cosmetic* side-channel. The authoritative result is still the
//! `ReviewData` the command returns; dropping every event changes nothing but the loader.

use serde::Serialize;

/// A pipeline milestone. Serialized as the `analyze://progress` event payload; the
/// front-end discriminates on `kind`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Progress {
    /// Deterministic prep finished — the diff was parsed into `files` changed files.
    Static { files: usize },
    /// Cluster membership decided (after ④): the clusters the model will review, each with
    /// its member symbol names. Titles are not known yet — they arrive per [`Progress::Reviewed`].
    Clusters { clusters: Vec<ProgressCluster> },
    /// One cluster finished its AI review (⑥ title/summary) — the rail reveals it now,
    /// flipping its provisional label to the real `chapter` title.
    Reviewed { id: String, chapter: String },
    /// Every cluster reviewed; the final ordering/assembly pass is running.
    Final,
}

/// One cluster as the loader's queue rail shows it before its title is known.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressCluster {
    pub id: String,
    /// Provisional chapter label shown until the AI title arrives.
    pub chapter: String,
    /// Member symbol display names (what fills the rail under the chapter).
    pub cards: Vec<String>,
}

/// Where pipeline milestones go. Decoupled from Tauri so the engine stays unit-testable.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: Progress);
}

/// No-op sink — tests and any caller that doesn't surface progress pass `&()`.
impl ProgressSink for () {
    fn emit(&self, _event: Progress) {}
}
