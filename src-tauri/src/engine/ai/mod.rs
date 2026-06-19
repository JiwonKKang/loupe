//! Stage-① — AI foundation.
//!
//! A provider-agnostic seam over the Anthropic Messages API. The orchestrator
//! (Stage ②+) consumes only the `LlmProvider` trait object, so tests inject a mock
//! and real calls go through `anthropic::{OAuthProvider, ApiKeyProvider}`.
//!
//! This module deliberately contains **no clustering/ordering/prompt logic** — those
//! land in `steps.rs` / `prompts.rs` / `verify.rs` in later stages. Stage-① is just
//! the transport + contract: request/response types, the tier→model map, the error
//! taxonomy, and structured-output (`output_config.format` json_schema) plumbing.
//!
//! ## B3 (v2-critique) — resolved in favour of OAuth
//! The setup-token uses OAuth Bearer auth (`authorization: Bearer <token>` +
//! `anthropic-version` + `anthropic-beta: oauth-2025-04-20`). The public Messages API
//! (`/v1/messages`) rejects Bearer auth *without* the `oauth-2025-04-20` beta header
//! (claude-api skill, verified); no `user-agent` is required. So `OAuthProvider` is
//! the **normal default path** and `ApiKeyProvider` (`x-api-key`, no beta header) is
//! the alternative. Both are first-class; only the auth headers differ (see
//! `anthropic.rs`).

pub mod anthropic;
pub mod cli;
pub mod prompts;
pub mod steps;
pub mod verify;

use async_trait::async_trait;
use serde_json::Value;

/// Which model to use for a given call. Quality = clustering/ordering/title/summary
/// (Sonnet) — the 분류·정렬·요약 work where quality matters (cost mitigated by caching);
/// Fast = only simple/cheap calls (Haiku). `model_for` resolves the concrete id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Fast,
    Quality,
}

/// A single structured completion request. `json_schema`, when present, is sent as
/// `output_config.format = { type: "json_schema", schema }` so the model is forced to
/// emit schema-conforming JSON (M1). No assistant prefill is ever used.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    /// Structured-output schema. `None` => free-form text (still parsed as JSON when
    /// the caller expects JSON, but without server-side schema enforcement).
    pub json_schema: Option<Value>,
    pub tier: ModelTier,
    /// Sampling temperature (0.0..=1.0). **0.0 for classification / structured output**
    /// (clustering 재현성) — same input ⇒ same clusters across runs. Summaries may use a
    /// little more (0.0..0.3) but 분류가 우선이므로 기본은 0.0.
    pub temperature: f32,
}

/// The parsed result of a completion. `json` is the model's structured output already
/// run through `serde_json::from_str` (never raw-string-matched). `stop_reason` is
/// surfaced so callers can detect `refusal` / `max_tokens` before trusting `json`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionResponse {
    pub json: Value,
    pub stop_reason: String,
}

/// Error taxonomy for an LLM call. Kept coarse on purpose: the orchestrator only needs
/// to decide retry vs. fallback, not to render provider internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// Transport / non-2xx that is not specifically auth/overload (message carries the
    /// status + body snippet).
    Http(String),
    /// 401/403 — bad or missing credentials.
    Auth,
    /// `stop_reason == "refusal"` — the model declined; content may be empty.
    Refusal,
    /// Response body present but not parseable into the expected JSON shape.
    Parse(String),
    /// 429 / 529 — rate limited or overloaded; caller may back off.
    Overloaded,
    /// The request timed out client-side.
    Timeout,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(s) => write!(f, "HTTP error: {s}"),
            LlmError::Auth => write!(f, "authentication failed (check token / api key)"),
            LlmError::Refusal => write!(f, "model refused to answer"),
            LlmError::Parse(s) => write!(f, "failed to parse model output: {s}"),
            LlmError::Overloaded => write!(f, "rate limited / overloaded"),
            LlmError::Timeout => write!(f, "request timed out"),
        }
    }
}

impl std::error::Error for LlmError {}

/// The provider seam. One structured, non-streaming `complete` is all Stage-① needs;
/// streaming (long summaries) is deferred to a later stage so the trait stays minimal.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Run one structured completion. Implementations must:
    ///  - send the tier's model id (`model_for`),
    ///  - attach the structured-output schema when `req.json_schema` is `Some`,
    ///  - map non-2xx to the right `LlmError` (401/403 => Auth, 429/529 => Overloaded),
    ///  - parse the assistant content with `serde_json::from_str` (never raw match),
    ///  - surface `stop_reason` (so `refusal`/`max_tokens` are detectable).
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Concrete model id for a tier. Date-suffixed ids are forbidden (alias only).
    fn model_for(&self, tier: ModelTier) -> &'static str;
}

#[cfg(test)]
mod tests;
