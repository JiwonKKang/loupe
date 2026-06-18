//! Stage-④ — AI clustering = **seed correction** (v2.1 core).
//!
//! `cluster_step` is the one AI call this stage owns: it sends the refined cluster cards
//! (one per seed, Stage-③) to the model with the v2.1 seed-correction system prompt and a
//! flattened M1 schema, parses the structured output into [`ClusterResult`], then verifies
//! it against the input card-id whitelist (`verify::verify_clusters`):
//!  - hallucinated card ids ⇒ reject,
//!  - omitted card ids ⇒ auto-absorbed into `unclustered` (no drop, §3.1).
//!
//! On a verification failure the step **retries once** (one fresh request); a second
//! failure returns `Err` so the orchestrator (later stage) can fall back. Ordering (AI2),
//! labelling (AI3), and the small-PR (≤12 symbols) 1+2 merge are *not* this stage — a
//! small-PR branch point is reserved here but only clusters (no ordering yet).

use super::prompts::{cluster_output_schema, CLUSTER_SYSTEM};
use super::{CompletionRequest, LlmError, LlmProvider, ModelTier};
use crate::engine::clustercard::ClusterCardInput;
use crate::engine::model::ClusterKind;
use serde::Deserialize;
use std::collections::BTreeSet;

/// Symbol-count threshold below which a PR is "small". Reserved for the later AI1+AI2
/// single-call merge (planning §4.1 latency branch). This stage only clusters, so the
/// flag currently just records the branch; ordering is Stage-⑤.
pub const SMALL_PR_SYMBOLS: usize = 12;

/// Max tokens for the clustering call. Output is small (id lists + a kind enum), so this
/// is generous headroom, not a real limit.
const CLUSTER_MAX_TOKENS: u32 = 4096;

/// One cluster the AI produced: a (volatile) label, the member card ids, and a kind.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCluster {
    pub cluster_id: String,
    pub member_card_ids: Vec<String>,
    pub kind: ClusterKind,
}

/// The clustering step's result (post-parse, pre/post-verify). `unclustered` holds card
/// ids the AI left out *and* (after `verify_clusters`) any omitted whitelist ids absorbed
/// to keep every change visible.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterResult {
    pub clusters: Vec<AiCluster>,
    #[serde(default)]
    pub unclustered: Vec<String>,
}

/// Whether this PR takes the small-PR latency branch (planning §4.1). The actual
/// AI1+AI2 merge lands in Stage-⑤; today this only informs the caller.
pub fn is_small_pr(cards: &[ClusterCardInput]) -> bool {
    let symbols: usize = cards.iter().map(|c| c.changed_symbols.len()).sum();
    symbols <= SMALL_PR_SYMBOLS
}

/// Run the AI clustering (seed-correction) step.
///
/// Builds the whitelist from the input cards' card ids, sends one structured request, and
/// verifies the result. On verification failure, retries once; a second failure is `Err`.
/// The provider call uses the **Fast** tier (Haiku): setup-token이 Sonnet에 HTTP 429
/// (rate_limit)를 반환해 Sonnet을 못 쓰므로 Haiku 사용; 분류 재현성은 temperature=0으로
/// 확보한다 (구조화 출력/분류는 temp=0이 정석). Empty input ⇒ an empty result (no network
/// call).
pub async fn cluster_step(
    provider: &dyn LlmProvider,
    cards: &[ClusterCardInput],
) -> Result<ClusterResult, LlmError> {
    if cards.is_empty() {
        return Ok(ClusterResult {
            clusters: Vec::new(),
            unclustered: Vec::new(),
        });
    }

    let whitelist = whitelist_of(cards);
    let user = build_user_message(cards);
    let schema = cluster_output_schema();

    // First attempt, then one retry on a verification/parse failure (planning §8.3).
    let mut last_err: Option<LlmError> = None;
    for _attempt in 0..2 {
        let req = CompletionRequest {
            // setup-token이 Sonnet 429라 Haiku(Fast) 사용; 분류 재현성은 temperature=0으로 확보.
            tier: ModelTier::Fast,
            system: CLUSTER_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: CLUSTER_MAX_TOKENS,
            json_schema: Some(schema.clone()),
            // 분류·구조화 출력은 temp=0이 정석(재현성).
            temperature: 0.0,
        };
        match attempt_once(provider, req, &whitelist).await {
            Ok(result) => return Ok(result),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| LlmError::Parse("cluster_step: no attempt ran".into())))
}

/// One request → parse → verify cycle.
async fn attempt_once(
    provider: &dyn LlmProvider,
    req: CompletionRequest,
    whitelist: &BTreeSet<String>,
) -> Result<ClusterResult, LlmError> {
    let resp = provider.complete(req).await?;
    let parsed: ClusterResult = serde_json::from_value(resp.json.clone())
        .map_err(|e| LlmError::Parse(format!("cluster output shape: {e}")))?;
    super::verify::verify_clusters(parsed, whitelist)
}

/// The card-id whitelist = every changed-symbol card id across all input cards (M4).
pub fn whitelist_of(cards: &[ClusterCardInput]) -> BTreeSet<String> {
    cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.clone()))
        .collect()
}

/// Build the user-message JSON: the seed cards plus an explicit note that the seeds are
/// proposals. The cards already serialize as camelCase (Stage-③). Kept compact (the diff
/// is never here — input-size defence, Stage-③).
fn build_user_message(cards: &[ClusterCardInput]) -> String {
    // The clustering call sees the seeds (as cluster cards) under a `seeds` key, matching
    // the system prompt's vocabulary ("You are given `seeds`...").
    let payload = serde_json::json!({ "seeds": cards });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{\"seeds\":[]}".to_string())
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
