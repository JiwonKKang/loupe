//! Stage-④ clustering-step tests.
//!
//! A `MockProvider` (records the request, returns a canned/sequenced reply) drives the
//! `cluster_step` flow without a network:
//!  - happy path: AI clusters parse + verify,
//!  - whitelist reject of a hallucinated id, then retry,
//!  - no-drop absorption end-to-end,
//!  - the M1 output schema is flat (no dynamic key map),
//!  - the seed-correction system prompt carries the v2.1 instructions.

use super::*;
use crate::engine::ai::prompts::{cluster_output_schema, CLUSTER_SYSTEM};
use crate::engine::ai::{
    CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier,
};
use crate::engine::clustercard::{ChangedSymbolIn, ClusterCardInput};
use crate::engine::model::{ChangeType, ClusterKind, SymbolKind};
use crate::engine::relations::RelationHints;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

/// A mock provider returning a sequence of canned replies (one per call). Records the
/// last request so we can assert the system prompt / schema / tier the step used.
struct SeqProvider {
    replies: Mutex<std::collections::VecDeque<Result<CompletionResponse, LlmError>>>,
    last: Mutex<Option<CompletionRequest>>,
    calls: Mutex<usize>,
}

impl SeqProvider {
    fn new(replies: Vec<Result<Value, LlmError>>) -> Self {
        let q = replies
            .into_iter()
            .map(|r| {
                r.map(|json| CompletionResponse {
                    json,
                    stop_reason: "end_turn".into(),
                })
            })
            .collect();
        Self {
            replies: Mutex::new(q),
            last: Mutex::new(None),
            calls: Mutex::new(0),
        }
    }
    fn one(json: Value) -> Self {
        Self::new(vec![Ok(json)])
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl LlmProvider for SeqProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.calls.lock().unwrap() += 1;
        *self.last.lock().unwrap() = Some(req);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(LlmError::Parse("no more canned replies".into())))
    }
    fn model_for(&self, tier: ModelTier) -> &'static str {
        match tier {
            ModelTier::Fast => "mock-fast",
            ModelTier::Quality => "mock-quality",
        }
    }
}

fn sym(id: &str, name: &str) -> ChangedSymbolIn {
    ChangedSymbolIn {
        card_id: id.to_string(),
        name: name.to_string(),
        kind: SymbolKind::Function,
        change_type: ChangeType::Modified,
        summary: format!("Updates {name}."),
    }
}

fn card(seed_id: &str, syms: &[(&str, &str)]) -> ClusterCardInput {
    ClusterCardInput {
        cluster_id: seed_id.to_string(),
        algorithmic_type_hint: ClusterKind::Flow,
        entrypoint_candidates: vec![],
        changed_symbols: syms.iter().map(|(id, n)| sym(id, n)).collect(),
        relation_hints: RelationHints::default(),
        contracts_changed: vec![],
        related_tests: vec![],
    }
}

// ---------------------------------------------------------------------------
// happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cluster_step_parses_and_verifies_valid_output() {
    let cards = vec![card("seed-1", &[("a", "create"), ("b", "validate")])];
    let provider = SeqProvider::one(json!({
        "clusters": [
            { "clusterId": "c1", "memberCardIds": ["a", "b"], "kind": "flow" }
        ],
        "unclustered": []
    }));
    let out = cluster_step(&provider, &cards).await.expect("valid output");
    assert_eq!(out.clusters.len(), 1);
    assert_eq!(out.clusters[0].member_card_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(out.clusters[0].kind, ClusterKind::Flow);
    assert_eq!(provider.call_count(), 1, "valid output ⇒ no retry");
}

#[tokio::test]
async fn cluster_step_sends_seed_correction_prompt_and_schema_on_fast_tier_temp0() {
    let cards = vec![card("seed-1", &[("a", "create")])];
    let provider = SeqProvider::one(json!({
        "clusters": [{ "clusterId": "c1", "memberCardIds": ["a"], "kind": "flow" }],
        "unclustered": []
    }));
    let _ = cluster_step(&provider, &cards).await.unwrap();
    let req = provider.last.lock().unwrap().clone().unwrap();
    assert_eq!(
        req.tier,
        ModelTier::Fast,
        "clustering uses the Fast (Haiku) tier: setup-token이 Sonnet 429라 Haiku 사용"
    );
    assert_eq!(
        req.temperature, 0.0,
        "clustering must run at temperature=0 for classification 재현성"
    );
    assert_eq!(req.system, CLUSTER_SYSTEM);
    // v2.1 seed-correction instructions present.
    assert!(req.system.contains("STARTING POINT"), "seed = starting point");
    assert!(req.system.to_lowercase().contains("merge"));
    assert!(req.system.to_lowercase().contains("split"));
    assert!(req.json_schema.is_some(), "structured output schema attached");
    // The user message carries the seeds, never raw diff.
    assert!(req.user.contains("\"seeds\""));
    assert!(req.user.contains("seed-1"));
}

// ---------------------------------------------------------------------------
// whitelist reject + retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cluster_step_rejects_hallucinated_id_then_retries_and_succeeds() {
    let cards = vec![card("seed-1", &[("a", "create")])];
    // First reply hallucinates "ghost"; second reply is clean.
    let provider = SeqProvider::new(vec![
        Ok(json!({
            "clusters": [{ "clusterId": "c1", "memberCardIds": ["a", "ghost"], "kind": "flow" }],
            "unclustered": []
        })),
        Ok(json!({
            "clusters": [{ "clusterId": "c1", "memberCardIds": ["a"], "kind": "flow" }],
            "unclustered": []
        })),
    ]);
    let out = cluster_step(&provider, &cards).await.expect("retry should recover");
    assert_eq!(out.clusters[0].member_card_ids, vec!["a".to_string()]);
    assert_eq!(provider.call_count(), 2, "one reject ⇒ exactly one retry");
}

#[tokio::test]
async fn cluster_step_errors_after_second_failure() {
    let cards = vec![card("seed-1", &[("a", "create")])];
    // Both replies hallucinate ⇒ Err after the retry (orchestrator falls back).
    let bad = json!({
        "clusters": [{ "clusterId": "c1", "memberCardIds": ["ghost"], "kind": "flow" }],
        "unclustered": []
    });
    let provider = SeqProvider::new(vec![Ok(bad.clone()), Ok(bad)]);
    let err = cluster_step(&provider, &cards).await.unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
    assert_eq!(provider.call_count(), 2, "exactly two attempts, then give up");
}

#[tokio::test]
async fn cluster_step_absorbs_omitted_ids_into_unclustered() {
    // Whitelist a,b,c; the AI clusters only a → b,c absorbed into unclustered (no drop).
    let cards = vec![card("seed-1", &[("a", "x"), ("b", "y"), ("c", "z")])];
    let provider = SeqProvider::one(json!({
        "clusters": [{ "clusterId": "c1", "memberCardIds": ["a"], "kind": "flow" }],
        "unclustered": []
    }));
    let out = cluster_step(&provider, &cards).await.unwrap();
    assert_eq!(out.unclustered, vec!["b".to_string(), "c".to_string()]);
}

#[tokio::test]
async fn cluster_step_empty_input_makes_no_call() {
    let provider = SeqProvider::new(vec![]);
    let out = cluster_step(&provider, &[]).await.unwrap();
    assert!(out.clusters.is_empty() && out.unclustered.is_empty());
    assert_eq!(provider.call_count(), 0, "empty input ⇒ no network call");
}

#[tokio::test]
async fn cluster_step_propagates_transport_error_without_retry_loop_hanging() {
    let cards = vec![card("seed-1", &[("a", "x")])];
    // Both attempts hit an Overloaded transport error ⇒ Err returned (no panic, no hang).
    let provider = SeqProvider::new(vec![
        Err(LlmError::Overloaded),
        Err(LlmError::Overloaded),
    ]);
    let err = cluster_step(&provider, &cards).await.unwrap_err();
    assert_eq!(err, LlmError::Overloaded);
    assert_eq!(provider.call_count(), 2);
}

// ---------------------------------------------------------------------------
// small-PR branch + whitelist helper + M1 schema
// ---------------------------------------------------------------------------

#[test]
fn is_small_pr_thresholds_on_symbol_count() {
    let small = vec![card("s", &[("a", "x"), ("b", "y")])];
    assert!(is_small_pr(&small));
    let big_syms: Vec<(&str, &str)> = (0..13)
        .map(|i| (["a","b","c","d","e","f","g","h","i","j","k","l","m"][i], "n"))
        .collect();
    let big = vec![card("s", &big_syms)];
    assert!(!is_small_pr(&big), "13 symbols > SMALL_PR_SYMBOLS(12)");
}

#[test]
fn whitelist_of_collects_all_card_ids() {
    let cards = vec![
        card("s1", &[("a", "x"), ("b", "y")]),
        card("s2", &[("c", "z")]),
    ];
    let wl = whitelist_of(&cards);
    assert_eq!(wl.len(), 3);
    assert!(wl.contains("a") && wl.contains("b") && wl.contains("c"));
}

#[test]
fn cluster_output_schema_is_m1_flat_no_dynamic_map() {
    let schema = cluster_output_schema();
    // Top object: additionalProperties false, requires clusters + unclustered.
    assert_eq!(schema["additionalProperties"], json!(false));
    let req = schema["required"].as_array().unwrap();
    assert!(req.contains(&json!("clusters")) && req.contains(&json!("unclustered")));
    // clusters is an ARRAY of fixed-key objects (flattened, not {clusterId: [...]}).
    assert_eq!(schema["properties"]["clusters"]["type"], "array");
    let item = &schema["properties"]["clusters"]["items"];
    assert_eq!(item["additionalProperties"], json!(false));
    assert_eq!(item["properties"]["memberCardIds"]["type"], "array");
    assert_eq!(item["properties"]["memberCardIds"]["items"]["type"], "string");
    // kind is a kebab-case enum matching ClusterKind serde.
    let kinds = item["properties"]["kind"]["enum"].as_array().unwrap();
    assert!(kinds.contains(&json!("flow")));
    assert!(kinds.contains(&json!("shared-foundation")));
    assert!(kinds.contains(&json!("domain-concept")));
}

#[test]
fn ai_cluster_kind_deserializes_kebab_case() {
    // The enum the AI emits ("domain-concept") must round-trip into ClusterKind.
    let c: AiCluster = serde_json::from_value(json!({
        "clusterId": "c1",
        "memberCardIds": ["a"],
        "kind": "domain-concept"
    }))
    .unwrap();
    assert_eq!(c.kind, ClusterKind::DomainConcept);
}

// ===========================================================================
// Stage-⑤ — order_step
// ===========================================================================

/// A clustering result with one cluster `c1` over the given member ids (kind flow).
fn clustering(members: &[&str]) -> ClusterResult {
    ClusterResult {
        clusters: vec![AiCluster {
            cluster_id: "c1".into(),
            member_card_ids: members.iter().map(|s| s.to_string()).collect(),
            kind: ClusterKind::Flow,
        }],
        unclustered: vec![],
    }
}

fn wl(ids: &[&str]) -> std::collections::BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn order_step_orders_within_cluster_on_fast_tier_temp0() {
    let clusters = clustering(&["a", "b", "c"]);
    // The AI reorders the cluster's members into flow order c,a,b and lists clusterOrder.
    let provider = SeqProvider::one(json!({
        "clusterOrder": ["c1"],
        "orderedByCluster": [ { "clusterId": "c1", "cardIds": ["c", "a", "b"] } ]
    }));
    let out = order_step(&provider, &clusters, &RelationHints::default(), &wl(&["a", "b", "c"]))
        .await
        .expect("valid ordering");
    assert_eq!(out.cluster_order, vec!["c1".to_string()]);
    assert_eq!(out.ordered_by_cluster.len(), 1);
    assert_eq!(
        out.ordered_by_cluster[0].card_ids,
        vec!["c".to_string(), "a".to_string(), "b".to_string()]
    );
    let req = provider.last.lock().unwrap().clone().unwrap();
    assert_eq!(req.tier, ModelTier::Fast, "ordering uses Haiku (Fast)");
    assert_eq!(req.temperature, 0.0, "ordering at temp=0 for 재현성");
    assert_eq!(req.system, crate::engine::ai::prompts::ORDER_SYSTEM);
    assert!(req.user.contains("\"clusters\""));
    assert!(req.user.contains("relationHints"));
    assert_eq!(provider.call_count(), 1);
}

#[tokio::test]
async fn order_step_empty_clustering_makes_no_call() {
    let empty = ClusterResult { clusters: vec![], unclustered: vec!["x".into()] };
    let provider = SeqProvider::new(vec![]);
    let out = order_step(&provider, &empty, &RelationHints::default(), &wl(&["x"]))
        .await
        .unwrap();
    assert!(out.ordered_by_cluster.is_empty());
    assert_eq!(out.unclustered, vec!["x".to_string()], "unclustered carried through");
    assert_eq!(provider.call_count(), 0, "no clusters ⇒ no network call");
}

#[tokio::test]
async fn order_step_rejects_dropped_member_then_retries() {
    let clusters = clustering(&["a", "b", "c"]);
    // First reply drops "c" (not a permutation) → reject; second is a clean permutation.
    let provider = SeqProvider::new(vec![
        Ok(json!({
            "clusterOrder": ["c1"],
            "orderedByCluster": [ { "clusterId": "c1", "cardIds": ["a", "b"] } ]
        })),
        Ok(json!({
            "clusterOrder": ["c1"],
            "orderedByCluster": [ { "clusterId": "c1", "cardIds": ["a", "b", "c"] } ]
        })),
    ]);
    let out = order_step(&provider, &clusters, &RelationHints::default(), &wl(&["a", "b", "c"]))
        .await
        .expect("retry recovers");
    assert_eq!(out.ordered_by_cluster[0].card_ids.len(), 3);
    assert_eq!(provider.call_count(), 2, "one reject ⇒ exactly one retry");
}

#[tokio::test]
async fn order_step_degrades_to_clustering_order_when_ai_wont_validate() {
    // Both replies hallucinate (a Parse-class verify failure) ⇒ after the retry, ordering
    // degrades to the clustering's own member order (clustering is source of truth), never
    // leaking the bad id. The pipeline stays alive (no Err).
    let clusters = clustering(&["a", "b"]);
    let bad = json!({
        "clusterOrder": ["c1"],
        "orderedByCluster": [ { "clusterId": "c1", "cardIds": ["a", "ghost"] } ]
    });
    let provider = SeqProvider::new(vec![Ok(bad.clone()), Ok(bad)]);
    let out = order_step(&provider, &clusters, &RelationHints::default(), &wl(&["a", "b"]))
        .await
        .expect("degrades, does not error");
    assert_eq!(provider.call_count(), 2, "tried twice, then degraded");
    // Identity order = the clustering's own member order; no hallucinated id present.
    assert_eq!(
        out.ordered_by_cluster[0].card_ids,
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(out.cluster_order, vec!["c1".to_string()]);
}

#[tokio::test]
async fn order_step_propagates_transport_error_without_degrading() {
    // An Overloaded transport error is infrastructural, not a bad order — it must surface
    // (so the caller can back off), NOT silently degrade to identity order.
    let clusters = clustering(&["a", "b"]);
    let provider = SeqProvider::new(vec![
        Err(LlmError::Overloaded),
        Err(LlmError::Overloaded),
    ]);
    let err = order_step(&provider, &clusters, &RelationHints::default(), &wl(&["a", "b"]))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::Overloaded);
    assert_eq!(provider.call_count(), 2);
}

// ===========================================================================
// Small-PR branch — cluster_and_order_combined
// ===========================================================================

#[tokio::test]
async fn cluster_and_order_combined_clusters_and_orders_in_one_call() {
    let cards = vec![card("seed-1", &[("a", "create"), ("b", "validate")])];
    // One reply carries clustering (ordered members + kind) AND the inter-cluster order.
    let provider = SeqProvider::one(json!({
        "clusters": [ { "clusterId": "c1", "memberCardIds": ["a", "b"], "kind": "flow" } ],
        "unclustered": [],
        "clusterOrder": ["c1"]
    }));
    let (clustering, ordering) = cluster_and_order_combined(&provider, &cards)
        .await
        .expect("combined output");
    assert_eq!(clustering.clusters.len(), 1);
    assert_eq!(ordering.cluster_order, vec!["c1".to_string()]);
    // The combined memberCardIds ARE the order (Part B of the prompt).
    assert_eq!(
        ordering.ordered_by_cluster[0].card_ids,
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(provider.call_count(), 1, "small PR ⇒ ONE merged call");
    let req = provider.last.lock().unwrap().clone().unwrap();
    assert_eq!(req.system, crate::engine::ai::prompts::CLUSTER_AND_ORDER_SYSTEM);
}

#[tokio::test]
async fn cluster_and_order_combined_absorbs_omitted_ids() {
    // Whitelist a,b,c; combined call clusters only a → b,c absorbed (no drop), still valid.
    let cards = vec![card("seed-1", &[("a", "x"), ("b", "y"), ("c", "z")])];
    let provider = SeqProvider::one(json!({
        "clusters": [ { "clusterId": "c1", "memberCardIds": ["a"], "kind": "flow" } ],
        "unclustered": [],
        "clusterOrder": ["c1"]
    }));
    let (clustering, ordering) = cluster_and_order_combined(&provider, &cards).await.unwrap();
    assert_eq!(clustering.unclustered, vec!["b".to_string(), "c".to_string()]);
    assert_eq!(ordering.unclustered, vec!["b".to_string(), "c".to_string()]);
}

#[tokio::test]
async fn cluster_and_order_combined_empty_input_makes_no_call() {
    let provider = SeqProvider::new(vec![]);
    let (clustering, ordering) = cluster_and_order_combined(&provider, &[]).await.unwrap();
    assert!(clustering.clusters.is_empty() && ordering.ordered_by_cluster.is_empty());
    assert_eq!(provider.call_count(), 0);
}

// ===========================================================================
// Stage-⑥ — label_step (batch, B1, M4)
// ===========================================================================

fn label_input(cluster_id: &str, syms: &[(&str, &str)]) -> LabelInput {
    LabelInput {
        cluster_id: cluster_id.to_string(),
        kind: ClusterKind::Flow,
        changed_symbols: syms
            .iter()
            .map(|(n, s)| LabelSymbolIn {
                name: n.to_string(),
                kind: SymbolKind::Function,
                change_type: ChangeType::Modified,
                summary: s.to_string(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn label_step_batches_all_clusters_in_one_call() {
    let inputs = vec![
        label_input("c1", &[("createOrder", "creates an order")]),
        label_input("c2", &[("applyCoupon", "applies a coupon")]),
    ];
    let provider = SeqProvider::one(json!({
        "clusters": [
            { "clusterId": "c1", "title": "주문 생성", "summary": "Creates an order." },
            { "clusterId": "c2", "title": "쿠폰 적용", "summary": "Applies a coupon." }
        ],
        "mergeSuggestions": [],
        "splitSuggestions": []
    }));
    let allowed = wl(&["createOrder", "applyCoupon"]);
    let out = label_step(&provider, &inputs, &allowed).await.expect("labels");
    assert_eq!(out.labels.clusters.len(), 2);
    assert_eq!(provider.call_count(), 1, "ALL clusters in ONE batched call (§8.4)");
    let req = provider.last.lock().unwrap().clone().unwrap();
    assert_eq!(req.tier, ModelTier::Fast);
    assert_eq!(req.temperature, 0.0);
    assert_eq!(req.system, crate::engine::ai::prompts::LABEL_SYSTEM);
    assert!(req.user.contains("\"clusters\""));
}

#[tokio::test]
async fn label_step_b1_substitutes_fallback_for_empty_title_summary() {
    let inputs = vec![label_input("c1", &[("createOrder", "creates")])];
    // The AI returns an empty title and empty summary — B1 must substitute fallbacks.
    let provider = SeqProvider::one(json!({
        "clusters": [ { "clusterId": "c1", "title": "", "summary": "" } ],
        "mergeSuggestions": [],
        "splitSuggestions": []
    }));
    let out = label_step(&provider, &inputs, &wl(&["createOrder"])).await.unwrap();
    assert!(!out.labels.clusters[0].title.trim().is_empty(), "B1: title never empty");
    assert!(!out.labels.clusters[0].summary.trim().is_empty(), "B1: summary never empty");
}

#[tokio::test]
async fn label_step_backfills_a_cluster_the_ai_skipped() {
    // Two clusters in, but the AI only labelled c1 → c2 must get a fallback (B1).
    let inputs = vec![
        label_input("c1", &[("a", "x")]),
        label_input("c2", &[("b", "y")]),
    ];
    let provider = SeqProvider::one(json!({
        "clusters": [ { "clusterId": "c1", "title": "T", "summary": "S." } ],
        "mergeSuggestions": [],
        "splitSuggestions": []
    }));
    let out = label_step(&provider, &inputs, &wl(&["a", "b"])).await.unwrap();
    assert_eq!(out.labels.clusters.len(), 2, "every input cluster gets a label");
    let c2 = out.labels.clusters.iter().find(|l| l.cluster_id == "c2").unwrap();
    assert!(!c2.title.trim().is_empty() && !c2.summary.trim().is_empty());
}

#[tokio::test]
async fn label_step_flags_hallucinated_identifier_in_summary() {
    let inputs = vec![label_input("c1", &[("createOrder", "creates")])];
    // The summary mentions PaymentGateway — not in the input → flagged via M4 token check.
    let provider = SeqProvider::one(json!({
        "clusters": [
            { "clusterId": "c1", "title": "Order", "summary": "Calls PaymentGateway.charge() too." }
        ],
        "mergeSuggestions": [],
        "splitSuggestions": []
    }));
    let out = label_step(&provider, &inputs, &wl(&["createOrder"])).await.unwrap();
    let flagged = out.suspicious.get("c1").expect("c1 should have suspicious tokens");
    assert!(flagged.iter().any(|t| t.contains("PaymentGateway")), "got {flagged:?}");
    // The label is still produced (free-text hallucination is not fatal, §8.3).
    assert!(!out.labels.clusters[0].summary.is_empty());
}

#[tokio::test]
async fn label_step_drops_suggestion_naming_unknown_cluster() {
    let inputs = vec![label_input("c1", &[("a", "x")])];
    let provider = SeqProvider::one(json!({
        "clusters": [ { "clusterId": "c1", "title": "T", "summary": "S." } ],
        "mergeSuggestions": [ { "clusterIds": ["c1", "ghost"], "reason": "looks related" } ],
        "splitSuggestions": []
    }));
    let out = label_step(&provider, &inputs, &wl(&["a"])).await.unwrap();
    assert!(out.labels.merge_suggestions.is_empty(), "suggestion naming unknown cluster dropped");
}

#[tokio::test]
async fn label_step_empty_input_makes_no_call() {
    let provider = SeqProvider::new(vec![]);
    let out = label_step(&provider, &[], &wl(&[])).await.unwrap();
    assert!(out.labels.clusters.is_empty());
    assert_eq!(provider.call_count(), 0);
}

// ===========================================================================
// M1 schemas for stages ⑤/⑥
// ===========================================================================

#[test]
fn order_output_schema_is_m1_flat_no_dynamic_map() {
    use crate::engine::ai::prompts::order_output_schema;
    let schema = order_output_schema();
    assert_eq!(schema["additionalProperties"], json!(false));
    // orderedByCluster is an ARRAY of fixed-key objects (NOT {clusterId: [...]}).
    assert_eq!(schema["properties"]["orderedByCluster"]["type"], "array");
    let item = &schema["properties"]["orderedByCluster"]["items"];
    assert_eq!(item["additionalProperties"], json!(false));
    assert_eq!(item["properties"]["cardIds"]["type"], "array");
    assert_eq!(schema["properties"]["clusterOrder"]["type"], "array");
}

#[test]
fn label_output_schema_is_m1_flat() {
    use crate::engine::ai::prompts::label_output_schema;
    let schema = label_output_schema();
    assert_eq!(schema["additionalProperties"], json!(false));
    let item = &schema["properties"]["clusters"]["items"];
    assert_eq!(item["additionalProperties"], json!(false));
    let req = item["required"].as_array().unwrap();
    assert!(req.contains(&json!("title")) && req.contains(&json!("summary")));
    assert_eq!(schema["properties"]["mergeSuggestions"]["type"], "array");
    assert_eq!(schema["properties"]["splitSuggestions"]["type"], "array");
}

#[test]
fn combined_output_schema_carries_clusters_and_cluster_order() {
    use crate::engine::ai::prompts::cluster_and_order_output_schema;
    let schema = cluster_and_order_output_schema();
    let req = schema["required"].as_array().unwrap();
    assert!(req.contains(&json!("clusters")));
    assert!(req.contains(&json!("unclustered")));
    assert!(req.contains(&json!("clusterOrder")));
}
