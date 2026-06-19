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

/// Default CLI binary name (resolved on `PATH`).
const DEFAULT_CLAUDE_BIN: &str = "claude";

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
