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

// ===========================================================================
// [DEBUG-cl9x] TEMP diagnosis: "everything goes to Unclustered" bug.
//
// Reproduces the full analyze_clusters_cached path on dearday and compares it,
// stage by stage, against the cluster_step(all-cards-at-once) path that the
// prior dearday_cli_sonnet_full_pipeline test exercised. The hypothesis under
// test: run_cluster_pipeline_cached runs the AI on ONE seed at a time, so no
// cross-seed clustering happens and singleton seeds fall to unclustered.
//
//   LOUPE_OAUTH_TOKEN=sk-ant-oat01-... \
//   cargo test -- --ignored --nocapture all_unclustered_repro
// ===========================================================================
#[tokio::test]
#[ignore = "live CliProvider — opt in with `cargo test -- --ignored all_unclustered_repro`"]
async fn all_unclustered_repro() {
    use crate::engine::ai::steps::{cluster_step, is_small_pr};
    use crate::engine::{
        analyze_relations, build_cluster_cards_with_signals, build_review, run_cluster_pipeline,
    };

    let Ok(token) = std::env::var("LOUPE_OAUTH_TOKEN") else {
        eprintln!("LOUPE_OAUTH_TOKEN not set — skipping all_unclustered_repro");
        return;
    };
    let repo = std::env::var("LOUPE_DEARDAY_REPO")
        .unwrap_or_else(|_| "/Users/jiwon/desktop/projects/dearday".to_string());
    let base = std::env::var("LOUPE_DEARDAY_BASE").unwrap_or_else(|_| "main".to_string());
    let target =
        std::env::var("LOUPE_DEARDAY_TARGET").unwrap_or_else(|_| "feat/kakao-auth".to_string());

    if !std::path::Path::new(&repo).exists() {
        eprintln!("dearday repo absent — skipping");
        return;
    }

    let provider = CliProvider::new(token);

    // Rebuild the exact seed cards the cached pipeline builds.
    let review = build_review(&repo, &base, &target).expect("build_review");
    let analysis = analyze_relations(&repo, &base, &target).expect("analyze_relations");
    let cards = build_cluster_cards_with_signals(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
        &analysis.base_signals,
    );

    eprintln!("\n[DEBUG-cl9x] ===== seed cards =====");
    eprintln!("[DEBUG-cl9x] total seeds (input cards) = {}", cards.len());
    let total_syms: usize = cards.iter().map(|c| c.changed_symbols.len()).sum();
    eprintln!("[DEBUG-cl9x] total changed symbols (whitelist) = {total_syms}");
    eprintln!(
        "[DEBUG-cl9x] is_small_pr(ALL cards) = {} (SMALL_PR_SYMBOLS=12)",
        is_small_pr(&cards)
    );
    for (i, c) in cards.iter().enumerate() {
        eprintln!(
            "[DEBUG-cl9x]   seed[{i}] id={} symbols={} is_small_pr(this seed alone)={}",
            c.cluster_id,
            c.changed_symbols.len(),
            is_small_pr(std::slice::from_ref(c))
        );
    }

    // --------- PATH A: per-seed (what analyze_clusters_cached actually does) ---------
    eprintln!("\n[DEBUG-cl9x] ===== PATH A: run_cluster_pipeline PER SEED =====");
    let mut a_clusters = 0usize;
    let mut a_unclustered = 0usize;
    for (i, card) in cards.iter().enumerate() {
        let one = std::slice::from_ref(card);
        // PATH A reproduces the OLD per-seed product behaviour (single-seed AI input). The
        // pre-fix code restricted hints to the seed's members; the full hints here are a
        // superset and don't change the single-seed fragmentation this path demonstrates.
        match run_cluster_pipeline(&provider, one, &analysis.hints).await {
            Ok(frag) => {
                eprintln!(
                    "[DEBUG-cl9x]   seed[{i}] {} -> clusters={} unclustered={} (members={:?})",
                    card.cluster_id,
                    frag.clusters.len(),
                    frag.unclustered.len(),
                    frag.clusters
                        .iter()
                        .map(|c| c.ordered_card_ids.len())
                        .collect::<Vec<_>>()
                );
                a_clusters += frag.clusters.len();
                a_unclustered += frag.unclustered.len();
            }
            Err(e) => eprintln!("[DEBUG-cl9x]   seed[{i}] {} -> ERR {e}", card.cluster_id),
        }
    }
    eprintln!(
        "[DEBUG-cl9x] PATH A TOTAL: clusters={a_clusters} unclustered={a_unclustered}"
    );

    // --------- PATH B: cluster_step over ALL cards at once (prior passing test) ------
    eprintln!("\n[DEBUG-cl9x] ===== PATH B: cluster_step(ALL cards at once) =====");
    match cluster_step(&provider, &cards).await {
        Ok(res) => {
            eprintln!(
                "[DEBUG-cl9x] PATH B: clusters={} unclustered={}",
                res.clusters.len(),
                res.unclustered.len()
            );
            for c in &res.clusters {
                eprintln!(
                    "[DEBUG-cl9x]   cluster {} kind={:?} members={}",
                    c.cluster_id,
                    c.kind,
                    c.member_card_ids.len()
                );
            }
        }
        Err(e) => eprintln!("[DEBUG-cl9x] PATH B ERR {e}"),
    }

    // --------- PATH C: the actual product entry point, end to end --------------------
    eprintln!("\n[DEBUG-cl9x] ===== PATH C: analyze_clusters_cached (product path) =====");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open_in_dir(cache_dir.path()).unwrap();
    match analyze_clusters_cached(&provider, &cache, &repo, &base, &target).await {
        Ok(layout) => {
            eprintln!(
                "[DEBUG-cl9x] PATH C: clusters={} unclustered={} ordered={}",
                layout.clusters.len(),
                layout.unclustered.len(),
                layout.ordered_card_ids.len()
            );
            for c in &layout.clusters {
                eprintln!(
                    "[DEBUG-cl9x]   cluster {} '{}' members={:?}",
                    c.id, c.title, c.ordered_card_ids
                );
            }
            eprintln!("[DEBUG-cl9x]   unclustered ids = {:?}", layout.unclustered);
        }
        Err(e) => eprintln!("[DEBUG-cl9x] PATH C ERR {e}"),
    }
    eprintln!("\n[DEBUG-cl9x] ===== end =====\n");
}

// ===========================================================================
// [DEBUG-caddy] TEMP diagnosis for ISSUE B — "unclustered 과다" on an infra PR.
//
// Full structural breakdown of dearday main...feat/https-via-caddy:
//   - EVERY Stage-1 card (id / kind / symbol / path / summary) so we can see how
//     many are file-level (orphan/non-code/added/deleted) vs changed-symbol cards.
//   - the changed-symbol whitelist (the ONLY ids the AI ever clusters).
//   - the seeds (first-pass strong-relation components).
//   - the final layout: which card ids are clustered, which are unclustered, and
//     CRUCIALLY which cards are in NEITHER (never entered the whitelist at all).
//
//   LOUPE_OAUTH_TOKEN=sk-ant-oat01-... \
//   cargo test -p dearday-loupe -- --ignored --nocapture caddy_unclustered_breakdown
// (target/base overridable via LOUPE_DEARDAY_{REPO,BASE,TARGET}.)
// ===========================================================================
#[tokio::test]
#[ignore = "live CliProvider — opt in with `cargo test -- --ignored caddy_unclustered_breakdown`"]
async fn caddy_unclustered_breakdown() {
    use crate::engine::{
        analyze_relations, build_cluster_cards_with_signals, build_file_seed_cards, build_review,
        run_cluster_pipeline,
    };
    use std::collections::BTreeSet;

    let repo = std::env::var("LOUPE_DEARDAY_REPO")
        .unwrap_or_else(|_| "/Users/jiwon/desktop/projects/dearday".to_string());
    let base = std::env::var("LOUPE_DEARDAY_BASE").unwrap_or_else(|_| "main".to_string());
    let target = std::env::var("LOUPE_DEARDAY_TARGET")
        .unwrap_or_else(|_| "feat/https-via-caddy".to_string());

    if !std::path::Path::new(&repo).exists() {
        eprintln!("[DEBUG-caddy] dearday repo absent — skipping");
        return;
    }

    // ---- PURE (no AI) structural facts: cards, whitelist, seeds. ----
    let review = build_review(&repo, &base, &target).expect("build_review");
    let analysis = analyze_relations(&repo, &base, &target).expect("analyze_relations");
    // Symbol seed cards (Stage-③) …
    let mut cards_in = build_cluster_cards_with_signals(
        &analysis.seeds,
        &analysis.hints,
        &analysis.changed,
        &review.cards,
        &analysis.base_signals,
    );
    // … PLUS the new file-level seeds (Issue C — infra/config topic clustering). These are
    // the cards that used to fall straight to Unclustered; they now enter the whitelist.
    let already_seeded: BTreeSet<String> = cards_in
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.clone()))
        .collect();
    let file_seeds = build_file_seed_cards(&review.cards, &already_seeded);
    eprintln!(
        "\n[DEBUG-caddy] symbol seed-cards = {}, NEW file seed-cards = {} (these used to be Unclustered)",
        cards_in.len(),
        file_seeds.len()
    );
    cards_in.extend(file_seeds);

    eprintln!("\n[DEBUG-caddy] ===== ALL Stage-1 CARDS ({}) =====", review.cards.len());
    for c in &review.cards {
        eprintln!(
            "[DEBUG-caddy]   card kind={:<8?} change={:<8?} id={}  | summary={:?}",
            c.kind, c.change_type, c.id, c.summary
        );
    }

    // The whitelist = every changed-symbol card id the AI is allowed to cluster.
    let whitelist: BTreeSet<String> = cards_in
        .iter()
        .flat_map(|c| c.changed_symbols.iter().map(|s| s.card_id.clone()))
        .collect();
    eprintln!(
        "\n[DEBUG-caddy] ===== WHITELIST (changed-symbol cards only) = {} of {} total cards =====",
        whitelist.len(),
        review.cards.len()
    );
    for id in &whitelist {
        eprintln!("[DEBUG-caddy]   whitelist id = {id}");
    }

    // Cards that NEVER enter the AI input (not changed-symbol → not clustered, not unclustered).
    let not_in_whitelist: Vec<&str> = review
        .cards
        .iter()
        .map(|c| c.id.as_str())
        .filter(|id| !whitelist.contains(*id))
        .collect();
    eprintln!(
        "\n[DEBUG-caddy] ===== CARDS OUTSIDE THE WHITELIST = {} (file-level / non-code / added / orphan) =====",
        not_in_whitelist.len()
    );
    for id in &not_in_whitelist {
        eprintln!("[DEBUG-caddy]   NOT-clusterable card id = {id}");
    }

    eprintln!("\n[DEBUG-caddy] ===== SEEDS ({}) =====", analysis.seeds.len());
    for s in &analysis.seeds {
        eprintln!("[DEBUG-caddy]   {} members={} {:?}", s.id, s.card_ids.len(), s.card_ids);
    }

    // ---- AI path (needs token). ----
    let Ok(token) = std::env::var("LOUPE_OAUTH_TOKEN") else {
        eprintln!("\n[DEBUG-caddy] LOUPE_OAUTH_TOKEN not set — stopping after the pure breakdown.");
        return;
    };
    let provider = CliProvider::new(token);

    eprintln!("\n[DEBUG-caddy] ===== run_cluster_pipeline (ALL cards at once) =====");
    match run_cluster_pipeline(&provider, &cards_in, &analysis.hints).await {
        Ok(layout) => {
            let clustered: BTreeSet<&str> = layout
                .clusters
                .iter()
                .flat_map(|c| c.ordered_card_ids.iter().map(String::as_str))
                .collect();
            eprintln!(
                "[DEBUG-caddy] clusters={} clustered_ids={} unclustered={} (whitelist={})",
                layout.clusters.len(),
                clustered.len(),
                layout.unclustered.len(),
                whitelist.len()
            );
            for c in &layout.clusters {
                eprintln!(
                    "[DEBUG-caddy]   cluster {} kind={:?} title={:?}\n[DEBUG-caddy]      summary={:?}\n[DEBUG-caddy]      members={:?}",
                    c.id, c.kind, c.title, c.summary, c.ordered_card_ids
                );
            }
            eprintln!("[DEBUG-caddy]   unclustered ids = {:?}", layout.unclustered);
            eprintln!(
                "\n[DEBUG-caddy] SANITY: every whitelist id is clustered-or-unclustered? {}",
                whitelist.iter().all(|id| clustered.contains(id.as_str())
                    || layout.unclustered.iter().any(|u| u == id))
            );
        }
        Err(e) => eprintln!("[DEBUG-caddy] run_cluster_pipeline ERR {e}"),
    }
    eprintln!("\n[DEBUG-caddy] ===== end =====\n");
}
