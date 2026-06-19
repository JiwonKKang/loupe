//! Stage-④/⑤/⑥ — the AI clustering → ordering → labelling pipeline.
//!
//! - **Stage-④ `cluster_step`** (seed correction, v2.1): sends the refined cluster cards
//!   (one per seed, Stage-③) with the seed-correction prompt + flattened M1 schema,
//!   parses [`ClusterResult`], verifies against the card-id whitelist
//!   (`verify::verify_clusters`): hallucinated ids ⇒ reject; omitted ids ⇒ absorbed into
//!   `unclustered` (no drop, §3.1).
//! - **Stage-⑤ `order_step`** (planning §4.5 / §6.2): takes the clustering result +
//!   relation hints and decides intra-cluster flow order (caller→callee, code-appearance
//!   order) + inter-cluster order. Output is the M1 flattened
//!   `{clusterOrder:[clusterId], orderedByCluster:[{clusterId, cardIds:[ordered]}]}`
//!   (no dynamic key maps). Verified by `verify::verify_order`: every id whitelisted AND
//!   each cluster's ordered ids are a *permutation* of its clustering members (no drop /
//!   add / cross-cluster move).
//! - **Small-PR branch `cluster_and_order_combined`** (planning §4.1): when the symbol
//!   count ≤ [`SMALL_PR_SYMBOLS`], clustering + ordering run in **one** Haiku call to cut
//!   latency. Big PRs keep the two calls separate.
//! - **Stage-⑥ `label_step`** (planning §6.2 / §8.4): ONE batched call producing
//!   title/summary per cluster (+ display-only merge/split suggestions, §6.3). The B1
//!   invariant (title/summary never empty) is guaranteed by `verify::verify_labels`, and
//!   the M4 token check flags identifiers the AI may have invented.
//!
//! Every AI call uses **Sonnet (Quality) via the `claude` CLI (`ai::cli::CliProvider`) +
//! temperature=0**. The *direct* Messages API returns HTTP 429 on Sonnet (only Haiku is
//! reachable that way), but the `claude` CLI routes through the Claude Code backend and
//! reaches Sonnet on the same setup-token — so the pipeline runs on Sonnet for分류·정렬·요약
//! 품질·결정성, with temp=0 reinforcing 재현성. On a verification failure a step **retries
//! once**; a second failure is `Err` so the orchestrator can fall back. (The provider is
//! injected — these functions only set `ModelTier::Quality`; the orchestrator wires the
//! concrete `CliProvider`.)

use super::prompts::{
    cluster_and_order_output_schema, cluster_output_schema, label_output_schema,
    order_output_schema, CLUSTER_AND_ORDER_SYSTEM, CLUSTER_SYSTEM, LABEL_SYSTEM, ORDER_SYSTEM,
};
use super::{CompletionRequest, LlmError, LlmProvider, ModelTier};
use crate::engine::clustercard::ClusterCardInput;
use crate::engine::model::ClusterKind;
use crate::engine::relations::RelationHints;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Symbol-count threshold below which a PR is "small". At or below this, clustering +
/// ordering merge into one AI call (planning §4.1 latency branch); above it, the two
/// stages run as separate calls so each output stays small and easy to verify/retry.
pub const SMALL_PR_SYMBOLS: usize = 12;

/// Max tokens for the clustering call. Output is small (id lists + a kind enum), so this
/// is generous headroom, not a real limit.
const CLUSTER_MAX_TOKENS: u32 = 4096;

/// Max tokens for the ordering call (id permutations only — small output).
const ORDER_MAX_TOKENS: u32 = 4096;

/// Max tokens for the combined cluster+order call (small PRs only, still small output).
const COMBINED_MAX_TOKENS: u32 = 4096;

/// Max tokens for the labelling call. Title + 1–3-sentence summary per cluster for *all*
/// clusters in one batch — larger than the id-only calls, but bounded by cluster count.
const LABEL_MAX_TOKENS: u32 = 8192;

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

/// One cluster's ordered card ids (Stage-⑤). The M1 flattening of `{clusterId: [cardId]}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderedCluster {
    pub cluster_id: String,
    pub card_ids: Vec<String>,
}

/// The ordering step's result (Stage-⑤): the inter-cluster order plus, per cluster, the
/// intra-cluster card-id order. `unclustered` is carried over from the clustering result
/// (it has no order). Verified by `verify::verify_order` against the clustering members.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResult {
    pub cluster_order: Vec<String>,
    pub ordered_by_cluster: Vec<OrderedCluster>,
    /// Carried from the clustering result (not part of the AI ordering output).
    #[serde(default)]
    pub unclustered: Vec<String>,
}

/// The combined small-PR output (planning §4.1): one call returns clustering (with
/// already-ordered `memberCardIds` + kind), `unclustered`, AND the inter-cluster order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAndOrderResult {
    pub clusters: Vec<AiCluster>,
    #[serde(default)]
    pub unclustered: Vec<String>,
    #[serde(default)]
    pub cluster_order: Vec<String>,
}

/// One cluster's AI-produced title + summary (Stage-⑥). B1: never empty after verify.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterLabel {
    pub cluster_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

/// A merge/split suggestion (Stage-⑥, display-only — never auto-applied, §6.3).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionOut {
    pub cluster_ids: Vec<String>,
    #[serde(default)]
    pub reason: String,
}

/// The labelling step's result (Stage-⑥, batched): a label per cluster + display-only
/// merge/split suggestions. `verify::verify_labels` guarantees B1 (non-empty title/
/// summary) and runs the M4 token check.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelResult {
    pub clusters: Vec<ClusterLabel>,
    #[serde(default)]
    pub merge_suggestions: Vec<SuggestionOut>,
    #[serde(default)]
    pub split_suggestions: Vec<SuggestionOut>,
}

/// Whether this PR takes the small-PR latency branch (planning §4.1): when the total
/// changed-symbol count is ≤ [`SMALL_PR_SYMBOLS`], clustering and ordering are done in one
/// AI call (`cluster_and_order_combined`); above it, the two calls stay separate.
pub fn is_small_pr(cards: &[ClusterCardInput]) -> bool {
    let symbols: usize = cards.iter().map(|c| c.changed_symbols.len()).sum();
    symbols <= SMALL_PR_SYMBOLS
}

/// Run the AI clustering (seed-correction) step.
///
/// Builds the whitelist from the input cards' card ids, sends one structured request, and
/// verifies the result. On verification failure, retries once; a second failure is `Err`.
/// The provider call uses the **Quality** tier (Sonnet) through `CliProvider`: 직접 API는
/// Sonnet에 HTTP 429를 반환하지만 claude CLI 경유로 Sonnet 호출이 가능하므로, 분류는 Sonnet
/// 품질·결정성으로 수행하고 재현성은 temperature=0으로 보강한다. Empty input ⇒ an empty result
/// (no network call).
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
            // Sonnet(Quality) via CliProvider: 직접 API는 Sonnet 429라 claude CLI 경유로
            // Sonnet 사용(분류 품질·결정성). 재현성은 temperature=0으로 보강.
            tier: ModelTier::Quality,
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

// ===========================================================================
// Stage-⑤ — ordering (intra-cluster flow order + inter-cluster order)
// ===========================================================================

/// Run the AI **ordering** step (Stage-⑤, planning §4.5 / §6.2).
///
/// Given the already-decided `clusters` and the relation `hints` (the per-pair
/// caller↔callee / type signals that tell the model the *code-appearance* order), decide
/// each cluster's intra-order (top-down flow: caller before callee) and the inter-cluster
/// order. Identifiers are card ids. The result is verified (`verify::verify_order`) to be
/// a *permutation* of the clustering members — the AI may not drop, add, or move ids
/// between clusters. Retries once on a verification failure; a second failure is `Err`.
///
/// Quality (Sonnet) tier via `CliProvider` + temperature=0 (재현성). An empty / single-id
/// clustering needs no model call — the order is already determined, so it short-circuits.
pub async fn order_step(
    provider: &dyn LlmProvider,
    clusters: &ClusterResult,
    hints: &RelationHints,
    whitelist: &BTreeSet<String>,
) -> Result<OrderResult, LlmError> {
    // Nothing to order if there are no clusters: the order is trivially the unclustered
    // bucket. (Also covers the "AI returned only unclustered" case — no network call.)
    if clusters.clusters.is_empty() {
        return Ok(OrderResult {
            cluster_order: Vec::new(),
            ordered_by_cluster: Vec::new(),
            unclustered: clusters.unclustered.clone(),
        });
    }

    let user = build_order_message(clusters, hints);
    let schema = order_output_schema();

    let mut last_err: Option<LlmError> = None;
    for _attempt in 0..2 {
        let req = CompletionRequest {
            // Quality(Sonnet) via CliProvider — 정렬도 Sonnet 품질·결정성.
            tier: ModelTier::Quality,
            system: ORDER_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: ORDER_MAX_TOKENS,
            json_schema: Some(schema.clone()),
            temperature: 0.0,
        };
        match order_attempt_once(provider, req, clusters, whitelist).await {
            Ok(mut result) => {
                // The AI ordering output has no `unclustered`; carry it from clustering.
                result.unclustered = clusters.unclustered.clone();
                return Ok(result);
            }
            Err(e) => last_err = Some(e),
        }
    }

    // Ordering is a *refinement*; clustering is the source of truth (planning §2.3). If the
    // model can't return a valid permutation after the retry (e.g. it quietly reshuffles
    // membership during ordering — observed on real PRs), degrade to the clustering's own
    // member order rather than failing the whole pipeline. This is NOT the Stage-⑩
    // algorithmic fallback — it keeps the verified clusters intact and only forfeits the
    // intra-cluster flow re-ordering, which is best-effort. Deterministic + whitelist-safe
    // by construction (the clustering result is already verified).
    match &last_err {
        // A transport error (Auth/Overloaded/Timeout) is infrastructural, not a bad order
        // — surface it so the caller can retry/back off rather than silently degrade.
        Some(LlmError::Parse(_)) | None => Ok(identity_order(clusters)),
        Some(other) => Err(other.clone()),
    }
}

/// Build an [`OrderResult`] that preserves each cluster's existing member order and the
/// clustering's cluster order (sorted by cluster id for determinism). Used as the
/// best-effort degradation when the AI ordering won't validate (clustering stays
/// authoritative). Always a valid permutation of the clustering members.
fn identity_order(clusters: &ClusterResult) -> OrderResult {
    let ordered_by_cluster: Vec<OrderedCluster> = clusters
        .clusters
        .iter()
        .map(|c| OrderedCluster {
            cluster_id: c.cluster_id.clone(),
            card_ids: c.member_card_ids.clone(),
        })
        .collect();
    let mut cluster_order: Vec<String> =
        clusters.clusters.iter().map(|c| c.cluster_id.clone()).collect();
    cluster_order.sort();
    OrderResult {
        cluster_order,
        ordered_by_cluster,
        unclustered: clusters.unclustered.clone(),
    }
}

/// One ordering request → parse → verify cycle.
async fn order_attempt_once(
    provider: &dyn LlmProvider,
    req: CompletionRequest,
    clusters: &ClusterResult,
    whitelist: &BTreeSet<String>,
) -> Result<OrderResult, LlmError> {
    let resp = provider.complete(req).await?;
    let parsed: OrderResult = serde_json::from_value(resp.json.clone())
        .map_err(|e| LlmError::Parse(format!("order output shape: {e}")))?;
    super::verify::verify_order(parsed, clusters, whitelist)
}

/// Build the ordering user-message: the clusters (id + members + kind) and the relation
/// hints. The relation hints carry the code-appearance order the prompt asks the model to
/// follow (a caller's callees listed in code order).
fn build_order_message(clusters: &ClusterResult, hints: &RelationHints) -> String {
    let cluster_json: Vec<_> = clusters
        .clusters
        .iter()
        .map(|c| {
            serde_json::json!({
                "clusterId": c.cluster_id,
                "memberCardIds": c.member_card_ids,
                "kind": c.kind,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "clusters": cluster_json,
        "relationHints": hints,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{\"clusters\":[]}".to_string())
}

// ===========================================================================
// Small-PR branch — clustering + ordering in one call (planning §4.1)
// ===========================================================================

/// **Small-PR path (planning §4.1):** cluster AND order in a single Sonnet call (via
/// `CliProvider`) to cut latency. Used when [`is_small_pr`] is true. Returns the same `(ClusterResult,
/// OrderResult)` pair the two-call path produces, so the orchestrator is branch-agnostic.
///
/// The combined output is verified in two passes against the same card-id whitelist:
/// the clustering half by `verify::verify_clusters` (hallucination reject + no-drop
/// absorption) and the ordering half by `verify::verify_order` (permutation parity).
/// Retries once on failure. Empty input ⇒ empty results, no call.
pub async fn cluster_and_order_combined(
    provider: &dyn LlmProvider,
    cards: &[ClusterCardInput],
) -> Result<(ClusterResult, OrderResult), LlmError> {
    if cards.is_empty() {
        return Ok((
            ClusterResult {
                clusters: Vec::new(),
                unclustered: Vec::new(),
            },
            OrderResult {
                cluster_order: Vec::new(),
                ordered_by_cluster: Vec::new(),
                unclustered: Vec::new(),
            },
        ));
    }

    let whitelist = whitelist_of(cards);
    let user = build_user_message(cards);
    let schema = cluster_and_order_output_schema();

    let mut last_err: Option<LlmError> = None;
    for _attempt in 0..2 {
        let req = CompletionRequest {
            // Quality(Sonnet) via CliProvider — 소형 PR 결합 호출도 Sonnet.
            tier: ModelTier::Quality,
            system: CLUSTER_AND_ORDER_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: COMBINED_MAX_TOKENS,
            json_schema: Some(schema.clone()),
            temperature: 0.0,
        };
        match combined_attempt_once(provider, req, &whitelist).await {
            Ok(pair) => return Ok(pair),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| LlmError::Parse("cluster_and_order_combined: no attempt".into())))
}

/// One combined request → parse → verify (both halves) cycle.
async fn combined_attempt_once(
    provider: &dyn LlmProvider,
    req: CompletionRequest,
    whitelist: &BTreeSet<String>,
) -> Result<(ClusterResult, OrderResult), LlmError> {
    let resp = provider.complete(req).await?;
    let parsed: ClusterAndOrderResult = serde_json::from_value(resp.json.clone())
        .map_err(|e| LlmError::Parse(format!("combined output shape: {e}")))?;

    // Clustering half: verify (reject hallucination, absorb omitted into unclustered).
    let clusters = super::verify::verify_clusters(
        ClusterResult {
            clusters: parsed.clusters.clone(),
            unclustered: parsed.unclustered.clone(),
        },
        whitelist,
    )?;

    // Ordering half: the combined `memberCardIds` are ALREADY in order (Part B of the
    // prompt). Build the ordered view from the *verified* clusters (so absorbed/deduped
    // ids stay consistent), then verify it as a permutation.
    let ordered_by_cluster: Vec<OrderedCluster> = clusters
        .clusters
        .iter()
        .map(|c| OrderedCluster {
            cluster_id: c.cluster_id.clone(),
            card_ids: c.member_card_ids.clone(),
        })
        .collect();
    let order = OrderResult {
        cluster_order: parsed.cluster_order.clone(),
        ordered_by_cluster,
        unclustered: clusters.unclustered.clone(),
    };
    let order = super::verify::verify_order(order, &clusters, whitelist)?;
    Ok((clusters, order))
}

// ===========================================================================
// Stage-⑥ — title / summary (one batched call) + merge/split suggestions
// ===========================================================================

/// The result of labelling: the verified labels (B1-safe) and, per cluster id, any
/// suspicious code-identifier tokens the M4 check flagged in the title/summary (empty
/// when the text is clean). The caller logs/acts on `suspicious` but is never blocked by
/// it (planning §8.3: free-text hallucination can't be perfectly caught).
#[derive(Debug, Clone, PartialEq)]
pub struct LabelOutcome {
    pub labels: LabelResult,
    pub suspicious: BTreeMap<String, Vec<String>>,
}

/// Run the AI **labelling** step (Stage-⑥, planning §6.2 / §8.4) — ONE batched call for
/// **all** clusters (never per-cluster N calls). For each cluster the model returns a
/// `[target] + [change action]` title and a 1–3-sentence summary; it may also emit
/// display-only merge/split suggestions (§6.3).
///
/// `clusters` pairs each cluster id with its changed symbols (drawn from the Stage-③ cards
/// after ordering). `allowed_names` is the bare-name whitelist for the M4 token check.
///
/// Guarantees:
///  - **B1**: every title/summary is non-empty after `verify::verify_labels` (empty AI
///    output ⇒ a fallback string).
///  - **M4**: code-identifier tokens in the text not present in the input are reported in
///    `LabelOutcome::suspicious` (a re-request hook; not fatal).
///
/// Quality (Sonnet) tier via `CliProvider` + temperature=0 (요약은 0~0.3 가능하나 일단 0
/// 통일). Retries once on a parse failure. Empty input ⇒ empty labels, no call.
pub async fn label_step(
    provider: &dyn LlmProvider,
    clusters: &[LabelInput],
    allowed_names: &BTreeSet<String>,
) -> Result<LabelOutcome, LlmError> {
    if clusters.is_empty() {
        return Ok(LabelOutcome {
            labels: LabelResult {
                clusters: Vec::new(),
                merge_suggestions: Vec::new(),
                split_suggestions: Vec::new(),
            },
            suspicious: BTreeMap::new(),
        });
    }

    let cluster_ids: BTreeSet<String> = clusters.iter().map(|c| c.cluster_id.clone()).collect();
    let user = build_label_message(clusters);
    let schema = label_output_schema();

    let mut last_err: Option<LlmError> = None;
    for _attempt in 0..2 {
        let req = CompletionRequest {
            // Quality(Sonnet) via CliProvider — 요약도 Sonnet 품질.
            tier: ModelTier::Quality,
            system: LABEL_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: LABEL_MAX_TOKENS,
            json_schema: Some(schema.clone()),
            // 요약도 일단 0 통일 (분류·정렬과 같은 재현성 기준).
            temperature: 0.0,
        };
        match provider.complete(req).await {
            Ok(resp) => match serde_json::from_value::<LabelResult>(resp.json.clone()) {
                Ok(parsed) => {
                    // B1 + M4: never-empty title/summary, flag suspicious identifiers,
                    // drop suggestions naming unknown clusters.
                    let (labels, suspicious) =
                        super::verify::verify_labels(parsed, &cluster_ids, allowed_names);
                    // Ensure EVERY input cluster has a label (B1 — the AI may skip one).
                    let labels = backfill_missing_labels(labels, clusters);
                    return Ok(LabelOutcome { labels, suspicious });
                }
                Err(e) => last_err = Some(LlmError::Parse(format!("label output shape: {e}"))),
            },
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| LlmError::Parse("label_step: no attempt ran".into())))
}

/// One cluster as the labelling call sees it: the cluster id, the algorithmic kind hint,
/// and its changed symbols (name + kind + change + short summary). No diff body.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelInput {
    pub cluster_id: String,
    pub kind: ClusterKind,
    pub changed_symbols: Vec<LabelSymbolIn>,
}

/// A changed symbol shown to the labelling call (display context only).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSymbolIn {
    pub name: String,
    pub kind: crate::engine::model::SymbolKind,
    pub change_type: crate::engine::model::ChangeType,
    pub summary: String,
}

/// Build the labelling user-message JSON: all clusters with their changed symbols.
fn build_label_message(clusters: &[LabelInput]) -> String {
    let payload = serde_json::json!({ "clusters": clusters });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{\"clusters\":[]}".to_string())
}

/// B1 backstop: if the AI omitted a label for an input cluster, synthesize a non-empty
/// fallback so every cluster always has a title/summary (the front-end calls
/// `summary.charAt(0)` with no guard).
fn backfill_missing_labels(mut labels: LabelResult, inputs: &[LabelInput]) -> LabelResult {
    let have: BTreeSet<&str> = labels.clusters.iter().map(|l| l.cluster_id.as_str()).collect();
    let missing: Vec<&LabelInput> = inputs
        .iter()
        .filter(|i| !have.contains(i.cluster_id.as_str()))
        .collect();
    for inp in missing {
        labels.clusters.push(ClusterLabel {
            cluster_id: inp.cluster_id.clone(),
            // B1 fallback 문자열도 한국어 (label 한국어 확정).
            title: "변경 사항".to_string(),
            summary: "이 클러스터의 변경 사항입니다.".to_string(),
        });
    }
    labels
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
