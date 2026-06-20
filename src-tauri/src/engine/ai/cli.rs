//! `claude` CLI provider — Sonnet via the Claude Code backend (the only path that
//! reaches Sonnet on a setup-token).
//!
//! ## Why a CLI provider exists
//! The direct Messages API (`anthropic::OAuthProvider`, Bearer setup-token) returns
//! HTTP 429 on Sonnet — only Haiku is reachable that way. The `claude` CLI, however,
//! routes through the Claude Code backend and **does** reach Sonnet on the same
//! setup-token (verified: `claude -p ... --model sonnet` returns real output). So the
//! clustering/ordering/labelling pipeline calls Sonnet *through the CLI* instead of the
//! raw API. `OAuthProvider` (Haiku, direct) stays as the alternative.
//!
//! ## How `complete` shells out
//! `std::process::Command` runs:
//! ```text
//! claude -p <user> --model <sonnet|haiku> --output-format json \
//!        --tools "" --no-session-persistence \
//!        --append-system-prompt <system> \
//!        [--json-schema <schema>]      # only when req.json_schema is Some
//! ```
//! with `CLAUDE_CODE_OAUTH_TOKEN` set in the child env (never logged, never on argv).
//!
//! Noise/contamination defences (verified against the live CLI):
//!  - **`--tools ""`** disables every built-in coding-agent tool (Bash/Edit/Read/…), so
//!    the model can't run tools and pollute the answer (it only emits text/structured
//!    output). This is the documented "disable all tools" form.
//!  - **clean cwd** (`std::env::temp_dir`) keeps the current repo's `CLAUDE.md` /
//!    project context out of the system prompt. (`--bare` would also skip CLAUDE.md but
//!    it *forces* `ANTHROPIC_API_KEY`-only auth and rejects the OAuth setup-token, so we
//!    must NOT use it — verified: `--bare` ⇒ "Not logged in".)
//!  - **`--no-session-persistence`** avoids writing session files to disk.
//!
//! ## Output parsing (verified shapes)
//! `--output-format json` makes stdout a single wrapper object. Two cases:
//!  - **with `--json-schema`**: the schema-conforming object lands in a top-level
//!    `structured_output` field (already a JSON value, no fences); `result` carries a
//!    natural-language summary. We read `structured_output` first.
//!  - **without a schema** (free text): the model's text — our JSON, possibly fenced —
//!    is in `result`. We strip an optional ```json fence and `serde_json::from_str` it.
//!
//! `is_error: true` ⇒ `LlmError` (the CLI puts the message in `result`). A spawn failure
//! (binary missing) ⇒ `LlmError::Http("claude CLI not found")`; a non-zero exit with no
//! parseable wrapper ⇒ `LlmError::Http(stderr)`.
//!
//! ## Structured output: schema *and* prompt
//! `--json-schema` already forces a schema-valid object, but to match the API provider's
//! contract (and to harden free-text-ish calls) the schema is *also* appended to the
//! system prompt as an instruction ("emit only JSON matching this schema"). Both are
//! cheap and reinforce each other.

use super::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier};
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

/// Default CLI binary name (resolved on `PATH`).
const DEFAULT_CLAUDE_BIN: &str = "claude";

/// CLI model alias the codebase-aware agent runs on. The spec pins Sonnet by full id; the
/// CLI accepts both bare aliases and full names, so this is sent verbatim to `--model`.
const AGENTIC_MODEL: &str = "claude-sonnet-4-6";

/// Read-only tool allowlist for the agent. Only file *reading* / *searching* tools are
/// granted — `Read` (open a file), `Grep` (content search), `Glob` (path search), `LS`
/// (directory listing). **No write/exec tool is ever on this list** (Write/Edit/Bash/etc.),
/// so the agent can inspect the repo but cannot modify or run anything in it.
const AGENTIC_ALLOWED_TOOLS: &[&str] = &["Read", "Grep", "Glob", "LS"];

/// Defense-in-depth deny-list: even if a future default ever flipped a mutating/exec tool
/// on, these are explicitly forbidden. The allowlist above is the primary gate; this is a
/// belt-and-braces second barrier against modifying or executing anything in the user repo.
const AGENTIC_DISALLOWED_TOOLS: &[&str] =
    &["Bash", "Write", "Edit", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch"];

/// Hard wall-clock cap for one agentic call. Agents read files / run searches in a loop, so
/// they are far slower than a single completion; without a cap a stuck turn (e.g. waiting on
/// something) would hang the Tauri command forever. On expiry we return a human-readable Err
/// instead of blocking the UI.
const AGENTIC_TIMEOUT: Duration = Duration::from_secs(600);

/// The env var the `claude` CLI reads for the setup-token (OAuth). Set on the child
/// process only — never logged, never placed on the command line.
const TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// CLI model alias for a tier. The CLI accepts bare aliases (`sonnet` / `haiku`) and
/// resolves them to the latest concrete model — Quality ⇒ Sonnet (the whole point of
/// this provider), Fast ⇒ Haiku.
const CLI_MODEL_QUALITY: &str = "sonnet";
const CLI_MODEL_FAST: &str = "haiku";

/// `model_for` still reports the concrete alias ids the rest of the engine uses, so the
/// `LlmProvider` contract is unchanged (tests assert these).
const MODEL_QUALITY_ID: &str = "claude-sonnet-4-6";
const MODEL_FAST_ID: &str = "claude-haiku-4-5";

/// A provider that fulfils completions by shelling out to the `claude` CLI. Holds the
/// setup-token (passed to the child via env) and the binary path (overridable for tests).
pub struct CliProvider {
    token: String,
    claude_bin: String,
}

impl CliProvider {
    /// Build with the default `claude` binary (resolved on `PATH`).
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            claude_bin: DEFAULT_CLAUDE_BIN.to_string(),
        }
    }

    /// Build with an explicit binary path (a stub script in tests; an absolute path in
    /// odd installs).
    pub fn with_bin(token: impl Into<String>, claude_bin: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            claude_bin: claude_bin.into(),
        }
    }

    /// CLI model alias for a tier (`sonnet` / `haiku`).
    fn cli_model(tier: ModelTier) -> &'static str {
        match tier {
            ModelTier::Quality => CLI_MODEL_QUALITY,
            ModelTier::Fast => CLI_MODEL_FAST,
        }
    }

    /// Build the full system prompt: the caller's system text plus, when a schema is
    /// present, an explicit "emit only this JSON schema" instruction. `--json-schema`
    /// also enforces the shape server-side, but the prompt reinforces it (and helps the
    /// `result`-fallback path stay clean JSON).
    fn build_system_prompt(req: &CompletionRequest) -> String {
        let mut sys = req.system.clone();
        if let Some(schema) = &req.json_schema {
            let schema_text = serde_json::to_string(schema).unwrap_or_default();
            if !sys.is_empty() {
                sys.push_str("\n\n");
            }
            sys.push_str(
                "You MUST follow this exact JSON schema and output ONLY the JSON object \
                 (no markdown, no code fences, no commentary):\n",
            );
            sys.push_str(&schema_text);
        }
        sys
    }

    /// Assemble the `claude` argv for this request. Pure (no I/O) so it is unit-testable.
    /// `system` is the already-assembled system prompt (see `build_system_prompt`).
    fn build_args(req: &CompletionRequest, system: &str) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "-p".into(),
            req.user.clone(),
            "--model".into(),
            Self::cli_model(req.tier).into(),
            "--output-format".into(),
            "json".into(),
            // Disable all built-in coding-agent tools — the model only emits the answer,
            // never runs Bash/Edit/etc. ("" = disable all, per `claude --help`).
            "--tools".into(),
            String::new(),
            // Don't write session files to disk.
            "--no-session-persistence".into(),
            "--append-system-prompt".into(),
            system.to_string(),
        ];
        // Native structured-output enforcement when a schema is present.
        if let Some(schema) = &req.json_schema {
            args.push("--json-schema".into());
            args.push(serde_json::to_string(schema).unwrap_or_default());
        }
        args
    }
}

/// Strip an optional ```json … ``` (or bare ``` … ```) fence around a JSON payload.
/// The `--json-schema` path returns fence-free structured output, but the free-text
/// `result` path can include a fence — defend against it before `from_str`.
fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    // Drop an optional language tag on the opening fence line (e.g. "json\n…").
    let rest = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    rest.trim().strip_suffix("```").unwrap_or(rest).trim()
}

/// Extract the structured `CompletionResponse` from the CLI wrapper object.
///
///  - `is_error: true` ⇒ `LlmError` (message taken from `result`).
///  - `structured_output` present (the `--json-schema` path) ⇒ use it verbatim.
///  - else parse `result` as JSON (fence-stripped). A non-JSON `result` is kept as a
///    string `Value` so free-text callers still get their text (mirrors the API provider).
///  - empty/missing content ⇒ `LlmError::Parse`.
///
/// `stop_reason` is surfaced from the wrapper (`end_turn` / `max_tokens` / …).
fn parse_wrapper(wrapper: &Value) -> Result<CompletionResponse, LlmError> {
    let is_error = wrapper
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let result_text = wrapper.get("result").and_then(Value::as_str).unwrap_or("");

    if is_error {
        // The CLI reports auth failures in `result` ("Not logged in …"); surface those
        // as Auth so the orchestrator can distinguish a credential problem from transport.
        if result_text.contains("Not logged in") || result_text.contains("login") {
            return Err(LlmError::Auth);
        }
        return Err(LlmError::Http(format!("claude CLI error: {result_text}")));
    }

    let stop_reason = wrapper
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Preferred path: native structured output (from --json-schema). Already a Value.
    if let Some(structured) = wrapper.get("structured_output") {
        if !structured.is_null() {
            return Ok(CompletionResponse {
                json: structured.clone(),
                stop_reason,
            });
        }
    }

    // Fallback path: the JSON (or free text) is in `result`. Strip a fence, then parse.
    let trimmed = strip_code_fence(result_text);
    if trimmed.is_empty() {
        return Err(LlmError::Parse(
            "claude CLI returned empty result and no structured_output".into(),
        ));
    }
    let json = serde_json::from_str::<Value>(trimmed)
        // Not JSON? Keep the text as a string Value (free-text callers); the schema
        // callers above never reach here because they get `structured_output`.
        .unwrap_or_else(|_| Value::String(result_text.to_string()));

    Ok(CompletionResponse { json, stop_reason })
}

/// Assemble the `claude` argv for a **codebase-aware (agentic), read-only** run. Pure (no
/// I/O) so it is unit-testable — the spawn/cwd/env wiring lives in `ask_agentic`.
///
/// The agent is allowed to *read* the repo (Read/Grep/Glob/LS) and nothing else: the
/// allowlist grants only those four, and a deny-list explicitly blocks Write/Edit/Bash/etc.
/// `--permission-mode default` means anything not on the allowlist is *denied* (in `-p`
/// non-interactive mode there is no prompt to hang on — the model is just told it can't use
/// that tool), so the agent can never modify or execute anything in the user's repo.
fn build_agentic_args(prompt: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-p".into(),
        prompt.to_string(),
        "--model".into(),
        AGENTIC_MODEL.into(),
        "--output-format".into(),
        "json".into(),
        // Non-interactive: deny anything not explicitly allowed (no auto-accept of writes/
        // exec), and in -p mode there is no permission prompt to block on.
        "--permission-mode".into(),
        "default".into(),
        // Don't write session files to disk.
        "--no-session-persistence".into(),
    ];
    // Read-only allowlist: each tool as its own argv element (space-separated form).
    args.push("--allowedTools".into());
    for tool in AGENTIC_ALLOWED_TOOLS {
        args.push((*tool).to_string());
    }
    // Explicit deny-list (defense in depth) — never grant mutating/exec tools.
    args.push("--disallowedTools".into());
    for tool in AGENTIC_DISALLOWED_TOOLS {
        args.push((*tool).to_string());
    }
    args
}

/// Extract the agent's **natural-language** answer (the wrapper's `result` text) from the
/// `--output-format json` wrapper. Unlike `parse_wrapper` (which parses `result` as JSON for
/// the clustering pipeline), the agent replies in free prose, so we return `result` verbatim.
///  - `is_error: true` ⇒ `Err` (auth failures surfaced distinctly, like `parse_wrapper`).
///  - empty/missing `result` ⇒ `Err`.
fn extract_agentic_result(wrapper: &Value) -> Result<String, String> {
    let is_error = wrapper
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result_text = wrapper.get("result").and_then(Value::as_str).unwrap_or("");

    if is_error {
        if result_text.contains("Not logged in") || result_text.contains("login") {
            return Err("authentication failed (check the model token)".to_string());
        }
        return Err(format!("claude agent error: {result_text}"));
    }

    let trimmed = result_text.trim();
    if trimmed.is_empty() {
        return Err("claude agent returned an empty answer".to_string());
    }
    Ok(trimmed.to_string())
}

/// Run a **codebase-aware (agentic), read-only** query through the `claude` CLI and return the
/// agent's natural-language answer.
///
/// Unlike `CliProvider::complete` (a single prompt-only completion with **all** tools disabled
/// and a clean temp cwd), this runs the CLI as an agent that may *read* the repo: it is given
/// the repo as its working directory and the read-only tool set (Read/Grep/Glob/LS) so it can
/// open real files, grep for definitions/callers, and answer grounded in the actual codebase.
///
/// Safety (read-only is the top priority):
///  - **allowlist = Read/Grep/Glob/LS only**; **deny-list blocks Write/Edit/Bash/etc.** — the
///    agent cannot modify or execute anything in the user's repo.
///  - `current_dir(repo_path)` confines tool access to that repo (no access outside it).
///  - the setup-token goes to the child via the `CLAUDE_CODE_OAUTH_TOKEN` env only — **never on
///    argv, never logged**.
///  - a 600 s `tokio::time::timeout` caps a stuck run so the Tauri command can never hang.
///
/// `bin` overrides the binary (a stub script in tests); `None` uses the default on `PATH`.
pub async fn ask_agentic(
    token: String,
    repo_path: String,
    prompt: String,
    bin: Option<String>,
) -> Result<String, String> {
    let bin = bin.unwrap_or_else(|| DEFAULT_CLAUDE_BIN.to_string());
    let args = build_agentic_args(&prompt);

    // The CLI is blocking; run it on the blocking pool. The closure owns everything it needs.
    let spawn = tokio::task::spawn_blocking(move || {
        Command::new(&bin)
            .args(&args)
            // Give the agent the repo as context AND confine its read tools to that tree.
            .current_dir(&repo_path)
            // Setup-token via env only — never on argv, never logged.
            .env(TOKEN_ENV, token)
            .output()
    });

    // Hard wall-clock cap: agents loop over reads/searches and can be slow; never hang the UI.
    let output = match tokio::time::timeout(AGENTIC_TIMEOUT, spawn).await {
        Ok(joined) => joined.map_err(|e| format!("claude agent task join failed: {e}"))?,
        Err(_) => {
            return Err(format!(
                "the agent took too long to answer (over {}s) — try a narrower question",
                AGENTIC_TIMEOUT.as_secs()
            ));
        }
    };

    let output = match output {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("claude CLI not found".to_string());
        }
        Err(e) => return Err(format!("claude agent spawn failed: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let wrapper: Value = match serde_json::from_str(stdout.trim()) {
        Ok(v) => v,
        Err(e) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "claude agent exited {:?}: {}",
                    output.status.code(),
                    stderr.trim()
                ));
            }
            return Err(format!("claude agent stdout not JSON: {e}"));
        }
    };

    extract_agentic_result(&wrapper)
}

#[async_trait]
impl LlmProvider for CliProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let system = Self::build_system_prompt(&req);
        let args = Self::build_args(&req, &system);
        let bin = self.claude_bin.clone();
        let token = self.token.clone();

        // The CLI is blocking; run it on the blocking pool so we don't stall the async
        // runtime. The closure owns everything it needs (no borrows escape).
        let output = tokio::task::spawn_blocking(move || {
            Command::new(&bin)
                .args(&args)
                // Clean working directory: keep the current repo's CLAUDE.md / project
                // context out of the model's system prompt.
                .current_dir(std::env::temp_dir())
                // Setup-token via env only — never on argv, never logged.
                .env(TOKEN_ENV, token)
                .output()
        })
        .await
        // JoinError (the blocking task panicked) — treat as transport.
        .map_err(|e| LlmError::Http(format!("claude CLI task join failed: {e}")))?;

        let output = match output {
            Ok(o) => o,
            // spawn failed — almost always "binary not found".
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(LlmError::Http("claude CLI not found".into()));
            }
            Err(e) => return Err(LlmError::Http(format!("claude CLI spawn failed: {e}"))),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let wrapper: Value = match serde_json::from_str(stdout.trim()) {
            Ok(v) => v,
            Err(e) => {
                // No parseable wrapper. If the process also failed, surface stderr; else
                // the stdout was malformed.
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(LlmError::Http(format!(
                        "claude CLI exited {:?}: {}",
                        output.status.code(),
                        stderr.trim()
                    )));
                }
                return Err(LlmError::Parse(format!(
                    "claude CLI stdout not JSON: {e}"
                )));
            }
        };

        parse_wrapper(&wrapper)
    }

    fn model_for(&self, tier: ModelTier) -> &'static str {
        match tier {
            ModelTier::Quality => MODEL_QUALITY_ID,
            ModelTier::Fast => MODEL_FAST_ID,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ai::ModelTier;
    use serde_json::json;

    fn req(tier: ModelTier, schema: Option<Value>) -> CompletionRequest {
        CompletionRequest {
            system: "You classify cards.".into(),
            user: "payload".into(),
            max_tokens: 4096,
            json_schema: schema,
            tier,
            temperature: 0.0,
        }
    }

    #[test]
    fn args_use_sonnet_for_quality_and_haiku_for_fast() {
        let q = CliProvider::build_args(&req(ModelTier::Quality, None), "sys");
        let model_idx = q.iter().position(|a| a == "--model").unwrap();
        assert_eq!(q[model_idx + 1], "sonnet");

        let f = CliProvider::build_args(&req(ModelTier::Fast, None), "sys");
        let model_idx = f.iter().position(|a| a == "--model").unwrap();
        assert_eq!(f[model_idx + 1], "haiku");
    }

    #[test]
    fn args_disable_all_tools_and_request_json_output() {
        let a = CliProvider::build_args(&req(ModelTier::Quality, None), "sys");
        // --tools "" (disable all built-in tools): the flag is immediately followed by
        // an empty string.
        let tools_idx = a.iter().position(|x| x == "--tools").expect("--tools present");
        assert_eq!(a[tools_idx + 1], "", "--tools must be followed by empty string");
        // JSON output + non-interactive print.
        assert!(a.windows(2).any(|w| w[0] == "--output-format" && w[1] == "json"));
        assert!(a.iter().any(|x| x == "-p"));
        assert!(a.iter().any(|x| x == "--no-session-persistence"));
    }

    #[test]
    fn args_pass_user_and_system_prompt() {
        let a = CliProvider::build_args(&req(ModelTier::Quality, None), "MY SYSTEM");
        // -p <user>
        let p_idx = a.iter().position(|x| x == "-p").unwrap();
        assert_eq!(a[p_idx + 1], "payload");
        // --append-system-prompt <system>
        let s_idx = a.iter().position(|x| x == "--append-system-prompt").unwrap();
        assert_eq!(a[s_idx + 1], "MY SYSTEM");
    }

    #[test]
    fn args_include_json_schema_only_when_schema_present() {
        let no_schema = CliProvider::build_args(&req(ModelTier::Quality, None), "sys");
        assert!(
            !no_schema.iter().any(|x| x == "--json-schema"),
            "no --json-schema without a schema"
        );

        let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}}});
        let with_schema =
            CliProvider::build_args(&req(ModelTier::Quality, Some(schema.clone())), "sys");
        let idx = with_schema
            .iter()
            .position(|x| x == "--json-schema")
            .expect("--json-schema present");
        let sent: Value = serde_json::from_str(&with_schema[idx + 1]).unwrap();
        assert_eq!(sent, schema);
    }

    #[test]
    fn system_prompt_appends_schema_instruction() {
        let schema = json!({"type":"object"});
        let sys = CliProvider::build_system_prompt(&req(ModelTier::Quality, Some(schema)));
        assert!(sys.starts_with("You classify cards."));
        assert!(sys.contains("exact JSON schema"));
        assert!(sys.contains("ONLY the JSON object"));
        // Without a schema, the system prompt is unchanged.
        let plain = CliProvider::build_system_prompt(&req(ModelTier::Quality, None));
        assert_eq!(plain, "You classify cards.");
    }

    #[test]
    fn parse_prefers_structured_output_over_result() {
        // The --json-schema path: structured_output holds the object; result is prose.
        let wrapper = json!({
            "is_error": false,
            "stop_reason": "end_turn",
            "result": "I grouped a and b into cluster s1.",
            "structured_output": {
                "clusters": [{ "clusterId": "s1", "memberCardIds": ["a", "b"], "kind": "flow" }],
                "unclustered": []
            }
        });
        let out = parse_wrapper(&wrapper).unwrap();
        assert_eq!(out.stop_reason, "end_turn");
        assert_eq!(out.json["clusters"][0]["clusterId"], "s1");
        assert_eq!(out.json["clusters"][0]["kind"], "flow");
        // The prose result must NOT leak into json when structured_output exists.
        assert!(out.json.get("clusters").is_some());
    }

    #[test]
    fn parse_falls_back_to_result_json_when_no_structured_output() {
        let wrapper = json!({
            "is_error": false,
            "stop_reason": "end_turn",
            "result": "{\"clusters\":[{\"clusterId\":\"c1\",\"memberCardIds\":[\"x\"],\"kind\":\"infra\"}],\"unclustered\":[]}"
        });
        let out = parse_wrapper(&wrapper).unwrap();
        assert_eq!(out.json["clusters"][0]["clusterId"], "c1");
        assert_eq!(out.json["unclustered"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn parse_strips_json_code_fence_from_result() {
        let wrapper = json!({
            "is_error": false,
            "stop_reason": "end_turn",
            "result": "```json\n{\"clusterId\":\"c1\"}\n```"
        });
        let out = parse_wrapper(&wrapper).unwrap();
        assert_eq!(out.json["clusterId"], "c1");
    }

    #[test]
    fn parse_strips_bare_code_fence_from_result() {
        let wrapper = json!({
            "is_error": false,
            "stop_reason": "end_turn",
            "result": "```\n{\"ok\":true}\n```"
        });
        let out = parse_wrapper(&wrapper).unwrap();
        assert_eq!(out.json["ok"], true);
    }

    #[test]
    fn parse_is_error_true_becomes_http_error() {
        let wrapper = json!({
            "is_error": true,
            "result": "model overloaded, try again"
        });
        let err = parse_wrapper(&wrapper).unwrap_err();
        assert!(matches!(err, LlmError::Http(_)), "got {err:?}");
    }

    #[test]
    fn parse_not_logged_in_becomes_auth_error() {
        let wrapper = json!({
            "is_error": true,
            "result": "Not logged in · Please run /login"
        });
        assert_eq!(parse_wrapper(&wrapper).unwrap_err(), LlmError::Auth);
    }

    #[test]
    fn parse_empty_result_no_structured_output_is_parse_error() {
        let wrapper = json!({ "is_error": false, "result": "" });
        assert!(matches!(
            parse_wrapper(&wrapper).unwrap_err(),
            LlmError::Parse(_)
        ));
    }

    #[test]
    fn parse_non_json_free_text_result_kept_as_string() {
        // No schema, model replied with plain text (e.g. "pong"): keep it as a string
        // Value so free-text callers still get their text (mirrors OAuthProvider).
        let wrapper = json!({
            "is_error": false,
            "stop_reason": "end_turn",
            "result": "pong"
        });
        let out = parse_wrapper(&wrapper).unwrap();
        assert_eq!(out.json, Value::String("pong".into()));
    }

    #[test]
    fn strip_code_fence_passthrough_for_plain_json() {
        assert_eq!(strip_code_fence("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_code_fence("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn model_for_reports_concrete_alias_ids() {
        let p = CliProvider::new("tok");
        assert_eq!(p.model_for(ModelTier::Quality), "claude-sonnet-4-6");
        assert_eq!(p.model_for(ModelTier::Fast), "claude-haiku-4-5");
    }

    // A stub `claude` binary (a shell script) lets us exercise the full spawn → parse
    // path without the network. It echoes a canned wrapper JSON and ignores its args.
    #[cfg(unix)]
    #[tokio::test]
    async fn complete_spawns_stub_bin_and_parses_structured_output() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("loupe_cli_stub_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude_stub.sh");
        let mut f = std::fs::File::create(&bin).unwrap();
        // Emit a wrapper with structured_output; ignore all args/stdin.
        writeln!(
            f,
            "#!/bin/sh\ncat <<'EOF'\n{{\"is_error\":false,\"stop_reason\":\"end_turn\",\"result\":\"summary\",\"structured_output\":{{\"clusters\":[{{\"clusterId\":\"s1\",\"memberCardIds\":[\"a\"],\"kind\":\"flow\"}}],\"unclustered\":[]}}}}\nEOF"
        )
        .unwrap();
        let mut perms = std::fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).unwrap();

        let provider = CliProvider::with_bin("tok", bin.to_string_lossy().into_owned());
        let schema = json!({"type":"object"});
        let resp = provider
            .complete(req(ModelTier::Quality, Some(schema)))
            .await
            .expect("stub completes");
        assert_eq!(resp.stop_reason, "end_turn");
        assert_eq!(resp.json["clusters"][0]["clusterId"], "s1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- ask_agentic (codebase-aware, read-only) ----

    #[test]
    fn agentic_args_request_json_output_and_pin_sonnet() {
        let a = build_agentic_args("explain foo");
        // -p <prompt>
        let p_idx = a.iter().position(|x| x == "-p").unwrap();
        assert_eq!(a[p_idx + 1], "explain foo");
        // --model claude-sonnet-4-6
        let m_idx = a.iter().position(|x| x == "--model").unwrap();
        assert_eq!(a[m_idx + 1], "claude-sonnet-4-6");
        // --output-format json
        assert!(a.windows(2).any(|w| w[0] == "--output-format" && w[1] == "json"));
        // non-interactive print + no session files on disk
        assert!(a.iter().any(|x| x == "--no-session-persistence"));
    }

    #[test]
    fn agentic_args_allow_only_read_only_tools() {
        let a = build_agentic_args("q");
        let allow_idx = a
            .iter()
            .position(|x| x == "--allowedTools")
            .expect("--allowedTools present");
        // The four read-only tools follow the flag, in order.
        assert_eq!(&a[allow_idx + 1..allow_idx + 5], &["Read", "Grep", "Glob", "LS"]);
    }

    #[test]
    fn agentic_args_never_allow_write_or_exec_tools() {
        let a = build_agentic_args("q");
        // Bound the allow-list region: between --allowedTools and the next flag.
        let allow_idx = a.iter().position(|x| x == "--allowedTools").unwrap();
        let allow_end = a[allow_idx + 1..]
            .iter()
            .position(|x| x.starts_with("--"))
            .map(|p| allow_idx + 1 + p)
            .unwrap_or(a.len());
        let allowed = &a[allow_idx + 1..allow_end];
        for forbidden in ["Bash", "Write", "Edit", "MultiEdit", "NotebookEdit"] {
            assert!(
                !allowed.iter().any(|t| t == forbidden),
                "{forbidden} must never be allowed (read-only)"
            );
        }
        // And they appear on the explicit deny-list.
        let deny_idx = a
            .iter()
            .position(|x| x == "--disallowedTools")
            .expect("--disallowedTools present");
        let denied = &a[deny_idx + 1..];
        for forbidden in ["Bash", "Write", "Edit"] {
            assert!(
                denied.iter().any(|t| t == forbidden),
                "{forbidden} must be on the deny-list"
            );
        }
    }

    #[test]
    fn agentic_args_use_non_auto_permission_mode() {
        let a = build_agentic_args("q");
        let idx = a.iter().position(|x| x == "--permission-mode").unwrap();
        let mode = &a[idx + 1];
        // Must NOT be a mode that auto-grants writes/exec.
        assert_ne!(mode, "bypassPermissions");
        assert_ne!(mode, "acceptEdits");
        assert_eq!(mode, "default");
    }

    #[test]
    fn extract_agentic_returns_result_text() {
        let wrapper = json!({
            "is_error": false,
            "result": "The function foo() is defined in src/foo.rs and called from bar()."
        });
        let out = extract_agentic_result(&wrapper).unwrap();
        assert!(out.contains("foo()"));
    }

    #[test]
    fn extract_agentic_is_error_becomes_err() {
        let wrapper = json!({ "is_error": true, "result": "model overloaded" });
        let err = extract_agentic_result(&wrapper).unwrap_err();
        assert!(err.contains("agent error"), "got {err}");
    }

    #[test]
    fn extract_agentic_not_logged_in_is_auth_err() {
        let wrapper = json!({ "is_error": true, "result": "Not logged in · run /login" });
        let err = extract_agentic_result(&wrapper).unwrap_err();
        assert!(err.contains("authentication"), "got {err}");
    }

    #[test]
    fn extract_agentic_empty_result_is_err() {
        let wrapper = json!({ "is_error": false, "result": "" });
        assert!(extract_agentic_result(&wrapper).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ask_agentic_spawns_stub_bin_and_returns_result_text() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("loupe_agent_stub_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude_agent_stub.sh");
        let mut f = std::fs::File::create(&bin).unwrap();
        // Emit a wrapper with a free-text result; ignore all args/stdin.
        writeln!(
            f,
            "#!/bin/sh\ncat <<'EOF'\n{{\"is_error\":false,\"result\":\"foo() lives in src/foo.rs\"}}\nEOF"
        )
        .unwrap();
        let mut perms = std::fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).unwrap();

        let answer = ask_agentic(
            "tok".into(),
            dir.to_string_lossy().into_owned(),
            "where is foo()?".into(),
            Some(bin.to_string_lossy().into_owned()),
        )
        .await
        .expect("stub agent answers");
        assert!(answer.contains("src/foo.rs"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ask_agentic_missing_binary_is_not_found() {
        let err = ask_agentic(
            "tok".into(),
            std::env::temp_dir().to_string_lossy().into_owned(),
            "q".into(),
            Some("/nonexistent/claude_binary_xyz".into()),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not found"), "got {err}");
    }

    #[tokio::test]
    async fn complete_missing_binary_is_http_not_found() {
        let provider = CliProvider::with_bin("tok", "/nonexistent/claude_binary_xyz");
        let err = provider
            .complete(req(ModelTier::Quality, None))
            .await
            .unwrap_err();
        match err {
            LlmError::Http(msg) => assert!(msg.contains("not found"), "got {msg}"),
            other => panic!("expected Http(not found), got {other:?}"),
        }
    }
}
