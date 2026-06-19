//! ⑦ — SHA caching (planning §8.1 결정성 / §8.2 캐싱 / §8.4 레이턴시).
//!
//! AI clustering/ordering/labelling is slow (Sonnet via the `claude` CLI is ~70–100s a
//! call, ~5 min a pipeline) and not bit-deterministic. The cache turns "리뷰 한 건당 여러
//! 번"(§8.2) back into "한 번": once a `(repo, merge-base, head, schema)` layout is stored,
//! re-opening the same review returns it with **zero AI calls** and a byte-identical
//! result; once a *seed card* (`card_hash`) result is stored, a `head` change that does not
//! touch that seed's content reuses it (부분 무효화).
//!
//! ## Two tables, two grains
//!  - **`cluster_result`** — keyed by `(repo, merge_base_sha, card_hash, schema_ver)`. One
//!    row per *seed card*: the AI pipeline output for that single seed (its [`ClusterLayout`]
//!    fragment), as JSON. The grain of **부분 무효화** — a seed whose `card_hash` is unchanged
//!    is reused even when `head` moved (push). `merge_base_sha` (M3) anchors it to the
//!    actual 3-dot base, not the base branch tip.
//!  - **`review_layout`** — keyed by `(repo, merge_base_sha, head_sha, schema_ver)`. One row
//!    per *head*: the assembled inter-cluster layout (§8.1 결정성 — same head ⇒ same order).
//!    A full hit here is the 5분→즉시 path (AI 0 calls).
//!
//! ## M2 (v2-critique) — `Mutex<Connection>` + WAL + busy_timeout
//! The single `Connection` is wrapped in a `Mutex`; every `get`/`put` takes the lock, so a
//! background prewarm and a foreground read never corrupt the db. WAL mode + a busy_timeout
//! let concurrent opens (e.g. a future second connection) wait rather than fail.
//!
//! ## M3 (v2-critique) — the base key is the **merge-base SHA**
//! Both keys carry `merge_base_sha` (= `merge_base(base, target)`), never the base branch
//! tip. The 3-dot diff is `merge-base → target`; the tip can move without changing the diff,
//! so keying on the tip would miss the cache spuriously. See `gitdiff::DiffShas`.
//!
//! ## `schema_ver` — automatic invalidation on prompt/schema change
//! `schema_ver` is mixed into **both** the `card_hash` and the table keys. Bumping it when a
//! prompt or output schema changes makes every old row unreachable (a clean re-analysis)
//! without a migration.

use super::clustercard::ClusterCardInput;
use super::ClusterLayout;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;

/// Bump this when a prompt (`ai::prompts`) or an output schema changes so every cached row
/// (card hashes + layouts) becomes unreachable and the next analysis re-runs the AI. Mixed
/// into both the `card_hash` and the table keys.
pub const SCHEMA_VER: i64 = 1;

/// The SQLite-backed analysis cache (M2: a `Mutex<Connection>`). Construct with
/// [`Cache::open`] (a real file under a cache dir) or [`Cache::open_in_dir`] (tests pass a
/// tempdir). All access goes through `get_*` / `put_*`, which take the mutex internally.
pub struct Cache {
    conn: Mutex<Connection>,
}

impl Cache {
    /// Open (creating if needed) the cache db `loupe-cache.sqlite` inside `cache_dir`.
    /// The directory is created if missing. Sets WAL + a busy_timeout and ensures the
    /// schema (M2). `cache_dir` is a parameter so tests pass a `tempdir`; the IPC layer
    /// will pass the app data dir later (deferred — not this stage's scope).
    pub fn open_in_dir(cache_dir: &Path) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(cache_dir).map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                Some(format!("create cache dir {}: {e}", cache_dir.display())),
            )
        })?;
        let db_path = cache_dir.join("loupe-cache.sqlite");
        let conn = Connection::open(db_path)?;
        Self::init_conn(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory cache (tests / a degraded "no persistence" mode). Shares no state with
    /// any file; useful for the determinism unit tests that only need within-process reuse.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_conn(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// WAL + busy_timeout (M2) and the two tables. Idempotent (`IF NOT EXISTS`).
    fn init_conn(conn: &Connection) -> rusqlite::Result<()> {
        // WAL lets readers and a writer coexist; busy_timeout makes a contended open wait
        // (5s) instead of erroring. `query_row` because `journal_mode` returns the new mode.
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cluster_result (
                 repo_path      TEXT NOT NULL,
                 merge_base_sha TEXT NOT NULL,
                 card_hash      TEXT NOT NULL,
                 schema_ver     INTEGER NOT NULL,
                 result_json    TEXT NOT NULL,
                 created_at     INTEGER NOT NULL,
                 PRIMARY KEY (repo_path, merge_base_sha, card_hash, schema_ver)
             );
             CREATE TABLE IF NOT EXISTS review_layout (
                 repo_path      TEXT NOT NULL,
                 merge_base_sha TEXT NOT NULL,
                 head_sha       TEXT NOT NULL,
                 schema_ver     INTEGER NOT NULL,
                 layout_json    TEXT NOT NULL,
                 created_at     INTEGER NOT NULL,
                 PRIMARY KEY (repo_path, merge_base_sha, head_sha, schema_ver)
             );",
        )?;
        Ok(())
    }

    // ---- review_layout (head-grain, §8.1 결정성) --------------------------------------

    /// Look up the assembled layout for a head. A hit is the 5분→즉시 path (AI 0 calls).
    pub fn get_layout(
        &self,
        repo_path: &str,
        merge_base_sha: &str,
        head_sha: &str,
    ) -> Option<ClusterLayout> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let json: Option<String> = conn
            .query_row(
                "SELECT layout_json FROM review_layout
                 WHERE repo_path=?1 AND merge_base_sha=?2 AND head_sha=?3 AND schema_ver=?4",
                params![repo_path, merge_base_sha, head_sha, SCHEMA_VER],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        json.and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Store the assembled layout for a head (idempotent — `INSERT OR REPLACE`).
    pub fn put_layout(
        &self,
        repo_path: &str,
        merge_base_sha: &str,
        head_sha: &str,
        layout: &ClusterLayout,
    ) -> rusqlite::Result<()> {
        let json = serde_json::to_string(layout).map_err(serde_to_sqlite)?;
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO review_layout
                 (repo_path, merge_base_sha, head_sha, schema_ver, layout_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![repo_path, merge_base_sha, head_sha, SCHEMA_VER, json, now()],
        )?;
        Ok(())
    }

    // ---- cluster_result (seed-card grain, 부분 무효화) --------------------------------

    /// Look up the cached per-seed AI result by its `card_hash`. A hit ⇒ skip the AI for
    /// that seed even if `head` moved (its content is unchanged). Returns the stored
    /// per-seed [`ClusterLayout`] fragment.
    pub fn get_cluster(
        &self,
        repo_path: &str,
        merge_base_sha: &str,
        card_hash: &str,
    ) -> Option<ClusterLayout> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let json: Option<String> = conn
            .query_row(
                "SELECT result_json FROM cluster_result
                 WHERE repo_path=?1 AND merge_base_sha=?2 AND card_hash=?3 AND schema_ver=?4",
                params![repo_path, merge_base_sha, card_hash, SCHEMA_VER],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        json.and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Store a per-seed AI result keyed by its `card_hash` (idempotent).
    pub fn put_cluster(
        &self,
        repo_path: &str,
        merge_base_sha: &str,
        card_hash: &str,
        fragment: &ClusterLayout,
    ) -> rusqlite::Result<()> {
        let json = serde_json::to_string(fragment).map_err(serde_to_sqlite)?;
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT OR REPLACE INTO cluster_result
                 (repo_path, merge_base_sha, card_hash, schema_ver, result_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![repo_path, merge_base_sha, card_hash, SCHEMA_VER, json, now()],
        )?;
        Ok(())
    }
}

/// Compute the **card hash** of a single seed card (the 부분 무효화 key).
///
/// SHA-256 over a *normalized* JSON of the card with:
///  - `cluster_id` (= the volatile `"seed-N"` label) **excluded** — the seed index is an
///    ordering artefact, not content; two runs that produce the same content under a
///    different seed number must hash equal,
///  - every order-dependent array **sorted** (changed-symbol order, relation-hint pairs,
///    entrypoints, contracts, tests, signals) so re-discovery order can't perturb the hash,
///  - `SCHEMA_VER` mixed in, so a prompt/schema bump invalidates every card hash.
///
/// The card hashes therefore depend only on the seed's *content* (its symbols' ids, kinds,
/// change types, summaries, signals, and intra-seed relations), making the cache key stable
/// across runs (결정성) and across head changes that don't touch the seed (부분 무효화).
pub fn card_hash(card: &ClusterCardInput) -> String {
    let normalized = normalize_card(card);
    let mut hasher = Sha256::new();
    hasher.update(b"loupe-card-v");
    hasher.update(SCHEMA_VER.to_le_bytes());
    hasher.update(b"\0");
    // `to_string` over a normalized `Value` is deterministic: serde_json sorts nothing on
    // its own, but every array we built is pre-sorted and objects are emitted in our fixed
    // field order, so the byte stream is stable.
    hasher.update(serde_json::to_string(&normalized).unwrap_or_default().as_bytes());
    let digest = hasher.finalize();
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Build the normalized `Value` hashed by [`card_hash`]: drop `clusterId`, sort every
/// order-dependent array. Pure (no I/O).
fn normalize_card(card: &ClusterCardInput) -> Value {
    // Serialize the card to JSON, then post-process: it already serializes camelCase.
    let mut v = serde_json::to_value(card).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        // `clusterId` is the volatile seed label — exclude it entirely.
        obj.remove("clusterId");

        // Sort each order-dependent array. Changed symbols are sorted by cardId (their
        // stable identity); the string lists and hint-pair arrays sort lexicographically.
        sort_array_by_key(obj.get_mut("changedSymbols"), "cardId");
        sort_string_array(obj.get_mut("entrypointCandidates"));
        sort_string_array(obj.get_mut("contractsChanged"));
        sort_string_array(obj.get_mut("relatedTests"));
        sort_array_by_key(obj.get_mut("deletedSymbols"), "id");
        sort_array_by_key(obj.get_mut("renamePairs"), "toCardId");
        sort_array_by_key(obj.get_mut("signatureChanges"), "cardId");

        // relationHints: { strong: [[a,b]], weak: [[a,b]] } — sort each pair list.
        if let Some(hints) = obj.get_mut("relationHints").and_then(Value::as_object_mut) {
            sort_pair_array(hints.get_mut("strong"));
            sort_pair_array(hints.get_mut("weak"));
        }
    }
    v
}

/// Sort a JSON string array in place (lexicographic). No-op on a non-array.
fn sort_string_array(v: Option<&mut Value>) {
    if let Some(Value::Array(a)) = v {
        a.sort_by(|x, y| {
            x.as_str().unwrap_or_default().cmp(y.as_str().unwrap_or_default())
        });
    }
}

/// Sort a JSON array of objects by a string field `key` in place. No-op on a non-array.
fn sort_array_by_key(v: Option<&mut Value>, key: &str) {
    if let Some(Value::Array(a)) = v {
        a.sort_by(|x, y| {
            let kx = x.get(key).and_then(Value::as_str).unwrap_or_default();
            let ky = y.get(key).and_then(Value::as_str).unwrap_or_default();
            // Stable, total order: serialize the whole element as the tiebreaker so two
            // elements with the same key never compare equal non-deterministically.
            kx.cmp(ky).then_with(|| x.to_string().cmp(&y.to_string()))
        });
    }
}

/// Sort a JSON array of `[a, b]` pairs in place (by a then b). No-op on a non-array.
fn sort_pair_array(v: Option<&mut Value>) {
    if let Some(Value::Array(a)) = v {
        a.sort_by(|x, y| x.to_string().cmp(&y.to_string()));
    }
}

/// Map a serde_json error to a rusqlite error (so `put_*` returns a single error type).
fn serde_to_sqlite(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

/// Unix-seconds timestamp (best-effort; 0 if the clock is before the epoch).
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
