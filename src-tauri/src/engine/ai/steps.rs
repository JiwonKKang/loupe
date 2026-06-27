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
//! Every analysis AI call (cluster+order **and** labels) runs on **Haiku (Fast) via the
//! direct HTTP Messages API (`ai::anthropic::OAuthProvider`) + temperature=0**. The direct
//! API returns HTTP 429 on Sonnet (only Haiku is reachable that way), so HTTP = Haiku-only;
//! Haiku's cluster+order quality was verified comparable to Sonnet (multi-case, Sonnet-judged),
//! and HTTP is ~10x faster than routing Sonnet through the `claude` CLI (no subprocess /
//! cold-start). temp=0 keeps 재현성. On a verification failure a step **retries once**; a
//! second failure is `Err` so the orchestrator can fall back. (The provider is injected;
//! the orchestrator wires the concrete `OAuthProvider`. The `claude` CLI / Sonnet path now
//! survives ONLY for the agentic thread Q&A — `ai::cli::ask_agentic`.)

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
///
/// Retained for the original §4.1 latency-branch vocabulary and back-compat tests, but it is
/// **no longer the pipeline's combine/split switch** — that is now [`COMBINED_MAX_SYMBOLS`]
/// (the combined call is the default path for small *and* medium PRs; see
/// [`combined_fits`]). Kept as a documented label of "small" rather than dead-code.
pub const SMALL_PR_SYMBOLS: usize = 12;

/// **Historic** combine/split threshold (in changed symbols). When the pipeline still split ④⑤
/// for large PRs, a count above this forced the `cluster_step` + `order_step` path; at or below
/// it the combined call ran alone. The split has since been removed — the combined call
/// ([`cluster_and_order_combined`]) is now the *only* ④⑤ path for **every** PR size (its output
/// cap, [`COMBINED_MAX_TOKENS`], was raised to hold a large PR's structure), so this constant no
/// longer gates anything in `run_cluster_pipeline`. Retained as documented vocabulary and for the
/// `combined_fits` back-compat unit test; `#[allow(dead_code)]` since it has no lib-build caller.
#[allow(dead_code)]
pub const COMBINED_MAX_SYMBOLS: usize = 48;

/// Max tokens for the clustering call. Output is small (id lists + a kind enum), so this
/// is generous headroom, not a real limit. **No longer on the pipeline path** (the ④⑤ split was
/// removed; the combined call is the only ④⑤ path) — retained for the `cluster_step` tests.
#[allow(dead_code)]
const CLUSTER_MAX_TOKENS: u32 = 4096;

/// Max tokens for the ordering call (id permutations only — small output). **No longer on the
/// pipeline path** (see [`CLUSTER_MAX_TOKENS`]) — retained for the `order_step` tests.
#[allow(dead_code)]
const ORDER_MAX_TOKENS: u32 = 4096;

/// Max tokens for the combined cluster+order call (**콜1**, now the *only* ④⑤ path for every
/// PR size — the split was removed, so this single call must hold a large PR's whole structure).
///
/// Budget rationale (8192, not just 4096): the output is compact — per cluster one
/// `memberCardIds` id-list + a `clusterId` + a `kind`, plus the `clusterOrder` id list. That's
/// roughly "one short id string per changed symbol, twice" (once in a cluster, once in order) as
/// JSON, ~10–15 output tokens per id-with-punctuation. At 4096 a very large PR (≫48 symbols)
/// could *truncate* the JSON and trip the parse/verify path, which now has **no split fallback**
/// (a truncated combined call propagates `Err` → Stage-⑩ AI-0-call fallback). Doubling to 8192
/// covers ~300–500 symbols of structure with headroom for cluster wrappers/kinds and model
/// slack, while staying far inside a Sonnet context. Output is still id-lists+order (no diff
/// bodies), so the larger cap costs nothing on typical PRs — it only removes the truncation cliff
/// for the rare big one. (Conservative; raise further only with evidence of clean runs at higher
/// counts.)
const COMBINED_MAX_TOKENS: u32 = 8192;

/// Max tokens for the labelling call. Title + 1–3-sentence summary per cluster for *all*
/// clusters in one batch — larger than the id-only calls, but bounded by cluster count.
const LABEL_MAX_TOKENS: u32 = 8192;

/// B1 fallback title/summary substituted when a cluster's labelling call fails or omits it
/// (`label_one` / `verify_labels` / `backfill_missing_labels`). **Centralised** so the caching
/// layer can DETECT a fallen-back cluster (`mod::layout_is_cacheable`) and refuse to cache it —
/// a transient label failure must never freeze "변경 사항" into the SHA cache (it would then be
/// served on every re-open with no re-run). Changing these strings keeps detection in sync.
pub const FALLBACK_TITLE: &str = "변경 사항";
pub const FALLBACK_SUMMARY: &str = "이 클러스터의 변경 사항입니다.";

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
///
/// `card_summaries` is the per-card one-sentence Korean summary the same call emits alongside
/// the cluster label — one entry per member card. Optional (defaults empty): a missing or
/// failed label leaves a card's `ai_summary` as `None` (B1 is unaffected — `ai_summary` is the
/// separate AI-semantic field, not the statistical card `summary`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterLabel {
    pub cluster_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    /// Per-card summaries: `{cardId, summary}` for each member card of this cluster.
    #[serde(default)]
    pub card_summaries: Vec<CardSummary>,
}

/// One member card's AI one-sentence summary (Stage-⑥, part of `cardSummaries`).
/// `cardId` is whitelist-checked against the cluster's members; a hallucinated id is dropped.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardSummary {
    pub card_id: String,
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

/// Total changed-symbol count across the input cards (the clustering whitelist size). Both
/// the (legacy) "small PR" label and the (current) combine/split switch derive from it.
fn total_symbols(cards: &[ClusterCardInput]) -> usize {
    cards.iter().map(|c| c.changed_symbols.len()).sum()
}

/// Whether this PR is "small" by the original §4.1 latency label (≤ [`SMALL_PR_SYMBOLS`]).
///
/// **Note**: this is *no longer* the pipeline's combine/split switch — see [`combined_fits`].
/// The combined ④+⑤ call is now the default for small *and* medium PRs; this predicate is kept
/// for the §4.1 vocabulary and the existing back-compat unit test (`#[allow(dead_code)]` so the
/// retained-for-tests helper doesn't warn in the non-test lib build).
#[allow(dead_code)]
pub fn is_small_pr(cards: &[ClusterCardInput]) -> bool {
    total_symbols(cards) <= SMALL_PR_SYMBOLS
}

/// **Historic** combine/split predicate: whether ④+⑤ fit in ONE combined call (≤
/// [`COMBINED_MAX_SYMBOLS`]) rather than splitting into `cluster_step` + `order_step`.
///
/// The split was removed — the combined call is now the *only* ④⑤ path for every PR size — so the
/// orchestrator no longer consults this. Retained for the back-compat unit test;
/// `#[allow(dead_code)]` because it has no lib-build caller anymore.
#[allow(dead_code)]
pub fn combined_fits(cards: &[ClusterCardInput]) -> bool {
    total_symbols(cards) <= COMBINED_MAX_SYMBOLS
}

/// Run the AI clustering (seed-correction) step (the ④ half of the removed ④⑤ **split** path).
///
/// Builds the whitelist from the input cards' card ids, sends one structured request, and
/// verifies the result. On verification failure, retries once; a second failure is `Err`.
/// The provider call uses the **Quality** tier (Sonnet) through `CliProvider`: 직접 API는
/// Sonnet에 HTTP 429를 반환하지만 claude CLI 경유로 Sonnet 호출이 가능하므로, 분류는 Sonnet
/// 품질·결정성으로 수행하고 재현성은 temperature=0으로 보강한다. Empty input ⇒ an empty result
/// (no network call).
///
/// **No longer on the pipeline path** — `run_cluster_pipeline` always uses the combined
/// [`cluster_and_order_combined`] call now (the split was removed). Retained for the
/// `cluster_step` unit tests and the live `#[ignore]` repro; `#[allow(dead_code)]` so the
/// test-only function (and its prompt/schema/`attempt_once` deps) don't warn in the lib build.
#[allow(dead_code)]
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
///
/// **No longer on the pipeline path** — the ⑤ half of the removed ④⑤ split; ordering now comes
/// from the combined call's already-ordered `memberCardIds`. Retained for the `order_step` unit
/// tests; `#[allow(dead_code)]` so the test-only function (and its `identity_order` /
/// `order_attempt_once` / `build_order_message` / prompt+schema deps) don't warn in the lib build.
#[allow(dead_code)]
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
            // Fast(Haiku) via the direct HTTP Messages API (OAuthProvider): the whole
            // analysis pipeline runs on Haiku-HTTP — ~10x faster than the `claude` CLI
            // (no subprocess / cold-start), Sonnet 429s on the direct API, and Haiku's
            // cluster+order quality was verified comparable (multi-case, Sonnet-judged).
            tier: ModelTier::Fast,
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
/// **Fast (Haiku) tier** via `CliProvider` + temperature=0 (요약은 0~0.3 가능하나 일단 0
/// 통일). 라벨/요약은 클러스터링·정렬 결과를 *묘사*하는 가벼운 작업이라 Haiku로 빠르게 처리한다
/// (분류·정렬 = 콜1 combined는 Sonnet/Quality 유지). Retries once on a parse failure. Empty
/// input ⇒ empty labels, no call.
///
/// **No longer on the pipeline path.** Stage-⑥ now fans out one [`label_one`] **per cluster, all
/// concurrent** (the progressive-sidebar streaming variant — 1+N calls), so the orchestrator no
/// longer makes this single batched call. Retained for the `label_step` unit tests (B1/M4/backfill
/// /tier coverage); `#[allow(dead_code)]` so the test-only function doesn't warn in the lib build.
#[allow(dead_code)]
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
            // **콜2 = Haiku(Fast) via CliProvider.** 라벨/요약(제목·1~3문장 요약·카드별 요약)은
            // 클러스터 멤버십/정렬 결과를 *묘사*하는 가벼운 작업이라 Haiku로 빠르게 처리한다.
            // 분류·정렬(콜1 combined)은 구조를 *결정*하는 작업이라 Sonnet(Quality) 유지.
            // B1·M4·재현성(temp=0)·retry·backfill 로직은 tier와 무관하게 그대로다.
            tier: ModelTier::Fast,
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

/// Label **one** cluster (Stage-⑥, per-cluster streaming variant). Same system prompt /
/// schema / `verify_labels` guarantees as the batched [`label_step`], but scoped to a single
/// cluster so the pipeline can label clusters **concurrently** and reveal each in the loader
/// the moment its review finishes (the queue rail fills one cluster at a time).
///
/// Never fails: on a provider or parse error after one retry it returns the B1 fallback label
/// so a single flaky cluster can't sink the whole pipeline. Returns the verified (non-empty)
/// label plus this cluster's M4-suspicious tokens (empty when the text is clean).
///
/// **콜2..N+1 — the hot path.** `run_cluster_pipeline` fans this out **one call per cluster, all
/// concurrent** (no concurrency cap — cluster counts are small), emitting `Progress::Reviewed`
/// the moment each finishes so the sidebar fills progressively. The merge/split-suggestion fields
/// are intentionally empty here (a single-cluster call has no cross-cluster suggestion to make);
/// that matches the batched [`label_step`]'s output for a one-cluster input. The pipeline is fixed
/// at **1 + N** AI calls on a cache miss — combined ④⑤ (1) + one `label_one` per cluster (N).
pub async fn label_one(
    provider: &dyn LlmProvider,
    cluster: &LabelInput,
    allowed_names: &BTreeSet<String>,
) -> (ClusterLabel, Vec<String>) {
    let id = cluster.cluster_id.clone();
    let only = std::slice::from_ref(cluster);
    let cluster_ids: BTreeSet<String> = std::iter::once(id.clone()).collect();
    let user = build_label_message(only);
    let schema = label_output_schema();

    for _attempt in 0..2 {
        let req = CompletionRequest {
            // **콜2..N+1 = Haiku(Fast) via CliProvider.** 라벨/요약은 클러스터링·정렬 결과를
            // *묘사*하는 가벼운 작업이라 Haiku로 빠르게 처리한다 (분류·정렬 = 콜1 combined는
            // Sonnet/Quality 유지). B1·M4·재현성(temp=0)·retry·backfill 로직은 tier와 무관.
            tier: ModelTier::Fast,
            system: LABEL_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: LABEL_MAX_TOKENS,
            json_schema: Some(schema.clone()),
            temperature: 0.0,
        };
        if let Ok(resp) = provider.complete(req).await {
            if let Ok(parsed) = serde_json::from_value::<LabelResult>(resp.json.clone()) {
                let (labels, suspicious) =
                    super::verify::verify_labels(parsed, &cluster_ids, allowed_names);
                let labels = backfill_missing_labels(labels, only);
                if let Some(found) = labels.clusters.into_iter().find(|l| l.cluster_id == id) {
                    let bad = suspicious.get(&id).cloned().unwrap_or_default();
                    return (found, bad);
                }
            }
        }
    }

    // Both attempts failed — synthesize the B1 fallback (same string as the batched backfill).
    let empty = LabelResult {
        clusters: Vec::new(),
        merge_suggestions: Vec::new(),
        split_suggestions: Vec::new(),
    };
    let label = backfill_missing_labels(empty, only)
        .clusters
        .into_iter()
        .next()
        .expect("backfill adds the one missing cluster");
    (label, Vec::new())
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
///
/// Deliberately carries **no statistical `summary`** ("Updates X: +A −B lines"): that
/// belongs on the card, not the cluster. Feeding it here made the model copy symbol names
/// and line counts into the cluster `summary` (Issue A — cluster summary leaked per-card
/// detail). The label call gets only name/kind/change so the cluster summary stays an INTENT.
///
/// **Per-card AI summary**: `card_id` is the stable card id the model echoes back in
/// `cardSummaries`, and `snippet` is a *compressed* diff excerpt (the card's add/del lines,
/// capped — `SNIPPET_MAX_LINES`) giving the model the change-context it needs to write a one-
/// sentence Korean summary of *what that card's change does*. The snippet is per-card evidence
/// for `cardSummaries` only — the prompt forbids it from leaking into the cluster `summary`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelSymbolIn {
    /// Stable card id (echoed back in `cardSummaries[].cardId`).
    pub card_id: String,
    pub name: String,
    pub kind: crate::engine::model::SymbolKind,
    pub change_type: crate::engine::model::ChangeType,
    /// Compressed diff excerpt (the card's add/del lines, capped). Evidence for the per-card
    /// summary only — never serialized when empty (file-level cards may have none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snippet: String,
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
            // B1 fallback 문자열도 한국어 (label 한국어 확정). Centralised so the cache layer
            // can detect a fallen-back cluster and refuse to cache the layout.
            title: FALLBACK_TITLE.to_string(),
            summary: FALLBACK_SUMMARY.to_string(),
            // A skipped cluster has no per-card summaries; the affected cards' ai_summary
            // stays None (Optional — no B1 impact, the statistical card summary is intact).
            card_summaries: Vec::new(),
        });
    }
    labels
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;

// ===========================================================================
// Change-unit summaries (review stage) — per CARD, with whole-file + cluster context.
// ===========================================================================

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeUnitOut {
    title: String,
    why: String,
    tag: String,
    start_line: u32,
    end_line: u32,
    #[serde(default)]
    anchor_line: u32,
}
#[derive(serde::Deserialize)]
struct ChangeUnitsOut {
    summary: String,
    units: Vec<ChangeUnitOut>,
}

/// Review ONE file card into change-unit summaries (planning: 변경 단위 요약), giving the model
/// the file's FULL source + its cluster context so the summary reflects some codebase
/// understanding. **Fast (Haiku) via the injected provider** + temperature=0. Never errors: on a
/// transport/parse failure it returns `(None, [])` so a single flaky card can't sink the pipeline.
pub async fn review_card_units(
    provider: &dyn LlmProvider,
    tier: ModelTier,
    path: &str,
    diff_text: &str,
    full_source: &str,
    cluster_ctx: &str,
) -> (Option<String>, Vec<crate::engine::model::ChangeUnit>) {
    use super::prompts::{change_units_output_schema, CHANGE_UNITS_SYSTEM};
    let user = serde_json::json!({
        "path": path,
        "clusterContext": cluster_ctx,
        "fullSource": full_source,
        "diff": diff_text,
    })
    .to_string();
    let schema = change_units_output_schema();
    for _attempt in 0..2 {
        let req = CompletionRequest {
            tier,
            system: CHANGE_UNITS_SYSTEM.to_string(),
            user: user.clone(),
            max_tokens: 4096,
            json_schema: Some(schema.clone()),
            temperature: 0.0,
        };
        if let Ok(resp) = provider.complete(req).await {
            if let Ok(parsed) = serde_json::from_value::<ChangeUnitsOut>(resp.json.clone()) {
                let units = parsed
                    .units
                    .into_iter()
                    .map(|u| {
                        // Anchor falls back to the unit's start when the model omits it or returns
                        // an out-of-range line, so the bar always has a valid attach point.
                        let anchor = if u.anchor_line >= u.start_line && u.anchor_line <= u.end_line {
                            u.anchor_line
                        } else {
                            u.start_line
                        };
                        crate::engine::model::ChangeUnit {
                            title: u.title,
                            why: u.why,
                            tag: u.tag,
                            start_line: u.start_line,
                            end_line: u.end_line,
                            anchor_line: anchor,
                        }
                    })
                    .collect();
                return (Some(parsed.summary), units);
            }
        }
    }
    (None, Vec::new())
}
