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
