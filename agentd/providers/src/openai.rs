//! OpenAI-compatible provider (chat completions, streaming) and local server (Ollama, llama.cpp).

use std::sync::Arc;

use agentd_core::{
    Clock, ModelAccount, ModelProvider, ModelRequest, PortFuture, ProviderError, ProviderEvent,
    SecretStore, ToolCall, TurnRole, Usage,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::common::{emit, join_url, key_for};
use crate::http::HttpClient;
use crate::limits::openai_limits;
use crate::sse::SseEvent;

const DEFAULT_BASE: &str = "https://api.openai.com/v1";
/// No more calls are parsed per reply.
const MAX_CALLS: usize = 32;
const DEFAULT_LOCAL_BASE: &str = "http://127.0.0.1:11434/v1";

#[derive(Debug, Default)]
pub struct OpenAiParser {
    /// Tool calls assembled from `delta.tool_calls` chunks by `index`: `(id, name, argument JSON)`.
    calls: Vec<(String, String, String)>,
}

fn error_of(v: &Value) -> ProviderError {
    let kind = v["type"].as_str().or(v["code"].as_str()).unwrap_or("");
    match kind {
        "invalid_api_key" | "authentication_error" | "invalid_authentication" => {
            ProviderError::new("unauthorized", false)
        }
        "rate_limit_exceeded" | "insufficient_quota" | "requests" | "tokens" => {
            ProviderError::new("rate_limited", true)
        }
        "server_error" | "service_unavailable" => ProviderError::new("upstream", true),
        _ => ProviderError::new("bad_response", false),
    }
}

impl OpenAiParser {
    #[must_use]
    pub fn new() -> OpenAiParser {
        OpenAiParser::default()
    }

    /// Assembled calls are emitted as events; truncated or broken arguments do not count as a call.
    fn flush_calls(&mut self, out: &mut Vec<ProviderEvent>) {
        for (id, name, json) in std::mem::take(&mut self.calls) {
            let raw = if json.trim().is_empty() { "{}" } else { &json };
            if let Ok(args) = serde_json::from_str::<Value>(raw)
                && args.is_object()
                && !name.is_empty()
            {
                out.push(ProviderEvent::ToolCall(ToolCall { id, name, args }));
            }
        }
    }

    fn collect_calls(&mut self, delta: &Value) {
        let Some(list) = delta["tool_calls"].as_array() else {
            return;
        };
        for c in list {
            let idx = usize::try_from(c["index"].as_u64().unwrap_or(0)).unwrap_or(0);
            // The index comes from the server: bounded so a foreign stream cannot bloat memory.
            if idx >= MAX_CALLS {
                continue;
            }
            while self.calls.len() <= idx {
                self.calls.push(Default::default());
            }
            let slot = &mut self.calls[idx];
            if let Some(id) = c["id"].as_str() {
                slot.0 = id.to_string();
            }
            if let Some(n) = c["function"]["name"].as_str() {
                slot.1.push_str(n);
            }
            if let Some(a) = c["function"]["arguments"].as_str() {
                slot.2.push_str(a);
            }
        }
    }

    /// Returns `true` on `[DONE]` and on an error.
    pub fn on_event(&mut self, ev: &SseEvent, out: &mut Vec<ProviderEvent>) -> bool {
        if ev.data.trim() == "[DONE]" {
            self.flush_calls(out);
            return true;
        }
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            out.push(ProviderEvent::Error(ProviderError::new(
                "bad_response",
                false,
            )));
            return true;
        };
        if v.get("error").is_some_and(|e| !e.is_null()) {
            out.push(ProviderEvent::Error(error_of(&v["error"])));
            return true;
        }
        self.collect_calls(&v["choices"][0]["delta"]);
        if v["choices"][0]["finish_reason"].is_string() {
            self.flush_calls(out);
        }
        if let Some(t) = v["choices"][0]["delta"]["content"]
            .as_str()
            .filter(|t| !t.is_empty())
        {
            out.push(ProviderEvent::Delta(t.to_string()));
        }
        if v["usage"].is_object() {
            out.push(ProviderEvent::Usage(Usage {
                tokens_in: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                tokens_out: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            }));
        }
        false
    }
}

pub struct OpenAiCompat {
    http: HttpClient,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    local: bool,
}

impl OpenAiCompat {
    /// `local = true` is a local server: the key is optional, no limits.
    #[must_use]
    pub fn new(
        http: HttpClient,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        local: bool,
    ) -> OpenAiCompat {
        OpenAiCompat {
            http,
            secrets,
            clock,
            local,
        }
    }

    fn base<'a>(&self, a: &'a ModelAccount) -> &'a str {
        a.base_url.as_deref().unwrap_or(if self.local {
            DEFAULT_LOCAL_BASE
        } else {
            DEFAULT_BASE
        })
    }

    fn body(req: &ModelRequest) -> Vec<u8> {
        let mut messages: Vec<Value> = Vec::new();
        if let Some(s) = &req.system {
            messages.push(json!({"role": "system", "content": s}));
        }
        for t in &req.history {
            let role = match t.role {
                TurnRole::User => "user",
                TurnRole::Assistant => "assistant",
            };
            messages.push(json!({"role": role, "content": t.text}));
        }
        let mut body = json!({
            "model": req.model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
            "max_tokens": req.max_tokens,
        });
        if !req.tools.is_empty() {
            body["tools"] = req
                .tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.schema}})
                })
                .collect();
        }
        body.to_string().into_bytes()
    }

    async fn headers(
        &self,
        a: &ModelAccount,
    ) -> Result<Vec<(&'static str, String)>, ProviderError> {
        let key = key_for(self.secrets.as_ref(), a, !self.local).await?;
        Ok(key
            .map(|k| vec![("authorization", format!("Bearer {}", k.expose()))])
            .unwrap_or_default())
    }

    async fn run(
        &self,
        req: ModelRequest,
        tx: &mpsc::Sender<ProviderEvent>,
    ) -> Result<(), ProviderError> {
        let headers = self.headers(&req.account).await?;
        let url = join_url(self.base(&req.account), "/chat/completions");
        let (reply, mut stream) = self.http.open_sse(&url, &headers, Self::body(&req)).await?;
        let limits = openai_limits(&reply.headers, self.clock.now_unix());
        if !self.local && !limits.is_empty() && !emit(tx, ProviderEvent::RateLimit(limits)).await {
            return Ok(());
        }
        let mut parser = OpenAiParser::new();
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
        Err(ProviderError::new("network", true))
    }
}

impl ModelProvider for OpenAiCompat {
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
            let headers = self.headers(a).await?;
            let status = self
                .http
                .get_status(&join_url(self.base(a), "/models"), &headers)
                .await?;
            if (200..300).contains(&status) {
                Ok(())
            } else {
                Err(ProviderError::from_status(status))
            }
        })
    }
}
