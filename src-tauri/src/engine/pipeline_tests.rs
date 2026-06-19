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
use std::collections::HashMap;
use std::sync::Mutex;

/// A mock provider that returns canned JSON replies and counts calls.
///
/// Clustering / ordering / combined calls are served from a FIFO `replies` queue (one per
/// call, in order). **Labelling is content-addressed**: Stage-⑥ now labels each cluster on its
/// own call (`label_one`), bounded-concurrent, so the queue order can't map a reply to the
/// right cluster. Instead, a label call is detected by its system prompt and answered from the
/// `labels` map keyed by the cluster id in the request — concurrency-safe and deterministic.
struct SeqProvider {
    replies: Mutex<std::collections::VecDeque<Value>>,
    labels: HashMap<String, (String, String)>,
    calls: Mutex<usize>,
}

impl SeqProvider {
    fn new(replies: Vec<Value>) -> Self {
        Self::with_labels(replies, &[])
    }
    /// `labels`: `(clusterId, title, summary)` triples answered for per-cluster label calls.
    fn with_labels(replies: Vec<Value>, labels: &[(&str, &str, &str)]) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            labels: labels
                .iter()
                .map(|(id, t, s)| (id.to_string(), (t.to_string(), s.to_string())))
                .collect(),
            calls: Mutex::new(0),
        }
    }
    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

/// The first cluster id in a (single-cluster) label request body.
fn first_cluster_id(user: &str) -> String {
    serde_json::from_str::<Value>(user)
        .ok()
        .and_then(|v| v.get("clusters").and_then(|c| c.get(0)).cloned())
        .and_then(|c| c.get("clusterId").and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_default()
}

#[async_trait]
impl LlmProvider for SeqProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.calls.lock().unwrap() += 1;
        // The labelling system prompt is the only one asking for title/summary (mirrors the
        // discriminator the cache mock uses).
        let is_label = req.system.contains("title") && req.system.contains("summary");
        let json = if is_label {
            let id = first_cluster_id(&req.user);
            let (title, summary) = self
                .labels
                .get(&id)
                .cloned()
                .unwrap_or_else(|| ("변경".to_string(), "요약".to_string()));
            json!({
                "clusters": [ { "clusterId": id, "title": title, "summary": summary } ],
                "mergeSuggestions": [],
                "splitSuggestions": []
            })
        } else {
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Value::Null)
        };
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
        renamed_from: None,
        signature_change: None,
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
        deleted_symbols: vec![],
        rename_pairs: vec![],
        signature_changes: vec![],
    }
}

#[tokio::test]
async fn small_pr_takes_combined_branch_then_labels() {
    // 3 symbols ≤ SMALL_PR_SYMBOLS ⇒ combined cluster+order (1 call) + label (1 call).
    let cards = vec![card("seed-1", &[("a", "create"), ("b", "validate"), ("c", "save")])];
    let provider = SeqProvider::with_labels(
        vec![
            // combined: clusters (ordered members) + clusterOrder, one object.
            json!({
                "clusters": [ { "clusterId": "k1", "memberCardIds": ["a", "b", "c"], "kind": "flow" } ],
                "unclustered": [],
                "clusterOrder": ["k1"]
            }),
        ],
        // per-cluster labels (Stage-⑥ now calls once per cluster).
        &[("k1", "생성 흐름", "Creates, validates, saves.")],
    );

    let layout = run_cluster_pipeline(&provider, &cards, &RelationHints::default(), &())
        .await
        .expect("pipeline succeeds");

    assert_eq!(provider.call_count(), 2, "small PR ⇒ combined(1) + 1 cluster label(1) = 2 calls");
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
    let provider = SeqProvider::with_labels(
        vec![
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
        ],
        // per-cluster labels (one call each).
        &[
            ("k1", "로그인 흐름", "Handles login."),
            ("k2", "JWT 발급", "Issues JWT."),
        ],
    );

    let layout = run_cluster_pipeline(&provider, &cards, &RelationHints::default(), &())
        .await
        .expect("pipeline succeeds");

    assert_eq!(
        provider.call_count(),
        4,
        "big PR ⇒ cluster(1) + order(1) + per-cluster label(2) = 4 calls"
    );
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

/// A file seed-card: a singleton whose member card_id is a `<path>::__file` id and whose
/// name is the path (mirrors `clustercard::build_file_seed_cards`).
fn file_seed(seed_id: &str, path: &str, kind: SymbolKind) -> ClusterCardInput {
    ClusterCardInput {
        cluster_id: seed_id.to_string(),
        algorithmic_type_hint: ClusterKind::Infra,
        entrypoint_candidates: vec![],
        changed_symbols: vec![ChangedSymbolIn {
            card_id: format!("{path}::__file"),
            name: path.to_string(),
            kind,
            change_type: ChangeType::Added,
            summary: format!("Adds {path}."),
            renamed_from: None,
            signature_change: None,
        }],
        relation_hints: RelationHints::default(),
        contracts_changed: vec![],
        related_tests: vec![],
        deleted_symbols: vec![],
        rename_pairs: vec![],
        signature_changes: vec![],
    }
}

#[tokio::test]
async fn file_seeds_are_clustered_into_an_infra_topic_not_unclustered() {
    // Issue C: symbol-less infra/config files (CI, cargo, caddy) must enter the whitelist and
    // be groupable into an Infra cluster — NOT dropped to Unclustered. Here one code symbol +
    // three infra file seeds; the AI groups the 3 files into one infra cluster.
    let ci = ".github/workflows/ci.yml";
    let cargo = "Cargo.toml";
    let caddy = "Caddyfile";
    let cards = vec![
        card("seed-1", &[("a", "init_metrics")]),
        file_seed("file-seed-1", ci, SymbolKind::Config),
        file_seed("file-seed-2", cargo, SymbolKind::Config),
        file_seed("file-seed-3", caddy, SymbolKind::Config),
    ];
    // 4 symbols total ≤ SMALL_PR_SYMBOLS ⇒ combined branch (1 call) + label (1 call).
    let ci_id = format!("{ci}::__file");
    let cargo_id = format!("{cargo}::__file");
    let caddy_id = format!("{caddy}::__file");
    let provider = SeqProvider::with_labels(
        vec![json!({
            "clusters": [
                { "clusterId": "k1", "memberCardIds": ["a"], "kind": "flow" },
                { "clusterId": "k2", "memberCardIds": [ci_id, cargo_id, caddy_id], "kind": "infra" }
            ],
            "unclustered": [],
            "clusterOrder": ["k1", "k2"]
        })],
        &[
            ("k1", "메트릭 초기화", "메트릭 수집을 초기화한다."),
            ("k2", "HTTPS 인프라 구성", "Caddy 리버스 프록시로 HTTPS를 적용한다."),
        ],
    );

    let layout = run_cluster_pipeline(&provider, &cards, &RelationHints::default(), &())
        .await
        .expect("pipeline succeeds");

    // Nothing fell to Unclustered; the 3 infra files form one infra cluster.
    assert!(layout.unclustered.is_empty(), "infra files are clustered, not unclustered");
    let infra = layout.clusters.iter().find(|c| c.id == "k2").expect("infra cluster present");
    assert_eq!(infra.kind, ClusterKind::Infra);
    assert_eq!(infra.ordered_card_ids.len(), 3);
    assert!(infra.ordered_card_ids.contains(&ci_id));
    assert!(infra.ordered_card_ids.contains(&caddy_id));
    // Every card id (code + 3 files) appears exactly once in the flat order.
    assert_eq!(layout.ordered_card_ids.len(), 4);
}

#[tokio::test]
async fn empty_cards_produce_empty_layout_without_calls() {
    let provider = SeqProvider::new(vec![]);
    let layout = run_cluster_pipeline(&provider, &[], &RelationHints::default(), &())
        .await
        .expect("empty pipeline");
    assert!(layout.clusters.is_empty());
    assert!(layout.ordered_card_ids.is_empty());
    // Empty input goes through the small-PR combined path, which short-circuits, then
    // label_step also short-circuits ⇒ zero network calls.
    assert_eq!(provider.call_count(), 0);
}
