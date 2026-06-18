//! Stage-④ verification tests (M4 whitelist + token check, planning §8.3).

use super::*;
use crate::engine::ai::steps::{AiCluster, ClusterResult};
use crate::engine::model::ClusterKind;
use std::collections::BTreeSet;

fn wl(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn cluster(id: &str, members: &[&str]) -> AiCluster {
    AiCluster {
        cluster_id: id.to_string(),
        member_card_ids: members.iter().map(|s| s.to_string()).collect(),
        kind: ClusterKind::Flow,
    }
}

// ---------------------------------------------------------------------------
// 1. hallucination reject
// ---------------------------------------------------------------------------

#[test]
fn hallucinated_member_id_is_rejected() {
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a", "ghost"])],
        unclustered: vec![],
    };
    let err = verify_clusters(result, &wl(&["a", "b"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)), "expected reject, got {err:?}");
}

#[test]
fn hallucinated_unclustered_id_is_rejected() {
    let result = ClusterResult {
        clusters: vec![],
        unclustered: vec!["nope".into()],
    };
    let err = verify_clusters(result, &wl(&["a"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
}

// ---------------------------------------------------------------------------
// 2. no-drop absorption
// ---------------------------------------------------------------------------

#[test]
fn omitted_whitelist_id_is_absorbed_into_unclustered() {
    // Whitelist has a,b,c but the AI only placed a and b → c is absorbed (not dropped).
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a", "b"])],
        unclustered: vec![],
    };
    let out = verify_clusters(result, &wl(&["a", "b", "c"])).unwrap();
    assert_eq!(out.unclustered, vec!["c".to_string()], "c must be absorbed, never dropped");
    // The cluster is untouched.
    assert_eq!(out.clusters[0].member_card_ids, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn nothing_is_dropped_every_whitelist_id_is_present() {
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a"])],
        unclustered: vec!["b".into()],
    };
    let out = verify_clusters(result, &wl(&["a", "b", "c", "d"])).unwrap();
    // Union of all placed + unclustered == the whole whitelist.
    let mut all: BTreeSet<String> = out.unclustered.iter().cloned().collect();
    for c in &out.clusters {
        all.extend(c.member_card_ids.iter().cloned());
    }
    assert_eq!(all, wl(&["a", "b", "c", "d"]));
}

#[test]
fn duplicate_placement_is_deduplicated_first_cluster_wins() {
    // The AI placed `a` in both c1 and c2 → c1 keeps it, c2 loses it.
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a", "b"]), cluster("c2", &["a", "c"])],
        unclustered: vec![],
    };
    let out = verify_clusters(result, &wl(&["a", "b", "c"])).unwrap();
    assert_eq!(out.clusters[0].member_card_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(out.clusters[1].member_card_ids, vec!["c".to_string()]);
    // `a` is not also in unclustered.
    assert!(!out.unclustered.contains(&"a".to_string()));
}

#[test]
fn cluster_emptied_by_dedup_is_removed() {
    // c2 only had `a`, which c1 already claimed → c2 becomes empty and is dropped.
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a"]), cluster("c2", &["a"])],
        unclustered: vec![],
    };
    let out = verify_clusters(result, &wl(&["a"])).unwrap();
    assert_eq!(out.clusters.len(), 1);
    assert_eq!(out.clusters[0].cluster_id, "c1");
}

#[test]
fn valid_result_passes_unchanged() {
    let result = ClusterResult {
        clusters: vec![cluster("c1", &["a", "b"])],
        unclustered: vec!["c".into()],
    };
    let out = verify_clusters(result.clone(), &wl(&["a", "b", "c"])).unwrap();
    assert_eq!(out, result);
}

// ---------------------------------------------------------------------------
// 3. M4 token whitelist (loose code-identifier check)
// ---------------------------------------------------------------------------

#[test]
fn natural_language_words_pass_through() {
    let allowed = wl(&["createOrder", "OrderService"]);
    let text = "Creates an order and applies the discount before saving it.";
    assert!(
        suspicious_identifiers(text, &allowed).is_empty(),
        "plain words must not be flagged"
    );
}

#[test]
fn allowed_code_identifier_passes() {
    let allowed = wl(&["createOrder", "OrderService"]);
    let text = "OrderService.createOrder() now applies the coupon.";
    let bad = suspicious_identifiers(text, &allowed);
    assert!(bad.is_empty(), "allowed identifiers must pass: {bad:?}");
}

#[test]
fn hallucinated_code_identifier_is_flagged() {
    // PaymentGateway is NOT in the allowed set → flagged as suspicious.
    let allowed = wl(&["createOrder", "OrderService"]);
    let text = "Calls PaymentGateway.charge() to settle the order.";
    let bad = suspicious_identifiers(text, &allowed);
    assert!(
        bad.iter().any(|t| t.contains("PaymentGateway")),
        "PaymentGateway.charge() should be flagged: {bad:?}"
    );
}

#[test]
fn snake_case_unknown_identifier_is_flagged() {
    let allowed = wl(&["create_order"]);
    let text = "delegates to delete_account internally";
    let bad = suspicious_identifiers(text, &allowed);
    assert!(bad.iter().any(|t| t == "delete_account"), "got {bad:?}");
}

#[test]
fn owner_or_member_match_is_enough() {
    // "OrderService.unknownMethod" — owner OrderService is allowed → not flagged
    // (loose check: any base matching is enough; perfect coverage is impossible §8.3).
    let allowed = wl(&["OrderService"]);
    let text = "via OrderService.unknownMethod()";
    let bad = suspicious_identifiers(text, &allowed);
    assert!(bad.is_empty(), "owner match should suffice: {bad:?}");
}
