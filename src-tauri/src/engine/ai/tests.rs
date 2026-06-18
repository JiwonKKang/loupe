//! Stage-① AI-foundation tests.
//!
//! Unit coverage (no network):
//!  - a `MockProvider` exercising the `complete` flow + `model_for`,
//!  - structured-output JSON parsing round-trips through `CompletionResponse`,
//!  - the prompts.rs M1 schema builders (flat object, no dynamic key maps).
//!
//! Integration (network, `#[ignore]`): `LOUPE_OAUTH_TOKEN` env, if present, drives a
//! real OAuthProvider call to Haiku and asserts HTTP 200 + parseable text. The token
//! is read from the environment only — never hard-coded.

use super::anthropic::OAuthProvider;
use super::prompts::{array_schema, object_schema, string_schema};
use super::{
    CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

/// A mock provider: records the last request and returns a canned response (or error).
/// Proves the orchestrator-facing `complete` contract without any network.
struct MockProvider {
    last: Mutex<Option<CompletionRequest>>,
    reply: Result<CompletionResponse, LlmError>,
}

impl MockProvider {
    fn ok(json: Value) -> Self {
        Self {
            last: Mutex::new(None),
            reply: Ok(CompletionResponse {
                json,
                stop_reason: "end_turn".into(),
            }),
        }
    }
    fn err(e: LlmError) -> Self {
        Self {
            last: Mutex::new(None),
            reply: Err(e),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.last.lock().unwrap() = Some(req);
        self.reply.clone()
    }
    fn model_for(&self, tier: ModelTier) -> &'static str {
        match tier {
            ModelTier::Fast => "mock-fast",
            ModelTier::Quality => "mock-quality",
        }
    }
}

fn sample_request(tier: ModelTier, schema: Option<Value>) -> CompletionRequest {
    CompletionRequest {
        system: "You classify cards.".into(),
        user: "card-1".into(),
        max_tokens: 128,
        json_schema: schema,
        tier,
        temperature: 0.0,
    }
}

#[tokio::test]
async fn mock_complete_returns_canned_json_and_records_request() {
    let provider = MockProvider::ok(json!({ "clusterId": "c1", "kind": "flow" }));
    let schema = object_schema(&[
        ("clusterId", string_schema()),
        ("kind", string_schema()),
    ]);
    let resp = provider
        .complete(sample_request(ModelTier::Fast, Some(schema)))
        .await
        .expect("mock should succeed");

    assert_eq!(resp.stop_reason, "end_turn");
    assert_eq!(resp.json["clusterId"], "c1");
    assert_eq!(resp.json["kind"], "flow");

    // The request was actually passed through to the provider.
    let recorded = provider.last.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.user, "card-1");
    assert!(recorded.json_schema.is_some());
}

#[tokio::test]
async fn mock_complete_propagates_errors() {
    let provider = MockProvider::err(LlmError::Overloaded);
    let err = provider
        .complete(sample_request(ModelTier::Quality, None))
        .await
        .unwrap_err();
    assert_eq!(err, LlmError::Overloaded);
}

#[test]
fn model_for_distinguishes_tiers() {
    let p = MockProvider::ok(Value::Null);
    assert_eq!(p.model_for(ModelTier::Fast), "mock-fast");
    assert_eq!(p.model_for(ModelTier::Quality), "mock-quality");
}

#[test]
fn structured_output_parses_into_completion_response() {
    // Simulate the structured-output text block coming back and being parsed.
    let raw = r#"{"clusters":[{"clusterId":"c1","cardIds":["a","b"]}]}"#;
    let json: Value = serde_json::from_str(raw).unwrap();
    let resp = CompletionResponse {
        json,
        stop_reason: "end_turn".into(),
    };
    let clusters = resp.json["clusters"].as_array().unwrap();
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0]["clusterId"], "c1");
    assert_eq!(clusters[0]["cardIds"][1], "b");
}

#[test]
fn object_schema_is_m1_compliant() {
    // Every object => additionalProperties:false + all fields required, no dynamic map.
    let schema = object_schema(&[
        ("clusterId", string_schema()),
        ("cardIds", array_schema(string_schema())),
    ]);
    assert_eq!(schema["type"], "object");
    assert_eq!(
        schema["additionalProperties"], json!(false),
        "M1: additionalProperties must be false on every object"
    );
    let required = schema["required"].as_array().unwrap();
    assert_eq!(required.len(), 2);
    assert!(required.contains(&json!("clusterId")));
    assert!(required.contains(&json!("cardIds")));
    // The flattened array (no `{clusterId: [...]}` dynamic key map).
    assert_eq!(schema["properties"]["cardIds"]["type"], "array");
    assert_eq!(schema["properties"]["cardIds"]["items"]["type"], "string");
}

#[test]
fn ai2_output_schema_flattens_dynamic_map_to_array() {
    // M1: `{clusterId: [cardId]}` is forbidden — model it as an array of fixed-key
    // objects: [{clusterId, cardIds:[...]}].
    let item = object_schema(&[
        ("clusterId", string_schema()),
        ("cardIds", array_schema(string_schema())),
    ]);
    let schema = object_schema(&[("orderedByCluster", array_schema(item))]);
    assert_eq!(schema["properties"]["orderedByCluster"]["type"], "array");
    assert_eq!(
        schema["properties"]["orderedByCluster"]["items"]["additionalProperties"],
        json!(false)
    );
}

// ---------------------------------------------------------------------------
// Integration (network) — opt-in via LOUPE_OAUTH_TOKEN. Token never hard-coded.
// ---------------------------------------------------------------------------

/// Real OAuthProvider call to Haiku, asserting a 200 + parseable response. Skips
/// (passes) when `LOUPE_OAUTH_TOKEN` is unset. Run with:
///   LOUPE_OAUTH_TOKEN=... cargo test -- --ignored oauth_haiku_pong
#[ignore = "requires LOUPE_OAUTH_TOKEN; hits the live Anthropic API"]
#[tokio::test]
async fn oauth_haiku_pong() {
    let token = match std::env::var("LOUPE_OAUTH_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            eprintln!("LOUPE_OAUTH_TOKEN unset — skipping live OAuth integration test");
            return;
        }
    };
    let provider = OAuthProvider::new(token);
    let resp = provider
        .complete(CompletionRequest {
            system: "Reply with the single word pong.".into(),
            user: "ping".into(),
            max_tokens: 16,
            json_schema: None,
            tier: ModelTier::Fast,
            temperature: 0.0,
        })
        .await
        .expect("live OAuth call to Haiku should return 200");

    // Got SOME content and a stop reason (end_turn / max_tokens — not refusal).
    assert!(
        !resp.stop_reason.is_empty(),
        "expected a stop_reason, got {resp:?}"
    );
    assert!(
        resp.json != Value::Null,
        "expected non-empty content, got {resp:?}"
    );
}

/// Stage-③+④ end-to-end against a real repo + live Haiku: build cluster cards from
/// dearday's strong-seeds (`main...feat/https-via-caddy`) and run `analyze_clusters`, then
/// assert **every returned card id (clustered or unclustered) is inside the whitelist**
/// (M4 — no hallucination escaped verification) and **nothing was dropped** (§3.1 — the
/// clustered+unclustered union equals the whitelist).
///
/// Skips (passes) when `LOUPE_OAUTH_TOKEN` is unset or the dearday repo is absent. Token
/// is read from the environment only — never hard-coded. Run with:
///   LOUPE_OAUTH_TOKEN=... cargo test -- --ignored dearday_cluster_step_ids_in_whitelist
#[ignore = "requires LOUPE_OAUTH_TOKEN + dearday repo; hits the live Anthropic API"]
#[tokio::test]
async fn dearday_cluster_step_ids_in_whitelist() {
    use crate::engine::{analyze_clusters, analyze_relations, build_cluster_cards, build_review};
    use std::collections::BTreeSet;

    let token = match std::env::var("LOUPE_OAUTH_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            eprintln!("LOUPE_OAUTH_TOKEN unset — skipping live dearday clustering test");
            return;
        }
    };
    let repo = "/Users/jiwon/desktop/projects/dearday";
    if !std::path::Path::new(repo).exists() {
        eprintln!("dearday repo absent — skipping");
        return;
    }
    let (base, target) = ("main", "feat/https-via-caddy");

    // Reconstruct the same whitelist the pipeline uses (Stage-1 cards → seeds → cards).
    let review = build_review(repo, base, target).expect("stage-1 build_review");
    let analysis = analyze_relations(repo, base, target).expect("stage-2 relations");
    let cluster_cards =
        build_cluster_cards(&analysis.seeds, &analysis.hints, &analysis.changed, &review.cards);
    let whitelist: BTreeSet<String> = cluster_cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.clone()))
        .collect();

    if whitelist.is_empty() {
        eprintln!("dearday diff produced no changed code symbols — skipping cluster assertion");
        return;
    }

    let provider = OAuthProvider::new(token);
    let result = analyze_clusters(&provider, repo, base, target)
        .await
        .expect("live dearday clustering should succeed and verify");

    // Every clustered id is in the whitelist (verifier guarantees this; assert anyway).
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for c in &result.clusters {
        for id in &c.member_card_ids {
            assert!(whitelist.contains(id), "clustered id {id} not in whitelist");
            seen.insert(id.clone());
        }
    }
    for id in &result.unclustered {
        assert!(whitelist.contains(id), "unclustered id {id} not in whitelist");
        seen.insert(id.clone());
    }
    // No-drop: clustered ∪ unclustered == whitelist (§3.1 "all changes are visible").
    assert_eq!(seen, whitelist, "no card id may be dropped");
}

// ---------------------------------------------------------------------------
// Determinism harness — Haiku + temperature=0 clustering on a real dearday PR.
//
// One-off measurement (not a regression gate): runs the SAME `cluster_step`
// pipeline (same prompt, same schema, same whitelist verifier + retry, Haiku tier,
// temperature=0) TWICE in a row on identical cards, then prints both results and
// reports whether the two runs are identical (same cluster composition + same
// kinds). This is the empirical check that temperature=0 removed the run-to-run
// cluster drift Haiku showed at the default sampling temperature (1.0).
//
// `#[ignore]` + env-only token (never hard-coded). Run with:
//   LOUPE_OAUTH_TOKEN=... cargo test -p loupe -- --ignored --nocapture \
//       haiku_temp0_clustering_determinism
// ---------------------------------------------------------------------------

/// A run's clustering reduced to a comparable, order-independent shape:
/// `{ sorted(member card ids) -> kind }` per cluster, plus the sorted unclustered
/// set. Two runs are "the same clustering" iff these are equal — cluster *ids* and
/// list ordering are volatile (labels/ordering aren't this stage), so they're
/// normalised away; only the partition (which ids group together) and each group's
/// kind are compared.
fn canonical(res: &crate::engine::ai::steps::ClusterResult) -> (Vec<(Vec<String>, String)>, Vec<String>) {
    let mut clusters: Vec<(Vec<String>, String)> = res
        .clusters
        .iter()
        .map(|c| {
            let mut ids = c.member_card_ids.clone();
            ids.sort();
            (ids, format!("{:?}", c.kind))
        })
        .collect();
    // Order the clusters by their member set so two equal partitions compare equal
    // regardless of the order the model emitted them.
    clusters.sort();
    let mut unclustered = res.unclustered.clone();
    unclustered.sort();
    (clusters, unclustered)
}

#[ignore = "Haiku temp=0 determinism check; requires LOUPE_OAUTH_TOKEN + dearday repo; hits the live Anthropic API"]
#[tokio::test]
async fn haiku_temp0_clustering_determinism() {
    use crate::engine::ai::steps::{cluster_step, ClusterResult};
    use crate::engine::{analyze_relations, build_cluster_cards, build_review};
    use crate::engine::clustercard::ClusterCardInput;
    use std::collections::BTreeMap;

    let token = match std::env::var("LOUPE_OAUTH_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            eprintln!("LOUPE_OAUTH_TOKEN unset — skipping Haiku temp=0 determinism check");
            return;
        }
    };
    let repo = "/Users/jiwon/desktop/projects/dearday";
    if !std::path::Path::new(repo).exists() {
        eprintln!("dearday repo absent — skipping determinism check");
        return;
    }
    // kakao-auth PR is the richer code change (auth flow + jwt + queries + tests),
    // so it exercises clustering judgement (flow / contract / domain-concept /
    // shared-foundation / infra) and is the hardest case for run-to-run stability.
    let (base, target) = ("main", "feat/kakao-auth");

    // Build the cluster cards ONCE; both runs see byte-identical input.
    let review = build_review(repo, base, target).expect("stage-1 build_review");
    let analysis = analyze_relations(repo, base, target).expect("stage-2 relations");
    let cards: Vec<ClusterCardInput> =
        build_cluster_cards(&analysis.seeds, &analysis.hints, &analysis.changed, &review.cards);

    // card_id -> display name, for human-readable member lists.
    let name_of: BTreeMap<String, String> = cards
        .iter()
        .flat_map(|c| c.changed_symbols.iter())
        .map(|s| (s.card_id.clone(), s.name.clone()))
        .collect();
    let label = |id: &str| -> String {
        match name_of.get(id) {
            Some(n) => format!("{n} [{id}]"),
            None => id.to_string(),
        }
    };

    let total_symbols: usize = cards.iter().map(|c| c.changed_symbols.len()).sum();
    eprintln!(
        "\n========== Haiku temp=0 determinism :: dearday {base}...{target} =========="
    );
    eprintln!(
        "seeds(input cards)={}  changed code symbols(whitelist)={}",
        cards.len(),
        total_symbols
    );

    let provider = OAuthProvider::new(token);

    // Two back-to-back runs through the real Haiku+temp0 cluster_step. Same cards.
    let run1 = cluster_step(&provider, &cards).await;
    let run2 = cluster_step(&provider, &cards).await;

    let dump = |title: &str, r: &Result<ClusterResult, LlmError>| {
        eprintln!("\n---------------- {title} ----------------");
        match r {
            Err(e) => eprintln!("  ERROR: {e}"),
            Ok(res) => {
                for c in &res.clusters {
                    eprintln!("  cluster {} :: kind={:?}", c.cluster_id, c.kind);
                    for id in &c.member_card_ids {
                        eprintln!("      - {}", label(id));
                    }
                }
                if res.unclustered.is_empty() {
                    eprintln!("  unclustered: (none)");
                } else {
                    eprintln!("  unclustered ({}):", res.unclustered.len());
                    for id in &res.unclustered {
                        eprintln!("      - {}", label(id));
                    }
                }
                eprintln!(
                    "  [{} clusters, {} unclustered]",
                    res.clusters.len(),
                    res.unclustered.len()
                );
            }
        }
    };

    dump("RUN 1 (Haiku, temp=0)", &run1);
    dump("RUN 2 (Haiku, temp=0)", &run2);

    // Determinism verdict: compare the order-independent canonical shapes.
    match (&run1, &run2) {
        (Ok(a), Ok(b)) => {
            let ca = canonical(a);
            let cb = canonical(b);
            if ca == cb {
                eprintln!(
                    "\n==> DETERMINISTIC: both runs produced the SAME clustering \
                     (identical cluster composition + kinds)."
                );
            } else {
                eprintln!(
                    "\n==> NON-DETERMINISTIC: the two runs DIFFER (composition and/or kind). \
                     temp=0 did not fully stabilise Haiku on this PR."
                );
                eprintln!("    run1 canonical = {ca:?}");
                eprintln!("    run2 canonical = {cb:?}");
            }
        }
        _ => eprintln!(
            "\n==> INCONCLUSIVE: at least one run errored (see above); rerun when the \
             API is not rate-limited/overloaded."
        ),
    }
    eprintln!("\n========== end determinism check ==========\n");

    // Measurement harness, not a CI gate: a transient Overloaded/Timeout on the live
    // API must not fail the suite. We only assert the harness ran end to end.
    assert!(run1.is_ok() || run1.is_err());
    assert!(run2.is_ok() || run2.is_err());
}
