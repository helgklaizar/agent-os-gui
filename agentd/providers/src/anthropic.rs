//! Anthropic provider: Messages API, streaming SSE.

use std::sync::Arc;

use agentd_core::{
    Clock, ModelAccount, ModelProvider, ModelRequest, PortFuture, ProviderError, ProviderEvent,
    SecretStore, ToolCall, TurnRole, Usage,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::common::{emit, join_url, key_for};
use crate::http::HttpClient;
use crate::limits::anthropic_limits;
use crate::sse::SseEvent;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const VERSION: &str = "2023-06-01";

/// Stream event parsing; pure logic, no network.
#[derive(Debug, Default)]
pub struct AnthropicParser {
    usage: Usage,
    /// `tool_use` block being parsed: `(id, name, accumulated argument JSON)`.
    tool: Option<(String, String, String)>,
}

fn error_of(kind: &str) -> ProviderError {
    match kind {
        "overloaded_error" => ProviderError::new("overloaded", true),
        "rate_limit_error" => ProviderError::new("rate_limited", true),
        "authentication_error" | "permission_error" => ProviderError::new("unauthorized", false),
        "api_error" => ProviderError::new("upstream", true),
        _ => ProviderError::new("bad_response", false),
    }
}

impl AnthropicParser {
    #[must_use]
    pub fn new() -> AnthropicParser {
        AnthropicParser::default()
    }

    /// Returns `true` when the stream is finished (`message_stop` or an error).
    pub fn on_event(&mut self, ev: &SseEvent, out: &mut Vec<ProviderEvent>) -> bool {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            out.push(ProviderEvent::Error(ProviderError::new(
                "bad_response",
                false,
            )));
            return true;
        };
        let kind = v["type"].as_str().or(ev.event.as_deref()).unwrap_or("");
        let num = |p: &Value| p.as_u64();
        match kind {
            "message_start" => {
                self.usage.tokens_in = num(&v["message"]["usage"]["input_tokens"]).unwrap_or(0);
                self.usage.tokens_out = num(&v["message"]["usage"]["output_tokens"]).unwrap_or(0);
            }
            "content_block_start" => {
                let b = &v["content_block"];
                self.tool = (b["type"] == "tool_use").then(|| {
                    (
                        b["id"].as_str().unwrap_or_default().to_string(),
                        b["name"].as_str().unwrap_or_default().to_string(),
                        String::new(),
                    )
                });
            }
            "content_block_delta" => {
                if v["delta"]["type"] == "text_delta"
                    && let Some(t) = v["delta"]["text"].as_str().filter(|t| !t.is_empty())
                {
                    out.push(ProviderEvent::Delta(t.to_string()));
                } else if v["delta"]["type"] == "input_json_delta"
                    && let (Some((_, _, json)), Some(part)) =
                        (self.tool.as_mut(), v["delta"]["partial_json"].as_str())
                {
                    json.push_str(part);
                }
            }
            "content_block_stop" => {
                if let Some((id, name, json)) = self.tool.take() {
                    // A block without arguments arrives as an empty string: an object with no fields.
                    let raw = if json.trim().is_empty() { "{}" } else { &json };
                    // Truncated or broken JSON is not a call: an action with guessed arguments must not run.
                    if let Ok(args) = serde_json::from_str::<Value>(raw)
                        && args.is_object()
                        && !name.is_empty()
                    {
                        out.push(ProviderEvent::ToolCall(ToolCall { id, name, args }));
                    }
                }
            }
            "message_delta" => {
                if let Some(n) = num(&v["usage"]["output_tokens"]) {
                    self.usage.tokens_out = n;
                }
            }
            "message_stop" => {
                out.push(ProviderEvent::Usage(self.usage));
                return true;
            }
            "error" => {
                if self.usage != Usage::default() {
                    out.push(ProviderEvent::Usage(self.usage));
                }
                out.push(ProviderEvent::Error(error_of(
                    v["error"]["type"].as_str().unwrap_or(""),
                )));
                return true;
            }
            _ => {}
        }
        false
    }
}

pub struct Anthropic {
    http: HttpClient,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
}

impl Anthropic {
    #[must_use]
    pub fn new(
        http: HttpClient,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
    ) -> Anthropic {
        Anthropic {
            http,
            secrets,
            clock,
        }
    }

    fn base(a: &ModelAccount) -> &str {
        a.base_url.as_deref().unwrap_or(DEFAULT_BASE)
    }

    fn body(req: &ModelRequest) -> Vec<u8> {
        let messages: Vec<Value> = req
            .history
            .iter()
            .map(|t| {
                let role = match t.role {
                    TurnRole::User => "user",
                    TurnRole::Assistant => "assistant",
                };
                json!({"role": role, "content": t.text})
            })
            .collect();
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "messages": messages,
        });
        if let Some(s) = &req.system {
            body["system"] = json!(s);
        }
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({"name": t.name, "description": t.description, "input_schema": t.schema})
                })
                .collect();
        }
        body.to_string().into_bytes()
    }

    async fn run(
        &self,
        req: ModelRequest,
        tx: &mpsc::Sender<ProviderEvent>,
    ) -> Result<(), ProviderError> {
        let key = key_for(self.secrets.as_ref(), &req.account, true).await?;
        let key = key.map(|k| k.expose().clone()).unwrap_or_default();
        let headers = [
            ("x-api-key", key),
            ("anthropic-version", VERSION.to_string()),
        ];
        let url = join_url(Self::base(&req.account), "/v1/messages");
        let (reply, mut stream) = self.http.open_sse(&url, &headers, Self::body(&req)).await?;
        let limits = anthropic_limits(&reply.headers, self.clock.now_unix());
        if !limits.is_empty() && !emit(tx, ProviderEvent::RateLimit(limits)).await {
            return Ok(());
        }
        let mut parser = AnthropicParser::new();
        while let Some(ev) = stream.next().await? {
            let mut out = Vec::new();
            let done = parser.on_event(&ev, &mut out);
            for e in out {
                if !emit(tx, e).await {
                    return Ok(());
                }
            }
            if done {
                return Ok(());
            }
        }
        // The server closed the connection without `message_stop`: the response is truncated.
        Err(ProviderError::new("network", true))
    }
}

impl ModelProvider for Anthropic {
    fn stream<'a>(
        &'a self,
        req: ModelRequest,
        tx: mpsc::Sender<ProviderEvent>,
    ) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if let Err(e) = self.run(req, &tx).await {
                emit(&tx, ProviderEvent::Error(e)).await;
            }
        })
    }

    fn check<'a>(&'a self, a: &'a ModelAccount) -> PortFuture<'a, Result<(), ProviderError>> {
        Box::pin(async move {
            let key = key_for(self.secrets.as_ref(), a, true).await?;
            let key = key.map(|k| k.expose().clone()).unwrap_or_default();
            let headers = [
                ("x-api-key", key),
                ("anthropic-version", VERSION.to_string()),
            ];
            let status = self
                .http
                .get_status(&join_url(Self::base(a), "/v1/models?limit=1"), &headers)
                .await?;
            if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(ProviderError::from_status(status))
            }
        })
    }
}
