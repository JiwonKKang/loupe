//! ⑦ cache-module unit tests: `card_hash` determinism/normalization, the two-table
//! get/put round-trips, the merge-base (M3) key, and `Mutex<Connection>` (M2) concurrency.

use super::*;
use crate::engine::clustercard::{ChangedSymbolIn, ClusterCardInput};
use crate::engine::model::{ChangeType, Cluster, ClusterKind, SymbolKind};
use crate::engine::relations::RelationHints;
use crate::engine::{ClusterLayout, Suggestion};

fn sym(id: &str, name: &str) -> ChangedSymbolIn {
    ChangedSymbolIn {
        card_id: id.to_string(),
        name: name.to_string(),
        kind: SymbolKind::Function,
        change_type: ChangeType::Modified,
        summary: format!("Updates {name}."),
        snippet: format!("+// {name} changed"),
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

fn layout(cluster_id: &str, ids: &[&str]) -> ClusterLayout {
    ClusterLayout {
        clusters: vec![Cluster {
            id: cluster_id.to_string(),
            title: "제목".to_string(),
            summary: "요약".to_string(),
            kind: ClusterKind::Flow,
            type_hint: ClusterKind::Flow,
            ordered_card_ids: ids.iter().map(|s| s.to_string()).collect(),
        }],
        cluster_order: vec![cluster_id.to_string()],
        ordered_card_ids: ids.iter().map(|s| s.to_string()).collect(),
        unclustered: vec![],
        merge_suggestions: vec![],
        split_suggestions: vec![],
        // Per-card AI summaries round-trip through the layout cache (Stage-⑥).
        card_summaries: ids
            .iter()
            .map(|id| (id.to_string(), format!("{id} 변경 요약")))
            .collect(),
    }
}

#[test]
fn card_hash_is_stable_across_runs() {
    let c = card("seed-1", &[("a", "create"), ("b", "save")]);
    assert_eq!(card_hash(&c), card_hash(&c), "same card ⇒ same hash");
    // 64 lowercase hex chars (SHA-256).
    let h = card_hash(&c);
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|ch| ch.is_ascii_hexdigit()));
}

#[test]
fn card_hash_excludes_seed_id() {
    // Same content under a different seed label ⇒ same hash (cluster_id is volatile).
    let a = card("seed-1", &[("a", "create"), ("b", "save")]);
    let b = card("seed-7", &[("a", "create"), ("b", "save")]);
    assert_eq!(card_hash(&a), card_hash(&b), "seed label must not affect the hash");
}

#[test]
fn card_hash_ignores_changed_symbol_order() {
    // The symbol array is sorted by cardId before hashing ⇒ order-independent.
    let a = card("seed-1", &[("a", "create"), ("b", "save")]);
    let b = card("seed-1", &[("b", "save"), ("a", "create")]);
    assert_eq!(card_hash(&a), card_hash(&b), "symbol order must not affect the hash");
}

#[test]
fn card_hash_changes_with_content() {
    // A different symbol summary is real content ⇒ different hash (it would re-trigger AI).
    let mut a = card("seed-1", &[("a", "create")]);
    let b = card("seed-1", &[("a", "create")]);
    a.changed_symbols[0].summary = "Totally different behaviour now.".to_string();
    assert_ne!(card_hash(&a), card_hash(&b), "content change ⇒ different hash");
}

#[test]
fn card_hash_changes_with_change_type() {
    let mut a = card("seed-1", &[("a", "create")]);
    let b = card("seed-1", &[("a", "create")]);
    a.changed_symbols[0].change_type = ChangeType::Added;
    assert_ne!(card_hash(&a), card_hash(&b), "changeType is content");
}

#[test]
fn layout_round_trips_through_review_layout_table() {
    let cache = Cache::open_in_memory().unwrap();
    let l = layout("k1", &["a", "b"]);
    assert!(cache.get_layout("/repo", "mb", "head").is_none(), "cold ⇒ miss");

    cache.put_layout("/repo", "mb", "head", &l).unwrap();
    let got = cache.get_layout("/repo", "mb", "head").expect("hit");
    assert_eq!(got, l, "round-trip is byte-identical");
}

#[test]
fn layout_key_is_merge_base_not_base_tip() {
    // M3: the layout key uses the merge-base SHA. A different merge-base ⇒ a miss; the same
    // merge-base with a different (irrelevant) value never collides.
    let cache = Cache::open_in_memory().unwrap();
    let l = layout("k1", &["a"]);
    cache.put_layout("/repo", "mergebaseAAA", "head", &l).unwrap();

    // Same head, DIFFERENT merge-base ⇒ miss (the 3-dot base changed).
    assert!(cache.get_layout("/repo", "mergebaseBBB", "head").is_none());
    // Same merge-base + head ⇒ hit.
    assert!(cache.get_layout("/repo", "mergebaseAAA", "head").is_some());
    // Different repo ⇒ miss (repo_path is part of the key).
    assert!(cache.get_layout("/other", "mergebaseAAA", "head").is_none());
}

#[test]
fn cluster_result_round_trips_and_is_keyed_by_card_hash() {
    let cache = Cache::open_in_memory().unwrap();
    let c = card("seed-1", &[("a", "create")]);
    let hash = card_hash(&c);
    let frag = layout("k1", &["a"]);

    assert!(cache.get_cluster("/repo", "mb", &hash).is_none(), "cold ⇒ miss");
    cache.put_cluster("/repo", "mb", &hash, &frag).unwrap();
    assert_eq!(cache.get_cluster("/repo", "mb", &hash).unwrap(), frag);

    // A different card_hash (changed content) ⇒ miss ⇒ would re-run AI (부분 무효화).
    let mut c2 = c.clone();
    c2.changed_symbols[0].summary = "changed".into();
    assert!(cache.get_cluster("/repo", "mb", &card_hash(&c2)).is_none());
}

#[test]
fn cluster_result_survives_head_change_same_merge_base() {
    // 부분 무효화 핵심: cluster_result has NO head in its key. A push that changes `head` but
    // not a seed's content reuses the seed via (merge_base, card_hash).
    let cache = Cache::open_in_memory().unwrap();
    let c = card("seed-1", &[("a", "create")]);
    let hash = card_hash(&c);
    cache.put_cluster("/repo", "mb", &hash, &layout("k1", &["a"])).unwrap();
    // Different head would be a layout miss, but the per-seed result is still a hit.
    assert!(cache.get_cluster("/repo", "mb", &hash).is_some());
}

#[test]
fn put_is_idempotent_replace() {
    let cache = Cache::open_in_memory().unwrap();
    cache.put_layout("/repo", "mb", "head", &layout("k1", &["a"])).unwrap();
    let l2 = layout("k1", &["a", "b"]);
    cache.put_layout("/repo", "mb", "head", &l2).unwrap(); // REPLACE, not a PK violation
    assert_eq!(cache.get_layout("/repo", "mb", "head").unwrap(), l2);
}

#[test]
fn open_in_dir_persists_to_a_file() {
    // The IPC path uses a directory (app_data_dir later); tests use a tempdir.
    let dir = tempfile::tempdir().unwrap();
    let l = layout("k1", &["a"]);
    {
        let cache = Cache::open_in_dir(dir.path()).unwrap();
        cache.put_layout("/repo", "mb", "head", &l).unwrap();
    }
    // Re-open the same dir ⇒ the row is still there (persisted to disk).
    let cache = Cache::open_in_dir(dir.path()).unwrap();
    assert_eq!(cache.get_layout("/repo", "mb", "head").unwrap(), l);
    // The db file exists.
    assert!(dir.path().join("loupe-cache.sqlite").exists());
}

#[test]
fn mutex_connection_allows_concurrent_access() {
    // M2: the `Mutex<Connection>` makes the cache `Sync`; hammer it from several threads.
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(Cache::open_in_memory().unwrap());
    let mut handles = Vec::new();
    for t in 0..8 {
        let cache = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                let head = format!("head-{t}-{i}");
                let l = layout("k1", &["a"]);
                cache.put_layout("/repo", "mb", &head, &l).unwrap();
                let got = cache.get_layout("/repo", "mb", &head).unwrap();
                assert_eq!(got, l);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // A late read still hits (no corruption under contention).
    assert!(cache.get_layout("/repo", "mb", "head-0-0").is_some());
}
