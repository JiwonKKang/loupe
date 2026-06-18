//! Stage-④→⑤→⑥ orchestration tests for [`super::run_cluster_pipeline`].
//!
//! A sequenced mock provider feeds canned AI replies so the whole pipeline
//! (cluster → order → label, or the small-PR combined call) is exercised without a
//! network or git. Assertions cover:
//!  - the **small-PR branch** clusters + orders in ONE call, then labels (2 calls total),
//!  - the **big-PR branch** runs three calls (cluster, order, label),
//!  - the assembled [`super::ClusterLayout`] has the inter-cluster order, each cluster's
//!    ordered members + filled title/summary/kind, the flat `ordered_card_ids`, and the
//!    unclustered bucket trailing last,
//!  - B1: every cluster ends with a non-empty title/summary.

use super::ai::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier};
use super::clustercard::{ChangedSymbolIn, ClusterCardInput};
use super::model::{ChangeType, ClusterKind, SymbolKind};
use super::relations::RelationHints;
use super::run_cluster_pipeline;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

/// A mock provider returning a sequence of canned JSON replies (one per call) and
/// counting calls.
struct SeqProvider {
    replies: Mutex<std::collections::VecDeque<Value>>,
    calls: Mutex<usize>,
}

impl SeqProvider {
    fn new(replies: Vec<Value>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            calls: Mutex::new(0),
        }
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl LlmProvider for SeqProvider {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.calls.lock().unwrap() += 1;
        let json = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Value::Null);
        Ok(CompletionResponse {
            json,
            stop_reason: "end_turn".into(),
        })
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

#[tokio::test]
async fn small_pr_takes_combined_branch_then_labels() {
    // 3 symbols ≤ SMALL_PR_SYMBOLS ⇒ combined cluster+order (1 call) + label (1 call).
    let cards = vec![card("seed-1", &[("a", "create"), ("b", "validate"), ("c", "save")])];
    let provider = SeqProvider::new(vec![
        // combined: clusters (ordered members) + clusterOrder, one object.
        json!({
            "clusters": [ { "clusterId": "k1", "memberCardIds": ["a", "b", "c"], "kind": "flow" } ],
            "unclustered": [],
            "clusterOrder": ["k1"]
        }),
        // labels.
        json!({
            "clusters": [ { "clusterId": "k1", "title": "생성 흐름", "summary": "Creates, validates, saves." } ],
            "mergeSuggestions": [],
            "splitSuggestions": []
        }),
    ]);

    let layout = run_cluster_pipeline(&provider, &cards, &RelationHints::default())
        .await
        .expect("pipeline succeeds");

    assert_eq!(provider.call_count(), 2, "small PR ⇒ combined(1) + label(1) = 2 calls");
    assert_eq!(layout.clusters.len(), 1);
    let c = &layout.clusters[0];
    assert_eq!(c.id, "k1");
    assert_eq!(c.kind, ClusterKind::Flow);
    assert_eq!(c.ordered_card_ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(c.title, "생성 흐름");
    assert_eq!(c.summary, "Creates, validates, saves.");
    assert_eq!(layout.cluster_order, vec!["k1".to_string()]);
    assert_eq!(layout.ordered_card_ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert!(layout.unclustered.is_empty());
}

#[tokio::test]
async fn big_pr_runs_three_calls_and_orders_across_clusters() {
    // 13 symbols > SMALL_PR_SYMBOLS ⇒ separate cluster + order + label (3 calls).
    let names: Vec<(&str, &str)> = vec![
        ("a", "handleLogin"), ("b", "validateToken"), ("c", "saveSession"),
        ("d", "kakaoFetch"), ("e", "mapProfile"), ("f", "upsertUser"),
        ("g", "issueJwt"), ("h", "parseClaims"), ("i", "refresh"),
        ("j", "logout"), ("k", "config"), ("l", "router"), ("m", "test_login"),
    ];
    let cards = vec![card("seed-1", &names)];
    let provider = SeqProvider::new(vec![
        // clustering: two clusters.
        json!({
            "clusters": [
                { "clusterId": "k1", "memberCardIds": ["a", "b", "c"], "kind": "flow" },
                { "clusterId": "k2", "memberCardIds": ["g", "h"], "kind": "domain-concept" }
            ],
            "unclustered": ["k", "l", "m", "d", "e", "f", "i", "j"]
        }),
        // ordering: reorder members + put k2 (domain) before k1 (flow) is allowed; here
        // we keep flow first. orderedByCluster must be a permutation of each cluster.
        json!({
            "clusterOrder": ["k1", "k2"],
            "orderedByCluster": [
                { "clusterId": "k1", "cardIds": ["a", "b", "c"] },
                { "clusterId": "k2", "cardIds": ["h", "g"] }
            ]
        }),
        // labels.
        json!({
            "clusters": [
                { "clusterId": "k1", "title": "로그인 흐름", "summary": "Handles login." },
                { "clusterId": "k2", "title": "JWT 발급", "summary": "Issues JWT." }
            ],
            "mergeSuggestions": [],
            "splitSuggestions": []
        }),
    ]);

    let layout = run_cluster_pipeline(&provider, &cards, &RelationHints::default())
        .await
        .expect("pipeline succeeds");

    assert_eq!(provider.call_count(), 3, "big PR ⇒ cluster + order + label = 3 calls");
    assert_eq!(layout.cluster_order, vec!["k1".to_string(), "k2".to_string()]);
    // k2's members came back reordered as h,g.
    let k2 = layout.clusters.iter().find(|c| c.id == "k2").unwrap();
    assert_eq!(k2.ordered_card_ids, vec!["h".to_string(), "g".to_string()]);
    assert_eq!(k2.kind, ClusterKind::DomainConcept);
    // Flat order = k1 members, then k2 members, then the unclustered bucket.
    let head: Vec<String> = vec!["a", "b", "c", "h", "g"].into_iter().map(String::from).collect();
    assert_eq!(&layout.ordered_card_ids[..5], &head[..]);
    // The 8 unclustered ids trail and appear in the flat order's tail.
    assert_eq!(layout.unclustered.len(), 8);
    assert_eq!(layout.ordered_card_ids.len(), 13, "every changed symbol appears once");
    // B1 holds for every cluster.
    for c in &layout.clusters {
        assert!(!c.title.trim().is_empty() && !c.summary.trim().is_empty());
    }
}

#[tokio::test]
async fn empty_cards_produce_empty_layout_without_calls() {
    let provider = SeqProvider::new(vec![]);
    let layout = run_cluster_pipeline(&provider, &[], &RelationHints::default())
        .await
        .expect("empty pipeline");
    assert!(layout.clusters.is_empty());
    assert!(layout.ordered_card_ids.is_empty());
    // Empty input goes through the small-PR combined path, which short-circuits, then
    // label_step also short-circuits ⇒ zero network calls.
    assert_eq!(provider.call_count(), 0);
}
