//! Stage-④ verification tests (M4 whitelist + token check, planning §8.3).

use super::*;
use crate::engine::ai::steps::{
    AiCluster, ClusterLabel, ClusterResult, LabelResult, OrderResult, OrderedCluster, SuggestionOut,
};
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

fn clustering(clusters: Vec<AiCluster>, unclustered: &[&str]) -> ClusterResult {
    ClusterResult {
        clusters,
        unclustered: unclustered.iter().map(|s| s.to_string()).collect(),
    }
}

fn ordered(id: &str, ids: &[&str]) -> OrderedCluster {
    OrderedCluster {
        cluster_id: id.to_string(),
        card_ids: ids.iter().map(|s| s.to_string()).collect(),
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

// ---------------------------------------------------------------------------
// 4. verify_order (Stage-⑤ permutation parity)
// ---------------------------------------------------------------------------

#[test]
fn order_permutation_of_members_passes_and_normalizes_cluster_order() {
    let clusters = clustering(vec![cluster("c1", &["a", "b"]), cluster("c2", &["c"])], &["d"]);
    let order = OrderResult {
        cluster_order: vec!["c2".into(), "c1".into()],
        ordered_by_cluster: vec![ordered("c1", &["b", "a"]), ordered("c2", &["c"])],
        unclustered: vec![],
    };
    let out = verify_order(order, &clusters, &wl(&["a", "b", "c", "d"])).unwrap();
    assert_eq!(out.cluster_order, vec!["c2".to_string(), "c1".to_string()]);
    assert_eq!(out.ordered_by_cluster[0].card_ids, vec!["b".to_string(), "a".to_string()]);
}

#[test]
fn order_dropping_a_member_is_rejected() {
    let clusters = clustering(vec![cluster("c1", &["a", "b", "c"])], &[]);
    // c is missing from the ordering → not a permutation of the members.
    let order = OrderResult {
        cluster_order: vec!["c1".into()],
        ordered_by_cluster: vec![ordered("c1", &["a", "b"])],
        unclustered: vec![],
    };
    let err = verify_order(order, &clusters, &wl(&["a", "b", "c"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
}

#[test]
fn order_moving_a_member_to_another_cluster_is_rejected() {
    let clusters = clustering(vec![cluster("c1", &["a"]), cluster("c2", &["b"])], &[]);
    // b was clustered in c2 but the ordering puts it in c1 → membership parity fails.
    let order = OrderResult {
        cluster_order: vec!["c1".into(), "c2".into()],
        ordered_by_cluster: vec![ordered("c1", &["a", "b"]), ordered("c2", &[])],
        unclustered: vec![],
    };
    let err = verify_order(order, &clusters, &wl(&["a", "b"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
}

#[test]
fn order_hallucinated_id_is_rejected() {
    let clusters = clustering(vec![cluster("c1", &["a"])], &[]);
    let order = OrderResult {
        cluster_order: vec!["c1".into()],
        ordered_by_cluster: vec![ordered("c1", &["ghost"])],
        unclustered: vec![],
    };
    let err = verify_order(order, &clusters, &wl(&["a"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
}

#[test]
fn order_omitting_a_whole_cluster_is_rejected() {
    let clusters = clustering(vec![cluster("c1", &["a"]), cluster("c2", &["b"])], &[]);
    // c2 is never ordered → its members would be dropped.
    let order = OrderResult {
        cluster_order: vec!["c1".into()],
        ordered_by_cluster: vec![ordered("c1", &["a"])],
        unclustered: vec![],
    };
    let err = verify_order(order, &clusters, &wl(&["a", "b"])).unwrap_err();
    assert!(matches!(err, LlmError::Parse(_)));
}

#[test]
fn order_missing_cluster_order_id_is_backfilled_not_rejected() {
    let clusters = clustering(vec![cluster("c1", &["a"]), cluster("c2", &["b"])], &[]);
    // clusterOrder only lists c1; c2 must be appended (deterministic), not rejected.
    let order = OrderResult {
        cluster_order: vec!["c1".into()],
        ordered_by_cluster: vec![ordered("c1", &["a"]), ordered("c2", &["b"])],
        unclustered: vec![],
    };
    let out = verify_order(order, &clusters, &wl(&["a", "b"])).unwrap();
    assert_eq!(out.cluster_order, vec!["c1".to_string(), "c2".to_string()]);
}

// ---------------------------------------------------------------------------
// 5. verify_labels (Stage-⑥ B1 + M4 + suggestion filtering)
// ---------------------------------------------------------------------------

fn label(id: &str, title: &str, summary: &str) -> ClusterLabel {
    ClusterLabel {
        cluster_id: id.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
    }
}

#[test]
fn labels_empty_title_summary_get_fallback_b1() {
    let labels = LabelResult {
        clusters: vec![label("c1", "", "   ")],
        merge_suggestions: vec![],
        split_suggestions: vec![],
    };
    let (out, _sus) = verify_labels(labels, &wl(&["c1"]), &wl(&[]));
    assert!(!out.clusters[0].title.trim().is_empty(), "B1: title never empty");
    assert!(!out.clusters[0].summary.trim().is_empty(), "B1: summary never empty");
}

#[test]
fn labels_flag_suspicious_identifier_but_keep_label() {
    let labels = LabelResult {
        clusters: vec![label("c1", "Order", "Calls PaymentGateway.charge() too.")],
        merge_suggestions: vec![],
        split_suggestions: vec![],
    };
    let (out, sus) = verify_labels(labels, &wl(&["c1"]), &wl(&["createOrder"]));
    assert!(sus.get("c1").map(|v| !v.is_empty()).unwrap_or(false), "PaymentGateway flagged");
    assert!(!out.clusters[0].summary.is_empty(), "label is still kept (not fatal)");
}

#[test]
fn labels_drop_suggestion_with_unknown_cluster() {
    let labels = LabelResult {
        clusters: vec![label("c1", "T", "S.")],
        merge_suggestions: vec![SuggestionOut {
            cluster_ids: vec!["c1".into(), "ghost".into()],
            reason: "x".into(),
        }],
        split_suggestions: vec![SuggestionOut {
            cluster_ids: vec!["c1".into()],
            reason: "keep me".into(),
        }],
    };
    let (out, _sus) = verify_labels(labels, &wl(&["c1"]), &wl(&[]));
    assert!(out.merge_suggestions.is_empty(), "merge naming unknown cluster dropped");
    assert_eq!(out.split_suggestions.len(), 1, "valid split suggestion kept");
}
