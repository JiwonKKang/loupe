mod engine;

use tauri::{AppHandle, Emitter, Manager};

/// Re-emits engine pipeline milestones over the `analyze://progress` Tauri event so the
/// front-end `AnalyzeScreen` can mirror the real stages live. Holds a clone of the app handle;
/// `emit` is best-effort (a dropped event only affects the loader, never the result).
struct TauriProgress(AppHandle);

impl engine::ProgressSink for TauriProgress {
    fn emit(&self, event: engine::Progress) {
        let _ = self.0.emit("analyze://progress", event);
    }
}

/// Build the review payload for `base...target` in the repo at `repo_path`.
/// The front-end calls this via `invoke('load_review', { repoPath, base, target })`.
#[tauri::command]
fn load_review(
    repo_path: String,
    base: String,
    target: String,
) -> Result<engine::ReviewData, String> {
    engine::build_review(&repo_path, &base, &target).map_err(|e| e.to_string())
}

/// ⑧ — the full Stage-1 + Stage-2 (AI cluster) payload for the review screen.
///
/// The front-end calls this in the background right after `load_review` (which gives the flat
/// cards instantly): `invoke('analyze_review', { repoPath, base, target, token })`. A cache hit
/// returns immediately (same head ⇒ same order, AI 0 calls); a miss runs the AI pipeline (~몇
/// 분) while the front-end shows the cluster-analysis loader.
///
/// Auth: the `token` is the onboarding setup-token (`claude setup-token`), used to build the
/// `CliProvider` (Sonnet via the `claude` CLI). It is moved straight into the provider and
/// **never logged** (the provider passes it to the child via env, never on argv). When empty,
/// we error out rather than shelling out unauthenticated.
///
/// Cache: `analyze_review` opens its SQLite cache under `<app_data_dir>/loupe` (resolved from
/// the Tauri path API), so results persist across app restarts.
#[tauri::command]
async fn analyze_review(
    app: AppHandle,
    repo_path: String,
    base: String,
    target: String,
    token: String,
) -> Result<engine::ReviewData, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("missing model token — finish onboarding first".to_string());
    }
    // A `claude setup-token` is a single ASCII string with no spaces (sk-ant-oat01-…). A value
    // with whitespace or non-ASCII characters is a paste error (e.g. a card summary landed in
    // the token field). Catch it here with an actionable message — otherwise the bad value
    // reaches `CLAUDE_CODE_OAUTH_TOKEN` and surfaces as a cryptic CLI "invalid header value".
    if token.chars().any(|c| c.is_whitespace() || !c.is_ascii()) {
        return Err(
            "model token looks invalid (it contains spaces or non-ASCII characters). Re-run \
             `claude setup-token` and paste the sk-ant-… value into onboarding."
                .to_string(),
        );
    }

    // <app_data_dir>/loupe (created on demand by the cache layer).
    let cache_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data dir: {e}"))?
        .join("loupe");

    // CliProvider holds the setup-token (Sonnet via the `claude` CLI). `token` is moved in and
    // never logged. The trait object is what the engine consumes.
    let provider = engine::ai::cli::CliProvider::new(token);

    // Stream pipeline milestones to the loader over `analyze://progress`.
    let progress = TauriProgress(app.clone());

    engine::analyze_review(&provider, &cache_dir, &repo_path, &base, &target, &progress)
        .await
        .map_err(|e| e.to_string())
}

/// Local branches of the repo at `repo_path`, for the onboarding range picker.
/// The front-end calls this via `invoke('list_branches', { repoPath })` once a
/// folder is chosen, then fills the base/target dropdowns from the result.
#[derive(serde::Serialize)]
struct BranchList {
    branches: Vec<String>,
    current: Option<String>,
    default: Option<String>,
}

#[tauri::command]
fn list_branches(repo_path: String) -> Result<BranchList, String> {
    let b = engine::list_branches(&repo_path).map_err(|e| e.to_string())?;
    Ok(BranchList {
        branches: b.branches,
        current: b.current,
        default: b.default,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_review,
            analyze_review,
            list_branches
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
