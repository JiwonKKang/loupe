//! ⑦ — `#[ignore]` real-CliProvider cache test on the `dearday` repo (작성만, 실행 선택).
//!
//! This is the *live* proof of the latency win: a first `analyze_clusters_cached` runs the
//! real `claude` CLI (Sonnet, ~5분) and the second returns from the cache instantly (AI 0
//! calls) with an identical layout. It is `#[ignore]` so `cargo test` never runs it; opt in
//! with `cargo test -- --ignored dearday`. **No token is hardcoded** — it is read from the
//! `LOUPE_OAUTH_TOKEN` env var (the setup-token, `sk-ant-oat01-…`). The repo + refs are env
//! driven too so this file carries no machine-specific paths:
//!
//! ```sh
//! LOUPE_OAUTH_TOKEN=sk-ant-oat01-... \
//! LOUPE_DEARDAY_REPO=/abs/path/to/dearday \
//! LOUPE_DEARDAY_BASE=main \
//! LOUPE_DEARDAY_TARGET=some-feature-branch \
//! cargo test -- --ignored --nocapture dearday_cache_hit_is_instant
//! ```
//!
//! Without `LOUPE_OAUTH_TOKEN` the test no-ops (returns early) so an accidental
//! `--ignored` run on a machine without the token does not fail.

use super::cache::Cache;
use crate::engine::ai::cli::CliProvider;
use crate::engine::analyze_clusters_cached;
use std::time::Instant;

#[tokio::test]
#[ignore = "live CliProvider (~5min) — opt in with `cargo test -- --ignored dearday`"]
async fn dearday_cache_hit_is_instant() {
    let Ok(token) = std::env::var("LOUPE_OAUTH_TOKEN") else {
        eprintln!("LOUPE_OAUTH_TOKEN not set — skipping dearday live cache test");
        return;
    };
    let repo = std::env::var("LOUPE_DEARDAY_REPO")
        .expect("set LOUPE_DEARDAY_REPO to the dearday repo path");
    let base = std::env::var("LOUPE_DEARDAY_BASE").unwrap_or_else(|_| "main".to_string());
    let target = std::env::var("LOUPE_DEARDAY_TARGET")
        .expect("set LOUPE_DEARDAY_TARGET to the review target ref");

    // A tempdir cache so the test is self-contained (the IPC path will use app_data_dir).
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open_in_dir(cache_dir.path()).unwrap();
    let provider = CliProvider::new(token);

    // 1st pass — real Sonnet via the CLI (~5분).
    let t0 = Instant::now();
    let first = analyze_clusters_cached(&provider, &cache, &repo, &base, &target)
        .await
        .expect("first dearday analysis");
    let first_secs = t0.elapsed().as_secs_f64();
    eprintln!(
        "dearday 1st pass: {first_secs:.1}s, {} clusters, {} cards",
        first.clusters.len(),
        first.ordered_card_ids.len()
    );

    // 2nd pass — full-layout cache hit, must be near-instant and identical.
    let t1 = Instant::now();
    let second = analyze_clusters_cached(&provider, &cache, &repo, &base, &target)
        .await
        .expect("second dearday analysis");
    let second_secs = t1.elapsed().as_secs_f64();
    eprintln!("dearday 2nd pass (cache hit): {second_secs:.3}s");

    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap(),
        "cached 2nd pass is byte-identical to the 1st"
    );
    assert!(
        second_secs < 1.0,
        "cache hit should be near-instant, was {second_secs:.3}s"
    );
}
