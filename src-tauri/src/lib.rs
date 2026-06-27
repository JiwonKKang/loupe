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

/// JSON file holding persisted threads, keyed by `(repo_path, base, target)`.
///
/// Stored at `<app_data_dir>/loupe/threads.json` as a single JSON object `{ key: [...] }`.
/// This lives alongside the SQLite analysis cache but is **independent** of it: clearing the
/// cache (re-running the AI pipeline) does not touch threads, and vice versa.
fn threads_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(loupe_dir(app)?.join("threads.json"))
}

/// Derive the storage key for a thread bucket from the three identifying parts.
///
/// The parts are joined with the ASCII Unit Separator (`U+001F`) — a control character that
/// cannot appear in a branch name or a filesystem path, so distinct `(repo_path, base, target)`
/// triples can never collide into the same key.
fn threads_key(repo_path: &str, base: &str, target: &str) -> String {
    format!("{repo_path}\u{1f}{base}\u{1f}{target}")
}

/// Read a `threads.json` file into a JSON object, degrading to an empty object on any problem.
///
/// A missing file (fresh install), an unreadable file, or corrupt / non-object JSON all map to
/// an empty `Map` rather than an error: threads are a convenience layer, so a damaged file must
/// never panic or block the app — it simply reads back as "no saved threads". Pure (path in,
/// map out) so it is unit-testable without a Tauri `AppHandle`.
fn read_threads_object_at(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return serde_json::Map::new(),
    };
    match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// `read_threads_object_at` resolved against the app's `threads.json`. A path-resolution
/// failure also degrades to an empty object (threads must never block the app).
fn read_threads_object(app: &AppHandle) -> serde_json::Map<String, serde_json::Value> {
    match threads_path(app) {
        Ok(p) => read_threads_object_at(&p),
        Err(_) => serde_json::Map::new(),
    }
}

/// Look up a bucket and render it as the `load_threads` return string (`"[]"` when absent).
fn load_threads_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> String {
    match obj.get(key) {
        Some(value) => value.to_string(),
        None => "[]".to_string(),
    }
}

/// Load the persisted thread array for `(repo_path, base, target)`.
///
/// The front-end calls `invoke('load_threads', { repoPath, base, target })` (Tauri maps the
/// camelCase `repoPath` to `repo_path`). Returns the bucket's value re-serialised as a JSON
/// **string** (the front-end `JSON.parse`s it), or the literal `"[]"` when the file is missing,
/// corrupt, or has no entry for this key. Never panics: a broken file degrades to `"[]"`.
#[tauri::command]
fn load_threads(
    app: AppHandle,
    repo_path: String,
    base: String,
    target: String,
) -> Result<String, String> {
    let obj = read_threads_object(&app);
    let key = threads_key(&repo_path, &base, &target);
    Ok(load_threads_string(&obj, &key))
}

/// Persist the thread array for `(repo_path, base, target)` to `threads.json`.
///
/// The front-end calls `invoke('save_threads', { repoPath, base, target, threads })` where
/// `threads` is `JSON.stringify(threadsArray)`. We parse `threads` and require it to be a JSON
/// **array** (anything else is a caller bug ⇒ `Err`), then merge it into the existing object
/// under this key (other keys are preserved) and write the whole object back.
///
/// The write is **atomic**: we write a sibling temp file and `rename` it over `threads.json`, so
/// a crash mid-write can never leave a half-written / corrupt file. On unix the temp file is
/// `chmod 0o600` before the rename (best-effort, same pattern as `save_token`).
#[tauri::command]
fn save_threads(
    app: AppHandle,
    repo_path: String,
    base: String,
    target: String,
    threads: String,
) -> Result<(), String> {
    let dir = loupe_dir(&app)?;
    let key = threads_key(&repo_path, &base, &target);
    save_threads_in_dir(&dir, &key, &threads)
}

/// Core of `save_threads`, taking the `loupe` directory + derived key directly so it is
/// unit-testable without a Tauri `AppHandle`. Validates `threads` is a JSON array, merges it
/// into the existing object under `key` (preserving other buckets), and writes the whole object
/// back atomically (temp file + rename, 0o600 on unix).
fn save_threads_in_dir(dir: &std::path::Path, key: &str, threads: &str) -> Result<(), String> {
    // Validate the payload is a JSON array before touching disk.
    let parsed: serde_json::Value =
        serde_json::from_str(threads).map_err(|e| format!("threads is not valid JSON: {e}"))?;
    if !parsed.is_array() {
        return Err("threads must be a JSON array".to_string());
    }

    std::fs::create_dir_all(dir)
        .map_err(|e| format!("could not create threads directory: {e}"))?;

    let path = dir.join("threads.json");

    // Read-modify-write the whole object, preserving other buckets.
    let mut obj = read_threads_object_at(&path);
    obj.insert(key.to_string(), parsed);

    let serialized = serde_json::Value::Object(obj).to_string();

    // Atomic write: temp file in the same dir + rename. The temp name is unique per-pid to avoid
    // clobbering a concurrent writer's temp file.
    let tmp = dir.join(format!("threads.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &serialized)
        .map_err(|e| format!("could not write threads temp file: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)) {
            // Best-effort: the data is written; only the chmod failed.
            eprintln!("warning: could not set 0o600 on threads file: {e}");
        }
    }

    if let Err(e) = std::fs::rename(&tmp, &path) {
        // Clean up the temp file so a failed rename doesn't leave litter behind.
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not persist threads: {e}"));
    }

    Ok(())
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

    // Verify against the SAME path the analysis pipeline uses — Haiku via the direct HTTP
    // Messages API (OAuthProvider). Fast tier ⇒ Haiku (Sonnet would 429 on the direct API).
    let provider = engine::ai::anthropic::OAuthProvider::new(token);

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

/// Ask a **codebase-aware** follow-up question about a selected region of a diff.
///
/// The front-end calls
/// `invoke('ask_thread', { token, repoPath, context, question, history, model })`
/// (Tauri maps the camelCase `repoPath` to `repo_path`, same as `load_review` / `analyze_review`).
/// `context` is the front-end-assembled string (card symbol/path + a windowed diff excerpt + a
/// "user is asking about line N (side)" marker); `question` is the user's current message;
/// `history` is the prior turns of this thread (this question excluded); `model` is the per-thread
/// model choice (`"sonnet"` | `"haiku"`, default Sonnet). The reply is a plain string (the agent's
/// natural-language answer).
///
/// `model` is mapped to a concrete CLI model id — `"haiku"` ⇒ `MODEL_FAST_ID` (`claude-haiku-4-5`),
/// anything else (including `"sonnet"`, empty, or unknown) ⇒ `MODEL_QUALITY_ID`
/// (`claude-sonnet-4-6`) — then passed to `ask_agentic`. The agentic loop is turn-capped there.
///
/// Unlike the old prompt-only path (which saw *only* the diff), this runs the `claude` CLI as a
/// **read-only agent** in the repo: it can open real files, grep for definitions/callers, and
/// answer grounded in the actual codebase — not just the diff excerpt. See `cli::ask_agentic`
/// for the safety model (read-only allowlist Read/Grep/Glob/LS, write/exec tools denied, cwd
/// confined to `repo_path`, 180 s timeout).
///
/// Auth mirrors `analyze_review` / `verify_token`: `validate_token` runs first (whitespace /
/// non-ASCII / empty rejected, token **never logged**), then the token is moved into
/// `ask_agentic` which passes it to the child via env only (never on argv).
#[tauri::command]
async fn ask_thread(
    token: String,
    repo_path: String,
    context: String,
    question: String,
    history: Vec<ThreadTurn>,
    model: String,
) -> Result<String, String> {
    let token = validate_token(&token)?;

    // Map the per-thread model choice to a concrete CLI model id. Only "haiku" selects the fast
    // model; everything else (sonnet / empty / unknown) falls back to the quality model.
    let model_id = match model.as_str() {
        "haiku" => engine::ai::cli::MODEL_FAST_ID,
        _ => engine::ai::cli::MODEL_QUALITY_ID,
    };

    // Prompt: tell the agent it is reviewing a real repo, give it the diff context + the user's
    // selected question, and invite it to read the relevant code (definitions / callers / related
    // files) so the answer is grounded in the actual codebase. Prior turns follow as a transcript.
    let mut prompt = String::from(
        "You are a senior engineer pair-reviewing a code change in THIS repository with a user. \
         Below is the change context (file / symbol / diff), the SPECIFIC region the user selected \
         in the diff, and their question about it. Answer their question — whatever it is — but \
         keep your answer anchored to the SELECTED region: resolve a vague 'this' / '이거' / 'here' \
         to that selected code, and don't drift onto unrelated parts of the repo unless the \
         question clearly calls for it. When it helps, read the relevant code in this repo directly \
         (definitions, callers, related files) so the answer is grounded in the ACTUAL codebase — \
         not just the diff excerpt. \
         \n\nJUMP LINKS (optional, only when natural): the context ends with a numbered list \
         '점프 가능한 리뷰 카드' — the review cards in THIS change. Answer the question normally \
         first. If — and only if — a place you genuinely need to mention happens to be one of \
         those cards, you MAY make it clickable as a Markdown link [표시 텍스트](loupe-card:N) (N = \
         its number in the list). Do NOT force references or bend your answer to fit the cards. \
         When a caller / usage / related piece of code is NOT among these cards (i.e. not part of \
         this diff), just say so plainly in prose — e.g. '이번 diff엔 없지만 `AuthService.login`에서 \
         호출됩니다' — with no link. Most answers need few or no links; that is fine. \
         \n\nReply in the user's language (if the question is in Korean, answer in Korean). Be \
         concise and concrete. If you genuinely do not know, say so.\n\n",
    );
    prompt.push_str("--- Change context ---\n");
    prompt.push_str(&context);
    prompt.push_str("\n\n--- Conversation so far ---\n");
    for turn in &history {
        let role = if turn.author == "ai" {
            "Assistant"
        } else {
            "User"
        };
        prompt.push_str(role);
        prompt.push_str(": ");
        prompt.push_str(&turn.text);
        prompt.push('\n');
    }
    prompt.push_str("\nUser: ");
    prompt.push_str(&question);

    engine::ai::cli::ask_agentic(token, repo_path, prompt, model_id, None).await
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

    // Cluster + order + labels run on Haiku via the direct HTTP Messages API (OAuthProvider):
    // ~10x faster than the `claude` CLI. The change-unit (review) step instead runs on Sonnet via
    // the CLI — its grouping needs the stronger model — so the same `token` is also passed through
    // for `analyze_review` to build a `CliProvider`. Never logged (providers pass it via env/once).
    let provider = engine::ai::anthropic::OAuthProvider::new(token.clone());

    // Stream pipeline milestones to the loader over `analyze://progress`.
    let progress = TauriProgress(app.clone());

    engine::analyze_review(&provider, &token, &cache_dir, &repo_path, &base, &target, &progress)
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

// `async` + `spawn_blocking`: a *synchronous* Tauri command runs on the main
// (UI) thread, so opening the repo + walking its refs would freeze the webview
// for the duration — right after the user picks a folder, before the
// "Reading branches…" indicator can even paint. git2 is blocking, so we hop it
// onto the blocking pool and keep the UI thread free.
#[tauri::command]
async fn list_branches(repo_path: String) -> Result<BranchList, String> {
    tokio::task::spawn_blocking(move || {
        let b = engine::list_branches(&repo_path).map_err(|e| e.to_string())?;
        Ok::<BranchList, String>(BranchList {
            branches: b.branches,
            current: b.current,
            default: b.default,
        })
    })
    .await
    .map_err(|e| format!("branch scan task panicked: {e}"))?
}

/// Open the analyzed project in an external editor (IntelliJ / VS Code) and jump to
/// `file:line` — for when the diff alone isn't enough and you want the whole file in
/// context. We open the PROJECT (`repo_path`), not just the file, then navigate.
///
/// `editor`: "idea" | "code" | "auto" (auto tries `code`, then `idea`). We try the CLI
/// launcher first (`code` / `idea` on the augmented PATH), then fall back to the binary
/// INSIDE an installed `.app` bundle — so it works even when the user never installed the
/// shell-command launcher. `code <project> --goto <file>:<line>` opens the folder window
/// and reveals the line; `idea --line <line> <file>` opens the file's project and navigates.
#[tauri::command]
fn open_in_editor(
    editor: String,
    repo_path: String,
    file: String,
    line: u32,
) -> Result<(), String> {
    let abs = std::path::Path::new(&repo_path)
        .join(&file)
        .to_string_lossy()
        .to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    let code_args = vec![repo_path.clone(), "--goto".to_string(), format!("{abs}:{line}")];
    let idea_args = vec!["--line".to_string(), line.to_string(), abs.clone()];

    // (binary, args) candidates, launcher first then bundled-app binaries.
    let code_bins: Vec<String> = vec![
        "code".into(),
        "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code".into(),
        format!("{home}/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"),
        "/Applications/VSCodium.app/Contents/Resources/app/bin/codium".into(),
        "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code-insiders".into(),
    ];
    let idea_bins: Vec<String> = vec![
        "idea".into(),
        "/Applications/IntelliJ IDEA.app/Contents/MacOS/idea".into(),
        "/Applications/IntelliJ IDEA CE.app/Contents/MacOS/idea".into(),
        "/Applications/IntelliJ IDEA Ultimate.app/Contents/MacOS/idea".into(),
        format!("{home}/Applications/IntelliJ IDEA.app/Contents/MacOS/idea"),
        format!("{home}/Applications/IntelliJ IDEA CE.app/Contents/MacOS/idea"),
    ];
    let path = engine::ai::cli::augmented_path();
    let launch = |bin: &str, args: &[String]| -> bool {
        std::process::Command::new(bin)
            .args(args)
            .env("PATH", &path)
            .spawn()
            .is_ok()
    };
    // VS Code: `code <project> --goto <file>:<line>` opens the folder AND navigates.
    let open_code = || -> bool { code_bins.iter().any(|b| launch(b, &code_args)) };
    // IntelliJ: a single `idea --line N <file>` only opens the lone file — so open
    // the PROJECT dir first, then navigate to file:line (best-effort) in it.
    let open_idea = || -> bool {
        for b in &idea_bins {
            if launch(b, std::slice::from_ref(&repo_path)) {
                let _ = launch(b, &idea_args);
                return true;
            }
        }
        false
    };
    let opened = match editor.as_str() {
        "idea" => open_idea(),
        "code" => open_code(),
        _ => open_code() || open_idea(), // auto
    };
    if opened {
        return Ok(());
    }
    let which = match editor.as_str() {
        "idea" => "IntelliJ IDEA",
        "code" => "VS Code",
        _ => "에디터",
    };
    Err(format!(
        "{which}를 찾을 수 없어요 — 앱이 설치돼 있는지, 또는 CLI 런처가 있는지 확인하세요 \
         (VS Code: ⌘⇧P → 'Shell Command: Install code', IntelliJ: Toolbox → Settings → 'Shell scripts')."
    ))
}

// --- GitHub PR approval (summary screen, all-pass) -------------------------------
// Loupe makes no network calls of its own — PR approval is delegated to the user's
// `gh` CLI (same model as the `claude` CLI), so the app never handles GitHub creds.
// `gh` infers the repo from the repo's remote because we run it with cwd = repo_path.

#[derive(serde::Serialize)]
struct PrInfo {
    number: u64,
    url: String,
    state: String,
    title: String,
}

/// Run `gh` in `repo_path` with the GUI-augmented PATH (so it resolves when the app
/// was launched from Finder/Spotlight). Maps the "gh not installed" spawn error to a
/// friendly message; the caller inspects the exit status/stderr for the rest.
fn run_gh(repo_path: &str, args: &[&str]) -> Result<std::process::Output, String> {
    std::process::Command::new("gh")
        .args(args)
        .current_dir(repo_path)
        .env("PATH", engine::ai::cli::augmented_path())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "GitHub CLI(gh)를 찾을 수 없어요 — `brew install gh` 후 `gh auth login` 하세요.".into()
            } else {
                format!("gh 실행 실패: {e}")
            }
        })
}

/// Strip a remote prefix (`origin/`, `upstream/`, …) from a branch so `gh` matches the
/// PR's HEAD branch name. The review can be run against remote-tracking refs (e.g.
/// `origin/feature-x`), but a PR's head branch on GitHub is the plain name
/// (`feature-x`). We only strip a leading segment that is an ACTUAL remote, so a real
/// branch like `feature/x` is left intact.
fn pr_branch_name(repo_path: &str, branch: &str) -> String {
    if let Ok(out) = std::process::Command::new("git")
        .args(["-C", repo_path, "remote"])
        .env("PATH", engine::ai::cli::augmented_path())
        .output()
    {
        if out.status.success() {
            for r in String::from_utf8_lossy(&out.stdout).lines() {
                let r = r.trim();
                if !r.is_empty() {
                    if let Some(rest) = branch.strip_prefix(&format!("{r}/")) {
                        return rest.to_string();
                    }
                }
            }
        }
    }
    branch.to_string()
}

/// Translate a non-zero `gh` stderr into a specific, friendly message.
fn gh_error(stderr: &str) -> String {
    let s = stderr.to_lowercase();
    if s.contains("gh auth login") || s.contains("not logged") || s.contains("authentication") {
        "GitHub CLI에 로그인돼 있지 않아요 — 터미널에서 `gh auth login` 후 다시 시도하세요.".into()
    } else if s.contains("no git remotes") || s.contains("none of the git remotes") {
        "이 저장소에 GitHub 원격(remote)이 없어요.".into()
    } else {
        format!("gh 오류: {}", stderr.trim())
    }
}

fn parse_pr(stdout: &str) -> Result<PrInfo, String> {
    let v: serde_json::Value =
        serde_json::from_str(stdout).map_err(|e| format!("gh 응답 파싱 실패: {e}"))?;
    Ok(PrInfo {
        number: v.get("number").and_then(|x| x.as_u64()).unwrap_or(0),
        url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        state: v.get("state").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
    })
}

fn is_no_pr(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("no pull requests found") || s.contains("no open pull requests")
}

/// The PR (if any) for the reviewed branch. `Ok(None)` = no PR for this branch, so the
/// UI hides the Approve action. Used to gate + label the summary-screen Approve button.
#[tauri::command]
async fn pr_status(repo_path: String, target: String) -> Result<Option<PrInfo>, String> {
    tokio::task::spawn_blocking(move || {
        let target = pr_branch_name(&repo_path, &target); // drop any origin/ prefix for gh
        let out = run_gh(&repo_path, &["pr", "view", &target, "--json", "number,url,state,title"])?;
        if out.status.success() {
            return Ok(Some(parse_pr(&String::from_utf8_lossy(&out.stdout))?));
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        if is_no_pr(&stderr) {
            Ok(None)
        } else {
            Err(gh_error(&stderr))
        }
    })
    .await
    .map_err(|e| format!("PR 조회 작업이 패닉했어요: {e}"))?
}

/// Approve the GitHub PR for `target` via `gh pr review <branch> --approve`. Validates
/// the PR first (so the message names it and merged/closed PRs are rejected). This is
/// ONLY ever invoked by an explicit user click on the all-pass summary screen.
#[tauri::command]
async fn approve_pr(
    repo_path: String,
    target: String,
    body: Option<String>,
) -> Result<PrInfo, String> {
    tokio::task::spawn_blocking(move || {
        let target = pr_branch_name(&repo_path, &target); // drop any origin/ prefix for gh
        // Resolve + validate the PR first — precise errors + the info the UI confirmed.
        let view = run_gh(&repo_path, &["pr", "view", &target, "--json", "number,url,state,title"])?;
        if !view.status.success() {
            let stderr = String::from_utf8_lossy(&view.stderr);
            if is_no_pr(&stderr) {
                return Err(format!("이 브랜치({target})에 열린 PR이 없어요."));
            }
            return Err(gh_error(&stderr));
        }
        let pr = parse_pr(&String::from_utf8_lossy(&view.stdout))?;
        match pr.state.as_str() {
            "MERGED" => return Err(format!("이미 머지된 PR이에요 (#{}) — 승인할 수 없어요.", pr.number)),
            "CLOSED" => return Err(format!("닫힌 PR이에요 (#{}).", pr.number)),
            _ => {}
        }
        let body_str = body.unwrap_or_default();
        let body_trimmed = body_str.trim();
        let mut args: Vec<&str> = vec!["pr", "review", &target, "--approve"];
        if !body_trimmed.is_empty() {
            args.push("--body");
            args.push(body_trimmed);
        }
        let out = run_gh(&repo_path, &args)?;
        if out.status.success() {
            return Ok(pr);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let s = stderr.to_lowercase();
        if s.contains("already approved") {
            return Ok(pr); // idempotent — already approved reads as success
        }
        if s.contains("approve your own") {
            return Err("본인이 올린 PR은 승인할 수 없어요.".into());
        }
        Err(gh_error(&stderr))
    })
    .await
    .map_err(|e| format!("PR 승인 작업이 패닉했어요: {e}"))?
}

/// Post a LINE-ANCHORED review comment on the open PR for `target`, at `file:line` on
/// the new-file (RIGHT) side. gh has no line-comment CLI, so we POST via `gh api` to the
/// pulls/comments endpoint with the PR head commit. ONLY invoked by an explicit user
/// click in a thread while an OPEN PR exists. Returns the created comment's html_url.
#[tauri::command]
async fn pr_comment(
    repo_path: String,
    target: String,
    file: String,
    line: u32,
    body: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let target = pr_branch_name(&repo_path, &target);
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err("댓글 내용이 비어 있어요.".into());
        }
        // owner/repo for the gh api path (gh infers it from the repo's remote).
        let nwo_out = run_gh(&repo_path, &["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])?;
        if !nwo_out.status.success() {
            return Err(gh_error(&String::from_utf8_lossy(&nwo_out.stderr)));
        }
        let nwo = String::from_utf8_lossy(&nwo_out.stdout).trim().to_string();
        if nwo.is_empty() {
            return Err("GitHub 저장소를 확인할 수 없어요.".into());
        }
        // The PR number, state and head commit to anchor the comment to.
        let view = run_gh(&repo_path, &["pr", "view", &target, "--json", "number,state,headRefOid"])?;
        if !view.status.success() {
            let stderr = String::from_utf8_lossy(&view.stderr);
            if is_no_pr(&stderr) {
                return Err(format!("이 브랜치({target})에 열린 PR이 없어요."));
            }
            return Err(gh_error(&stderr));
        }
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&view.stdout))
            .map_err(|e| format!("gh 응답 파싱 실패: {e}"))?;
        let number = v.get("number").and_then(|x| x.as_u64()).unwrap_or(0);
        let state = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let commit = v.get("headRefOid").and_then(|x| x.as_str()).unwrap_or("");
        if state != "OPEN" {
            return Err(format!("열린 PR이 아니에요 (#{number}, {state})."));
        }
        if commit.is_empty() {
            return Err("PR의 커밋(SHA)을 확인할 수 없어요.".into());
        }
        let endpoint = format!("repos/{nwo}/pulls/{number}/comments");
        let body_f = format!("body={body}");
        let commit_f = format!("commit_id={commit}");
        let path_f = format!("path={file}");
        let line_f = format!("line={}", line.max(1));
        let out = run_gh(&repo_path, &[
            "api", "-X", "POST", &endpoint,
            "-f", &body_f, "-f", &commit_f, "-f", &path_f, "-F", &line_f, "-f", "side=RIGHT",
        ])?;
        if out.status.success() {
            let r: serde_json::Value =
                serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap_or(serde_json::Value::Null);
            return Ok(r.get("html_url").and_then(|x| x.as_str()).unwrap_or("").to_string());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        // gh writes the 422 validation BODY to STDOUT (stderr only has "Validation Failed
        // (HTTP 422)"), and GitHub's wording for a line/path not in the diff is
        // "could not be resolved" — so match across both streams.
        let combined =
            format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr).to_lowercase();
        if combined.contains("not part of the diff")
            || (combined.contains("could not be resolved")
                && (combined.contains("line") || combined.contains("path")))
        {
            return Err("그 줄엔 PR 댓글을 달 수 없어요 — diff에 포함된, 변경된 줄에만 달 수 있어요.".into());
        }
        Err(gh_error(&stderr))
    })
    .await
    .map_err(|e| format!("PR 댓글 작업이 패닉했어요: {e}"))?
}

#[derive(serde::Serialize)]
struct GhStatus {
    installed: bool,
    authed: bool,
}

/// Whether the GitHub CLI is installed AND authenticated. Onboarding shows this to
/// nudge `gh` setup (PR approve / comment / PR-URL review all delegate to gh).
#[tauri::command]
async fn gh_status() -> GhStatus {
    tokio::task::spawn_blocking(|| {
        let path = engine::ai::cli::augmented_path();
        let installed = std::process::Command::new("gh")
            .arg("--version")
            .env("PATH", &path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !installed {
            return GhStatus { installed: false, authed: false };
        }
        let authed = std::process::Command::new("gh")
            .args(["auth", "status"])
            .env("PATH", &path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        GhStatus { installed, authed }
    })
    .await
    .unwrap_or(GhStatus { installed: false, authed: false })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // single-instance MUST be registered first (Tauri requirement). On a second
        // `loupe://…` invocation it focuses the already-running window instead of
        // spawning a new app; the `deep-link` feature re-delivers the URL to it.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
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
            clear_token,
            load_threads,
            save_threads,
            open_in_editor,
            pr_status,
            approve_pr,
            pr_comment,
            gh_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// save → load round-trips the exact array, a never-saved key reads back as `"[]"`, and an
    /// independent key in the same file is preserved across a save.
    #[test]
    fn threads_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.json");

        let key_a = threads_key("/repo", "main", "feature");
        let key_b = threads_key("/repo", "main", "other");

        // A never-saved key degrades to "[]".
        let empty = read_threads_object_at(&path);
        assert_eq!(load_threads_string(&empty, &key_a), "[]");

        // Save bucket A, read it back identically.
        let payload_a = r#"[{"id":"t1","turns":[{"author":"you","text":"hi"}]}]"#;
        save_threads_in_dir(dir.path(), &key_a, payload_a).unwrap();

        let after_a = read_threads_object_at(&path);
        let loaded_a = load_threads_string(&after_a, &key_a);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&loaded_a).unwrap(),
            serde_json::from_str::<serde_json::Value>(payload_a).unwrap()
        );
        // A different, never-saved key in the same file still reads "[]".
        assert_eq!(load_threads_string(&after_a, &key_b), "[]");

        // Save bucket B; bucket A must survive (other buckets preserved).
        let payload_b = r#"[{"id":"t2"}]"#;
        save_threads_in_dir(dir.path(), &key_b, payload_b).unwrap();

        let after_b = read_threads_object_at(&path);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&load_threads_string(&after_b, &key_a))
                .unwrap(),
            serde_json::from_str::<serde_json::Value>(payload_a).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&load_threads_string(&after_b, &key_b))
                .unwrap(),
            serde_json::from_str::<serde_json::Value>(payload_b).unwrap()
        );
    }

    /// A non-array payload is rejected, and a corrupt file degrades to an empty object.
    #[test]
    fn threads_rejects_non_array_and_degrades_on_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let key = threads_key("/repo", "a", "b");

        assert!(save_threads_in_dir(dir.path(), &key, r#"{"not":"array"}"#).is_err());
        assert!(save_threads_in_dir(dir.path(), &key, "not json at all").is_err());

        // Corrupt file → empty object → "[]".
        let path = dir.path().join("threads.json");
        std::fs::write(&path, "{ broken json").unwrap();
        let obj = read_threads_object_at(&path);
        assert_eq!(load_threads_string(&obj, &key), "[]");
    }
}
