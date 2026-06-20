mod engine;

use tauri::{AppHandle, Emitter, Manager};

/// Validate a model setup-token, returning the trimmed value on success.
///
/// A `claude setup-token` is a single ASCII string with no spaces (sk-ant-oat01-…). A value
/// with whitespace or non-ASCII characters is a paste error (e.g. a card summary landed in
/// the token field). Empty is rejected too. Shared by `analyze_review` (so a bad value never
/// reaches `CLAUDE_CODE_OAUTH_TOKEN` and surfaces as a cryptic CLI "invalid header value")
/// and by `save_token` (so we never persist garbage). The token itself is **never logged**.
fn validate_token(token: &str) -> Result<String, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("missing model token — finish onboarding first".to_string());
    }
    if token.chars().any(|c| c.is_whitespace() || !c.is_ascii()) {
        return Err(
            "model token looks invalid (it contains spaces or non-ASCII characters). Re-run \
             `claude setup-token` and paste the sk-ant-… value into onboarding."
                .to_string(),
        );
    }
    Ok(token)
}

/// `<app_data_dir>/loupe`, the same directory `analyze_review` uses for its SQLite cache.
fn loupe_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data dir: {e}"))?
        .join("loupe"))
}

/// Plaintext file holding the persisted model setup-token (unix perms 0o600).
fn token_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(loupe_dir(app)?.join("model-token"))
}

/// Persist the model setup-token as plaintext at `<app_data_dir>/loupe/model-token`.
///
/// The token is validated first (same rules as `analyze_review`); an invalid value is rejected
/// rather than written. On unix the file is `chmod 0o600` (owner read/write only) — best-effort:
/// if `set_permissions` fails (e.g. an exotic filesystem), the token stays written and we only
/// log that the permission tightening did not apply. The token itself is **never logged**.
#[tauri::command]
fn save_token(app: AppHandle, token: String) -> Result<(), String> {
    let token = validate_token(&token)?;

    let dir = loupe_dir(&app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create token directory: {e}"))?;
    let path = dir.join("model-token");
    std::fs::write(&path, &token).map_err(|e| format!("could not write token: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            // Best-effort: the token is already written; only the chmod failed. Never log the
            // token, only the perms error.
            eprintln!("warning: could not set 0o600 on token file: {e}");
        }
    }

    Ok(())
}

/// Load the persisted model setup-token. Returns `None` when no token file exists (fresh
/// install / after `clear_token`), `Some(trimmed)` when present, and `Err` only on a real read
/// failure. The token itself is **never logged**.
#[tauri::command]
fn load_token(app: AppHandle) -> Result<Option<String>, String> {
    let path = token_path(&app)?;
    match std::fs::read_to_string(&path) {
        Ok(contents) => Ok(Some(contents.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read token: {e}")),
    }
}

/// Remove the persisted model setup-token. A missing file is treated as success (idempotent).
#[tauri::command]
fn clear_token(app: AppHandle) -> Result<(), String> {
    let path = token_path(&app)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not clear token: {e}")),
    }
}

/// Verify a model setup-token by making one minimal live call through the `claude` CLI.
///
/// `validate_token` runs first (whitespace / non-ASCII / empty rejected, same rules as
/// `save_token` / `analyze_review`). On success we build a `CliProvider` and issue a single
/// tiny `Fast` (Haiku) completion — "reply with OK" — so a bad/expired token surfaces here as a
/// human-readable error instead of mid-pipeline. `max_tokens` is small and `json_schema` is
/// `None` to keep the round-trip cheap. The token is moved straight into the provider and
/// **never logged** (the provider passes it to the child via env, never on argv). Returns
/// `Ok(())` on any successful completion; the actual answer text is irrelevant.
#[tauri::command]
async fn verify_token(token: String) -> Result<(), String> {
    use engine::ai::{CompletionRequest, LlmProvider, ModelTier};

    let token = validate_token(&token)?;

    let provider = engine::ai::cli::CliProvider::new(token);

    let req = CompletionRequest {
        system: "reply with OK".to_string(),
        user: "ping".to_string(),
        max_tokens: 16,
        json_schema: None,
        tier: ModelTier::Fast,
        temperature: 0.0,
    };

    // Map the coarse LlmError to a human-readable string. The token is never part of the
    // error (LlmError carries only status/body snippets, never credentials).
    provider
        .complete(req)
        .await
        .map(|_| ())
        .map_err(|e| format!("could not reach the model: {e}"))
}

/// One prior message in a thread conversation, as supplied by the front-end. `author` is the
/// literal `"you"` (the human) or `"ai"` (a previous assistant reply); any other value is
/// treated as the human side when rendering the transcript. Tauri deserialises the JS array
/// `[{author, text}]` into `Vec<ThreadTurn>` (fields are already snake_case-free, so no rename
/// is needed).
#[derive(serde::Deserialize)]
struct ThreadTurn {
    author: String,
    text: String,
}

/// Ask a free-text follow-up question about a selected region of a diff.
///
/// The front-end calls `invoke('ask_thread', { token, context, question, history })`. `context`
/// is the front-end-assembled string (card symbol/path + a windowed diff excerpt + a "user is
/// asking about line N (side)" marker); `question` is the user's current message; `history` is
/// the prior turns of this thread (this question excluded). The reply is a plain string (the
/// model's natural-language answer).
///
/// Auth mirrors `analyze_review` / `verify_token`: `validate_token` runs first (whitespace /
/// non-ASCII / empty rejected, token never logged), then a `CliProvider` (Sonnet via the
/// `claude` CLI) is built with the moved-in token. We use the `Quality` tier because answer
/// quality matters more than latency here, and `json_schema: None` so the model returns free
/// text. `temperature: 0.3` keeps answers grounded but not robotic.
///
/// Extraction: with no schema, `CliProvider` puts the model's text in `CompletionResponse.json`
/// as a `Value::String` (the free-text fallback in `cli::parse_wrapper`). We return that string
/// directly; on the off chance the model emitted JSON, we hand back its compact serialisation so
/// the caller still gets the text rather than an error.
#[tauri::command]
async fn ask_thread(
    token: String,
    context: String,
    question: String,
    history: Vec<ThreadTurn>,
) -> Result<String, String> {
    use engine::ai::{CompletionRequest, LlmProvider, ModelTier};

    let token = validate_token(&token)?;
    let provider = engine::ai::cli::CliProvider::new(token);

    let system = "You are a senior engineer reviewing a code change together with a user. \
         The user has selected one region of a diff and is asking about it. Answer concisely and \
         concretely, grounded in the provided diff. Reply in the user's language (if the question \
         is in Korean, answer in Korean). If you do not know, say so."
        .to_string();

    // user message = front-end context, then the prior transcript (author → User/Assistant),
    // then the current question last.
    let mut user = context;
    user.push_str("\n\n--- Conversation so far ---\n");
    for turn in &history {
        let role = if turn.author == "ai" {
            "Assistant"
        } else {
            "User"
        };
        user.push_str(role);
        user.push_str(": ");
        user.push_str(&turn.text);
        user.push('\n');
    }
    user.push_str("\nUser: ");
    user.push_str(&question);

    let req = CompletionRequest {
        system,
        user,
        max_tokens: 1024,
        json_schema: None,
        tier: ModelTier::Quality,
        temperature: 0.3,
    };

    let resp = provider
        .complete(req)
        .await
        .map_err(|e| format!("could not reach the model: {e}"))?;

    // Free-text path: parse_wrapper stores plain text as Value::String. Return it directly;
    // if the model returned structured JSON instead, hand back its compact serialisation.
    let text = match resp.json {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    Ok(text)
}

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
    let token = validate_token(&token)?;

    // <app_data_dir>/loupe (created on demand by the cache layer).
    let cache_dir = loupe_dir(&app)?;

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
            ask_thread,
            list_branches,
            verify_token,
            save_token,
            load_token,
            clear_token
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
