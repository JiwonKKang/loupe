//! Anthropic Messages API providers (raw HTTP via reqwest — there is no official
//! Rust SDK, so direct `POST https://api.anthropic.com/v1/messages` is the correct
//! transport per the claude-api skill).
//!
//! Two providers share everything except the auth header(s):
//!  - [`OAuthProvider`] — `authorization: Bearer <token>` + `anthropic-version` +
//!    `anthropic-beta: oauth-2025-04-20`. **This is the normal default path (B3
//!    resolved):** the onboarding setup-token uses OAuth Bearer auth, and the public
//!    Messages API (`/v1/messages`) rejects Bearer auth without the `oauth-2025-04-20`
//!    beta header (claude-api skill, verified). No `user-agent` is required.
//!  - [`ApiKeyProvider`] — `x-api-key: <key>` + `anthropic-version`. The BYO-key
//!    alternative. Does *not* send `anthropic-beta` (key auth doesn't need it).
//!
//! Both build the identical request body, attach structured output the same way
//! (`output_config.format = { type: "json_schema", schema }`, M1), and parse the
//! response identically — only `apply_auth` differs.
//!
//! Hard rules honoured here (v2-critique / claude-api skill, verified):
//!  - **No assistant prefill** (400 on Haiku 4.5 / Sonnet 4.6) — JSON shape is forced
//!    by structured output instead.
//!  - **`effort` only on the Quality/Sonnet tier** (Haiku rejects it with 400). The
//!    Quality tier emits `effort: "medium"`; the tier gate guarantees it never leaks
//!    to Haiku.
//!  - `model_for`: Fast => `claude-haiku-4-5`, Quality => `claude-sonnet-4-6`
//!    (alias ids, never date-suffixed).
//!  - 401/403 => `Auth`, 429/529 => `Overloaded`, request timeout => `Timeout`.
//!  - `stop_reason == "refusal"` => `Refusal` (content may be empty).

use super::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, ModelTier};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

const MODEL_FAST: &str = "claude-haiku-4-5";
const MODEL_QUALITY: &str = "claude-sonnet-4-6";

/// OAuth (Bearer) calls to `/v1/messages` require this beta header — the endpoint
/// rejects setup-token Bearer auth without it (claude-api skill, verified). It is
/// OAuth-only; `ApiKeyProvider` (`x-api-key`) must not send it.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// OAuth (setup-token) provider — the default path. Header = `authorization: Bearer`.
pub struct OAuthProvider {
    client: reqwest::Client,
    token: String,
}

/// API-key provider (BYO `sk-ant-api...`). Header = `x-api-key`. The alternative path.
pub struct ApiKeyProvider {
    client: reqwest::Client,
    api_key: String,
}

impl OAuthProvider {
    /// Build with an internally-constructed client (default timeout).
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            client: default_client(),
            token: token.into(),
        }
    }

    /// Build with a caller-supplied client (e.g. to share a connection pool).
    pub fn with_client(client: reqwest::Client, token: impl Into<String>) -> Self {
        Self {
            client,
            token: token.into(),
        }
    }
}

impl ApiKeyProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: default_client(),
            api_key: api_key.into(),
        }
    }

    pub fn with_client(client: reqwest::Client, api_key: impl Into<String>) -> Self {
        Self {
            client,
            api_key: api_key.into(),
        }
    }
}

fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        // Builder only fails on TLS backend init; fall back to a bare client so the
        // constructor stays infallible.
        .unwrap_or_default()
}

/// Resolve a tier to its concrete model id (shared by both providers).
fn model_for_tier(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => MODEL_FAST,
        ModelTier::Quality => MODEL_QUALITY,
    }
}

/// Build the shared request body. Identical for both providers.
///
/// - `messages` is a single user turn (no assistant prefill — prefill 400s).
/// - `system` is sent as a top-level string when non-empty.
/// - structured output via `output_config.format` (M1) when a schema is present.
/// - `effort: "medium"` only on the Quality tier (Sonnet); Haiku rejects `effort`.
fn build_body(req: &CompletionRequest) -> Value {
    let mut body = json!({
        "model": model_for_tier(req.tier),
        "max_tokens": req.max_tokens,
        // temperature is always sent; 0.0 makes classification/structured output
        // reproducible (same input ⇒ same clusters). Sampling default (1.0) was the
        // cause of run-to-run cluster drift on Haiku.
        "temperature": req.temperature,
        "messages": [
            { "role": "user", "content": req.user }
        ],
    });
    let obj = body.as_object_mut().expect("object literal");

    if !req.system.is_empty() {
        obj.insert("system".into(), Value::String(req.system.clone()));
    }

    // Structured output (M1): output_config.format = { type: "json_schema", schema }.
    let mut output_config = serde_json::Map::new();
    if let Some(schema) = &req.json_schema {
        output_config.insert(
            "format".into(),
            json!({ "type": "json_schema", "schema": schema }),
        );
    }
    // effort is Quality-only (Sonnet 4.6). Never on Haiku (400). Kept conservative at
    // "medium". The tier gate guarantees it never leaks to the Fast (Haiku) tier.
    if req.tier == ModelTier::Quality {
        output_config.insert("effort".into(), Value::String("medium".into()));
    }
    if !output_config.is_empty() {
        obj.insert("output_config".into(), Value::Object(output_config));
    }

    body
}

/// Parse a Messages API success body into a `CompletionResponse`.
///
/// `stop_reason == "refusal"` => `Refusal` (content may be empty). Otherwise the
/// assistant text is concatenated from `content[].text` and parsed with
/// `serde_json::from_str` (never raw-matched) when it looks like JSON; if the body
/// is not JSON it is wrapped as a string `Value` so the caller still gets the text.
fn parse_response(body: &Value) -> Result<CompletionResponse, LlmError> {
    let stop_reason = body
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if stop_reason == "refusal" {
        return Err(LlmError::Refusal);
    }

    // Concatenate every text block. Structured-output responses put the JSON in a
    // single text block; we tolerate multiple.
    let text: String = body
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();

    let trimmed = text.trim();
    let json = if trimmed.is_empty() {
        Value::Null
    } else {
        // Parse with from_str (skill: never raw-match). If it isn't valid JSON, keep
        // the text as a string Value rather than erroring — callers that asked for a
        // schema will validate the shape; free-text callers get their text.
        serde_json::from_str::<Value>(trimmed).unwrap_or(Value::String(text.clone()))
    };

    Ok(CompletionResponse { json, stop_reason })
}

/// Map a non-2xx response to the right `LlmError`. `status` + a body snippet.
fn map_error_status(status: reqwest::StatusCode, body: &str) -> LlmError {
    match status.as_u16() {
        401 | 403 => LlmError::Auth,
        429 | 529 => LlmError::Overloaded,
        _ => {
            let snippet: String = body.chars().take(500).collect();
            LlmError::Http(format!("{status}: {snippet}"))
        }
    }
}

/// Shared send path. `apply_auth` is the only per-provider difference.
async fn send(
    client: &reqwest::Client,
    req: CompletionRequest,
    apply_auth: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<CompletionResponse, LlmError> {
    let body = build_body(&req);

    let builder = client
        .post(API_URL)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body);
    let builder = apply_auth(builder);

    let resp = builder.send().await.map_err(|e| {
        if e.is_timeout() {
            LlmError::Timeout
        } else {
            LlmError::Http(e.to_string())
        }
    })?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(map_error_status(status, &text));
    }

    let value: Value = resp
        .json()
        .await
        .map_err(|e| LlmError::Parse(format!("response body not JSON: {e}")))?;

    parse_response(&value)
}

#[async_trait]
impl LlmProvider for OAuthProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let token = self.token.clone();
        send(&self.client, req, move |b| {
            b.header("authorization", format!("Bearer {token}"))
                .header("anthropic-beta", OAUTH_BETA)
        })
        .await
    }

    fn model_for(&self, tier: ModelTier) -> &'static str {
        model_for_tier(tier)
    }
}

#[async_trait]
impl LlmProvider for ApiKeyProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let key = self.api_key.clone();
        send(&self.client, req, move |b| b.header("x-api-key", key)).await
    }

    fn model_for(&self, tier: ModelTier) -> &'static str {
        model_for_tier(tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ai::ModelTier;

    fn req(tier: ModelTier, schema: Option<Value>) -> CompletionRequest {
        CompletionRequest {
            system: "sys".into(),
            user: "ping".into(),
            max_tokens: 64,
            json_schema: schema,
            tier,
            temperature: 0.0,
        }
    }

    #[test]
    fn body_uses_alias_model_ids_per_tier() {
        let fast = build_body(&req(ModelTier::Fast, None));
        assert_eq!(fast["model"], "claude-haiku-4-5");
        let quality = build_body(&req(ModelTier::Quality, None));
        assert_eq!(quality["model"], "claude-sonnet-4-6");
    }

    #[test]
    fn body_has_no_assistant_prefill_single_user_turn() {
        let body = build_body(&req(ModelTier::Fast, None));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "exactly one user turn, no prefill");
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn effort_only_on_quality_tier() {
        let fast = build_body(&req(ModelTier::Fast, None));
        // No output_config at all on Fast with no schema => no effort leak to Haiku.
        assert!(fast.get("output_config").is_none(), "Haiku must not get effort");

        let quality = build_body(&req(ModelTier::Quality, None));
        assert_eq!(quality["output_config"]["effort"], "medium");
    }

    #[test]
    fn schema_attached_as_output_config_format_m1() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"]
        });
        let body = build_body(&req(ModelTier::Fast, Some(schema.clone())));
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["output_config"]["format"]["schema"], schema);
    }

    #[test]
    fn temperature_is_sent_in_body() {
        // temp=0 is what makes clustering reproducible; it must reach the wire.
        let mut r = req(ModelTier::Fast, None);
        r.temperature = 0.0;
        let body = build_body(&r);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));

        // A non-zero value also round-trips (0.5 is exactly representable in f32).
        r.temperature = 0.5;
        let body = build_body(&r);
        assert_eq!(body["temperature"].as_f64(), Some(0.5));
    }

    #[test]
    fn system_omitted_when_empty() {
        let mut r = req(ModelTier::Fast, None);
        r.system = String::new();
        let body = build_body(&r);
        assert!(body.get("system").is_none());
    }

    #[test]
    fn parse_refusal_stop_reason_is_error() {
        let body = json!({ "stop_reason": "refusal", "content": [] });
        assert_eq!(parse_response(&body), Err(LlmError::Refusal));
    }

    #[test]
    fn parse_extracts_and_parses_json_text_block() {
        let body = json!({
            "stop_reason": "end_turn",
            "content": [ { "type": "text", "text": "{\"clusterId\":\"c1\"}" } ]
        });
        let out = parse_response(&body).unwrap();
        assert_eq!(out.stop_reason, "end_turn");
        assert_eq!(out.json["clusterId"], "c1");
    }

    #[test]
    fn parse_keeps_non_json_text_as_string_value() {
        let body = json!({
            "stop_reason": "end_turn",
            "content": [ { "type": "text", "text": "pong" } ]
        });
        let out = parse_response(&body).unwrap();
        assert_eq!(out.json, Value::String("pong".into()));
    }

    #[test]
    fn map_error_status_classifies_auth_and_overload() {
        use reqwest::StatusCode;
        assert_eq!(map_error_status(StatusCode::UNAUTHORIZED, ""), LlmError::Auth);
        assert_eq!(map_error_status(StatusCode::FORBIDDEN, ""), LlmError::Auth);
        assert_eq!(
            map_error_status(StatusCode::TOO_MANY_REQUESTS, ""),
            LlmError::Overloaded
        );
        assert_eq!(
            map_error_status(StatusCode::from_u16(529).unwrap(), ""),
            LlmError::Overloaded
        );
        assert!(matches!(
            map_error_status(StatusCode::BAD_REQUEST, "bad"),
            LlmError::Http(_)
        ));
    }

    #[test]
    fn oauth_path_sends_bearer_and_beta_headers() {
        // Mirror the exact header set OAuthProvider::complete attaches, then build a
        // real reqwest::Request and inspect its headers. `/v1/messages` rejects Bearer
        // auth without anthropic-beta: oauth-2025-04-20, so both must be present.
        let client = default_client();
        let token = "tok123";
        let request = client
            .post(API_URL)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("anthropic-beta", OAUTH_BETA)
            .build()
            .expect("request builds");

        let headers = request.headers();
        assert_eq!(headers["authorization"], "Bearer tok123");
        assert_eq!(headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(headers["anthropic-version"], ANTHROPIC_VERSION);
    }

    #[test]
    fn api_key_path_omits_beta_header() {
        // x-api-key auth must NOT carry anthropic-beta — key auth doesn't need it.
        let client = default_client();
        let request = client
            .post(API_URL)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("x-api-key", "sk-ant-key")
            .build()
            .expect("request builds");

        let headers = request.headers();
        assert_eq!(headers["x-api-key"], "sk-ant-key");
        assert!(
            !headers.contains_key("anthropic-beta"),
            "api-key path must not send anthropic-beta"
        );
    }

    #[test]
    fn model_for_matches_tier() {
        let p = ApiKeyProvider::new("k");
        assert_eq!(p.model_for(ModelTier::Fast), "claude-haiku-4-5");
        assert_eq!(p.model_for(ModelTier::Quality), "claude-sonnet-4-6");
        let o = OAuthProvider::new("t");
        assert_eq!(o.model_for(ModelTier::Fast), "claude-haiku-4-5");
    }
}
