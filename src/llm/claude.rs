//! Anthropic Claude API client.
//!
//! Supports both standard API keys and OAuth tokens (sk-ant-oat*).
//! OAuth token detection inspired by edge-talk's prompter-antro.shs.
//!
//! Uses the same HTTP pattern as bme680-monitor — EspHttpConnection
//! with initiate_request/initiate_response. No external HTTP crate.

use crate::agent::types::ApiResponse;
use crate::llm::{CompletionRequest, LlmProvider};
use crate::net::http;
use anyhow::{bail, Context, Result};
use log::{debug, error, info, warn};
use serde_json::Value;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

/// Beta header required for OAuth tokens (from edge-talk).
const OAUTH_BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20";

/// HTTP timeout for LLM calls — these can be slow.
const LLM_TIMEOUT_SECS: u64 = 90;

/// How the client authenticates with the Anthropic API.
#[derive(Debug, Clone)]
enum AuthMode {
    /// Standard API key — uses `x-api-key` header.
    ApiKey(String),
    /// OAuth token (sk-ant-oat*) — uses `Authorization: Bearer` header
    /// with additional beta headers, same as Claude Code / edge-talk.
    OAuthToken(String),
}

impl AuthMode {
    /// Detect auth mode from the key/token string.
    ///
    /// OAuth tokens start with "sk-ant-oat" (from edge-talk's prompter-antro.shs):
    /// ```
    /// ext/api-key | If(String.Starts(With: "sk-ant-oat") { ... })
    /// ```
    fn detect(key: impl Into<String>) -> Self {
        let key = key.into();
        if key.starts_with("sk-ant-oat") {
            info!("Using Anthropic OAuth token");
            Self::OAuthToken(key)
        } else {
            Self::ApiKey(key)
        }
    }
}

/// Claude API client for ESP32.
pub struct ClaudeClient {
    auth: AuthMode,
    model: String,
}

impl ClaudeClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            auth: AuthMode::detect(api_key),
            model: DEFAULT_MODEL.into(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn build_request_body(&self, request: &CompletionRequest) -> Result<String> {
        let system_value = match &self.auth {
            AuthMode::OAuthToken(_) => {
                // "Claude Code trick" — OAuth tokens require the system prompt
                // to start with a specific first block, same as edge-talk's
                // prompter-antro.shs (lines 416-445).
                serde_json::json!([
                    {
                        "type": "text",
                        "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                        "cache_control": {"type": "ephemeral"}
                    },
                    {
                        "type": "text",
                        "text": request.system
                    }
                ])
            }
            AuthMode::ApiKey(_) => serde_json::json!(request.system),
        };

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "system": system_value,
            "messages": request.messages,
        });

        if let Value::Array(ref tools) = request.tools {
            if !tools.is_empty() {
                body["tools"] = request.tools.clone();
            }
        }

        serde_json::to_string(&body).context("failed to serialize request")
    }

    /// Build auth headers based on the detected auth mode.
    ///
    /// Standard API key:
    ///   x-api-key: sk-ant-api...
    ///   anthropic-version: 2023-06-01
    ///
    /// OAuth token (edge-talk style):
    ///   Authorization: Bearer sk-ant-oat...
    ///   anthropic-version: 2023-06-01
    ///   anthropic-beta: claude-code-20250219,oauth-2025-04-20,
    fn build_auth_headers(&self) -> Vec<(&str, String)> {
        match &self.auth {
            AuthMode::ApiKey(key) => {
                vec![
                    ("x-api-key", key.clone()),
                    ("anthropic-version", ANTHROPIC_VERSION.into()),
                ]
            }
            AuthMode::OAuthToken(token) => {
                vec![
                    ("Authorization", format!("Bearer {token}")),
                    ("anthropic-version", ANTHROPIC_VERSION.into()),
                    ("anthropic-beta", OAUTH_BETA_HEADER.into()),
                ]
            }
        }
    }
}

impl LlmProvider for ClaudeClient {
    fn complete(&self, request: &CompletionRequest) -> Result<ApiResponse> {
        let body = self.build_request_body(request)?;
        debug!("Claude request: {} bytes", body.len());

        let auth_headers = self.build_auth_headers();
        let extra_headers: Vec<(&str, &str)> = auth_headers
            .iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect();

        let (status, response_str) =
            http::post_json(ANTHROPIC_API_URL, &body, &extra_headers, LLM_TIMEOUT_SECS)?;

        if status != 200 {
            error!(
                "Claude API error ({}): {}",
                status,
                &response_str[..response_str.len().min(500)]
            );

            if let Ok(err_json) = serde_json::from_str::<Value>(&response_str) {
                let err_msg = err_json["error"]["message"]
                    .as_str()
                    .unwrap_or(&response_str);

                match status {
                    429 => bail!("rate limited: {err_msg}"),
                    401 => bail!("invalid API key or OAuth token"),
                    400 => {
                        if err_msg.contains("token") {
                            warn!("context window likely exceeded, compaction needed");
                        }
                        bail!("bad request: {err_msg}");
                    }
                    _ => bail!("Claude API error ({status}): {err_msg}"),
                }
            }

            bail!("Claude API error ({status})");
        }

        debug!("Claude response: {} bytes", response_str.len());
        serde_json::from_str(&response_str).context("failed to parse Claude response")
    }
}
