//! ⑦ — the **cache-determinism** + **whole-input invalidation** integration tests (the core
//! of this stage).
//!
//! A `CountingProvider` makes valid AI replies *and counts calls*. The proof of caching is
//! the call count: a second `run_cluster_pipeline_cached` over the same cards must do
//! **ZERO** AI calls (the whole-input row + the layout are cached) and return a
//! **byte-identical** layout — i.e. the cache pins the AI's residual non-determinism (§8.1).
//!
//! Clustering is a *global* decision (the model must see all seed cards together to merge
//! them), so the cache grain is the **whole clustering input**, not a single seed: the
//! pipeline runs once over all cards and the result is keyed by the set hash of every card's
//! content hash. Any seed content change invalidates that one row and re-runs the whole
//! pipeline; an unchanged input (even under a moved head) is a zero-call hit.
//!
//! The provider is *intentionally non-deterministic* in its labels (a per-call counter
//! leaks into the title) so that, without the cache, two runs would differ. With the cache,
//! the second run never calls the provider, so the title is frozen to the first run's value.

use super::ai::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier};
use super::cache::Cache;
use super::clustercard::{ChangedSymbolIn, ClusterCardInput};
use super::model::{ChangeType, ClusterKind, SymbolKind};
use super::relations::RelationHints;
use super::ClusterLayout;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A provider that builds a valid reply from the card ids it sees in the request and
/// **counts calls**. Its labels embed a monotonic counter, so identical inputs produce
/// *different* titles across runs — only the cache can make two runs byte-identical.
struct CountingProvider {
    calls: AtomicUsize,
    /// A salt mixed into titles so re-running the (uncached) pipeline yields a new title,
    /// proving the byte-identical 2nd run comes from the cache, not from determinism.
    salt: Mutex<usize>,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            salt: Mutex::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

/// Pull every string that looks like a card id out of the request user message. We parse
/// the JSON and collect `cardId` fields (clustering input) or `memberCardIds` (label/order
/// input). Returns them sorted+deduped (deterministic ordering for the reply).
fn card_ids_in(user: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(user).unwrap_or(Value::Null);
    let mut ids = std::collections::BTreeSet::new();
    collect_card_ids(&v, &mut ids);
    ids.into_iter().collect()
}

fn collect_card_ids(v: &Value, out: &mut std::collections::BTreeSet<String>) {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get("cardId") {
                out.insert(s.clone());
            }
            for (k, val) in map {
                if k == "memberCardIds" {
                    if let Value::Array(a) = val {
                        for e in a {
                            if let Value::String(s) = e {
                                out.insert(s.clone());
                            }
                        }
                    }
                }
                collect_card_ids(val, out);
            }
        }
        Value::Array(a) => {
            for e in a {
                collect_card_ids(e, out);
            }
        }
        _ => {}
    }
}

#[async_trait]
impl LlmProvider for CountingProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let ids = card_ids_in(&req.user);

        // Distinguish the call shape by which schema key the system prompt expects. The
        // labelling system prompt is the only one that asks for `title`/`summary`.
        let is_label = req.system.contains("title") && req.system.contains("summary");
        let is_combined = req.system.contains("both GROUP and ORDER");

        let json = if is_label {
            // One label per cluster id seen. Embed the salt so an UNCACHED re-run differs.
            let salt = {
                let mut s = self.salt.lock().unwrap();
                *s += 1;
                *s
            };
            let cluster_ids = label_cluster_ids(&req.user);
            // Per-cluster member card ids (for the cardSummaries reply). Stage-⑥ is now ONE
            // batched call covering every cluster, so we answer all clusters the request carried.
            let members = label_member_ids_by_cluster(&req.user);
            let clusters: Vec<Value> = cluster_ids
                .iter()
                .map(|cid| {
                    let card_summaries: Vec<Value> = members
                        .get(cid)
                        .map(|ids| {
                            ids.iter()
                                .map(|id| json!({ "cardId": id, "summary": format!("{id} 요약") }))
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({
                        "clusterId": cid,
                        "title": format!("클러스터 {cid} 변경 #{salt}"),
                        "summary": "변경 사항을 요약합니다.",
                        "cardSummaries": card_summaries
                    })
                })
                .collect();
            json!({ "clusters": clusters, "mergeSuggestions": [], "splitSuggestions": [] })
        } else if is_combined {
            // Small-PR combined: one cluster holding all ids, already ordered.
            json!({
                "clusters": [ { "clusterId": "k1", "memberCardIds": ids, "kind": "flow" } ],
                "unclustered": [],
                "clusterOrder": ["k1"]
            })
        } else {
            // Plain clustering (big-PR path, not exercised by single-seed runs).
            json!({
                "clusters": [ { "clusterId": "k1", "memberCardIds": ids, "kind": "flow" } ],
                "unclustered": []
            })
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

/// The cluster ids present in a labelling request (so the label reply covers them all).
fn label_cluster_ids(user: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(user).unwrap_or(Value::Null);
    let mut ids = Vec::new();
    if let Some(Value::Array(clusters)) = v.get("clusters") {
        for c in clusters {
            if let Some(Value::String(s)) = c.get("clusterId") {
                ids.push(s.clone());
            }
        }
    }
    ids
}

/// Member card ids per cluster in a labelling request (`clusters[].changedSymbols[].cardId`),
/// so the mock can emit one `cardSummaries` entry per member — mirroring the real model.
fn label_member_ids_by_cluster(user: &str) -> std::collections::HashMap<String, Vec<String>> {
    let v: Value = serde_json::from_str(user).unwrap_or(Value::Null);
    let mut out = std::collections::HashMap::new();
    if let Some(Value::Array(clusters)) = v.get("clusters") {
        for c in clusters {
            let Some(Value::String(cid)) = c.get("clusterId") else { continue };
            let mut ids = Vec::new();
            if let Some(Value::Array(syms)) = c.get("changedSymbols") {
                for s in syms {
                    if let Some(Value::String(id)) = s.get("cardId") {
                        ids.push(id.clone());
                    }
                }
            }
            out.insert(cid.clone(), ids);
        }
    }
    out
}

fn sym(id: &str, name: &str, summary: &str) -> ChangedSymbolIn {
    ChangedSymbolIn {
        card_id: id.to_string(),
        name: name.to_string(),
        kind: SymbolKind::Function,
        change_type: ChangeType::Modified,
        summary: summary.to_string(),
        snippet: format!("+// {name}: {summary}"),
        renamed_from: None,
        signature_change: None,
    }
}

fn card(seed_id: &str, syms: Vec<ChangedSymbolIn>) -> ClusterCardInput {
    ClusterCardInput {
        cluster_id: seed_id.to_string(),
        algorithmic_type_hint: ClusterKind::Flow,
        entrypoint_candidates: vec![],
        changed_symbols: syms,
        relation_hints: RelationHints::default(),
        contracts_changed: vec![],
        related_tests: vec![],
        deleted_symbols: vec![],
        rename_pairs: vec![],
        signature_changes: vec![],
    }
}

/// Drive the cached per-seed pipeline (the function the IPC layer will call under the hood).
async fn run(
    provider: &CountingProvider,
    cache: &Cache,
    merge_base: &str,
    cards: &[ClusterCardInput],
) -> ClusterLayout {
    super::run_cluster_pipeline_cached(provider, cache, "/repo", merge_base, cards, &RelationHints::default(), &())
        .await
        .expect("cached pipeline succeeds")
}

// ===========================================================================
// ★ Cache determinism — the core test.
// ===========================================================================

#[tokio::test]
async fn second_run_makes_zero_ai_calls_and_is_byte_identical() {
    let cache = Cache::open_in_memory().unwrap();
    let provider = CountingProvider::new();
    let cards = vec![
        card("seed-1", vec![sym("a", "create", "creates"), sym("b", "save", "saves")]),
        card("seed-2", vec![sym("c", "validate", "validates")]),
    ];

    // 1st run: AI is called once over the whole input (small PR ⇒ combined + label = 2 calls).
    let first = run(&provider, &cache, "mb1", &cards).await;
    let calls_after_first = provider.calls();
    assert!(calls_after_first > 0, "first run must hit the provider");

    // 2nd run, SAME cards, SAME merge-base: the whole-input row is a hit ⇒ ZERO calls.
    let second = run(&provider, &cache, "mb1", &cards).await;
    assert_eq!(
        provider.calls(),
        calls_after_first,
        "second run must make ZERO additional AI calls (cache hit)"
    );

    // Byte-identical: the cache pins the residual non-determinism (§8.1). Note the provider
    // salts titles, so without the cache the second run would differ.
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap(),
        "cached result is byte-identical to the first run"
    );
}

#[tokio::test]
async fn two_seeds_can_merge_into_one_cluster() {
    // ★ Regression for the "everything fragments" bug. The cached pipeline must run the AI
    // over ALL seed cards at once, so the model is free to merge two seeds into a single
    // cluster. The old per-seed design made this structurally impossible (each seed ran in
    // isolation ⇒ N seeds ⇒ ≥ N fragments). The mock provider, given both seeds together,
    // returns ONE cluster holding both ids; the result must therefore be a single cluster
    // with both cards — not two fragmented ones.
    let cache = Cache::open_in_memory().unwrap();
    let provider = CountingProvider::new();
    let cards = vec![
        card("seed-1", vec![sym("a", "create", "creates")]),
        card("seed-2", vec![sym("b", "validate", "validates")]),
    ];

    let layout = run(&provider, &cache, "mb1", &cards).await;

    // Both cards present exactly once.
    assert_eq!(layout.ordered_card_ids.len(), 2, "both seeds' cards survive");
    assert!(layout.ordered_card_ids.contains(&"a".to_string()));
    assert!(layout.ordered_card_ids.contains(&"b".to_string()));
    // ONE cluster — the AI saw both seeds together and merged them. (Pre-fix: 2 fragments.)
    assert_eq!(layout.clusters.len(), 1, "two seeds merge into one cluster");
    assert_eq!(
        layout.clusters[0].ordered_card_ids.len(),
        2,
        "the single cluster holds both seeds' cards"
    );
    assert_eq!(layout.cluster_order.len(), 1, "cluster_order lists the one cluster");
    assert!(layout.unclustered.is_empty(), "nothing fell to unclustered");
}

#[tokio::test]
async fn changing_any_seed_reruns_the_whole_pipeline() {
    // Whole-input grain: clustering is a *global* decision, so any seed content change
    // invalidates the single whole-input cache row and the pipeline re-runs over all cards
    // (the price of correctness — see `run_cluster_pipeline_cached`). This replaces the old
    // per-seed "partial invalidation": that design re-ran the AI on one isolated seed, which
    // is exactly what made cross-seed merges impossible.
    let cache = Cache::open_in_memory().unwrap();
    let provider = CountingProvider::new();
    let mut cards = vec![
        card("seed-1", vec![sym("a", "create", "creates")]),
        card("seed-2", vec![sym("b", "validate", "validates")]),
    ];

    // Warm the whole-input row.
    let _ = run(&provider, &cache, "mb1", &cards).await;
    let after_warm = provider.calls();
    assert!(after_warm > 0, "first run hits the provider");

    // Re-run with the SAME cards ⇒ whole-input hit ⇒ ZERO new calls.
    let before_unchanged = provider.calls();
    let _ = run(&provider, &cache, "mb1", &cards).await;
    assert_eq!(
        provider.calls(),
        before_unchanged,
        "unchanged input ⇒ whole-input cache hit ⇒ no new AI calls"
    );

    // Change ONE seed's content ⇒ the whole-input set hash changes ⇒ the whole pipeline
    // re-runs (combined + label = 2 calls for this small PR).
    cards[1].changed_symbols[0].summary = "now validates differently".to_string();
    let before_changed = provider.calls();
    let _ = run(&provider, &cache, "mb1", &cards).await;
    let new_calls = provider.calls() - before_changed;
    assert_eq!(new_calls, 2, "a changed seed re-runs the whole pipeline");
}

#[tokio::test]
async fn unchanged_content_under_a_new_head_is_a_hit() {
    // Head-independent reuse across a push: the whole-input row is keyed by (repo,
    // merge_base_sha, set_hash) with NO head. The same merge-base + same card contents ⇒
    // reuse, even though the head moved (the set hash is content-derived). Here we model
    // "same content seen again" — zero new calls.
    let cache = Cache::open_in_memory().unwrap();
    let provider = CountingProvider::new();
    let cards = vec![card("seed-1", vec![sym("a", "create", "creates")])];

    let first = run(&provider, &cache, "mb-shared", &cards).await;
    let after_first = provider.calls();
    // Run again with the SAME merge-base + content (a head change that didn't touch the seed).
    let second = run(&provider, &cache, "mb-shared", &cards).await;
    assert_eq!(provider.calls(), after_first, "unchanged seed ⇒ no new AI call");
    assert_eq!(first, second);
}

#[tokio::test]
async fn different_merge_base_is_a_miss() {
    // M3: the whole-input key is (repo, merge_base_sha, set_hash). A different merge-base
    // ⇒ a miss ⇒ the AI re-runs (the 3-dot base changed, so the diff content may differ).
    let cache = Cache::open_in_memory().unwrap();
    let provider = CountingProvider::new();
    let cards = vec![card("seed-1", vec![sym("a", "create", "creates")])];

    let _ = run(&provider, &cache, "mbA", &cards).await;
    let after_a = provider.calls();
    let _ = run(&provider, &cache, "mbB", &cards).await; // different merge-base
    assert!(
        provider.calls() > after_a,
        "a different merge-base must miss the cache and re-run AI"
    );
}

// ===========================================================================
// Full end-to-end through real git2 — `analyze_clusters_cached` with a tempdir cache.
// ===========================================================================

#[tokio::test]
async fn analyze_clusters_cached_e2e_second_open_is_zero_calls() {
    use git2::{Repository, Signature};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("Tester", "tester@example.com").unwrap();

    let path = dir.path().join("main.go");
    fs::write(
        &path,
        "package main\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n\nfunc Sub(a, b int) int {\n\treturn a - b\n}\n",
    )
    .unwrap();
    let base_oid = {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("main.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap()
    };
    let base_commit = repo.find_commit(base_oid).unwrap();
    repo.branch("main", &base_commit, true).unwrap();
    repo.branch("target", &base_commit, false).unwrap();
    repo.set_head("refs/heads/target").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    fs::write(
        &path,
        "package main\n\nfunc Add(a, b int) int {\n\treturn a + b + 0\n}\n\nfunc Sub(a, b int) int {\n\treturn a - b - 0\n}\n",
    )
    .unwrap();
    {
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("main.go")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(base_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "change both", &tree, &[&parent]).unwrap();
    }

    let repo_path = dir.path().to_str().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open_in_memory().unwrap();
    let _ = &cache_dir; // (open_in_dir path is covered by cache_tests; use in-memory here)
    let provider = CountingProvider::new();

    // 1st analysis: AI runs.
    let first = super::analyze_clusters_cached(&provider, &cache, repo_path, "main", "target", &())
        .await
        .expect("first analysis");
    let after_first = provider.calls();
    assert!(after_first > 0, "first analysis calls the AI");
    assert!(!first.ordered_card_ids.is_empty(), "produced a layout");

    // 2nd analysis: full-layout hit ⇒ ZERO AI calls + byte-identical.
    let second = super::analyze_clusters_cached(&provider, &cache, repo_path, "main", "target", &())
        .await
        .expect("second analysis");
    assert_eq!(provider.calls(), after_first, "second open ⇒ AI 0 calls (full layout hit)");
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap(),
        "same head ⇒ byte-identical layout (§8.1)"
    );
}
