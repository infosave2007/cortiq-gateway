//! Anthropic Messages provider. Translates the canonical [`ChatRequest`] to
//! `POST {base_url}/v1/messages` and back. `system` messages are lifted into the
//! top-level `system` field. Streaming translates Anthropic SSE events into the
//! OpenAI wire format, so the gateway's unified SSE passthrough works uniformly.

use super::{Caps, Provider};
use crate::config::ModelCfg;
use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, ChatResponse, Choice, Message, RouteInfo, Usage};
use async_trait::async_trait;

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    id: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
    caps: Caps,
    price_in: f64,
    price_out: f64,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(cfg: &ModelCfg, api_key: Option<String>) -> anyhow::Result<Self> {
        Ok(Self {
            id: cfg.id.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            model: cfg.model.clone(),
            api_key,
            caps: Caps::from_list(&cfg.caps),
            price_in: cfg.price_in,
            price_out: cfg.price_out,
            http: reqwest::Client::new(),
        })
    }

    /// Canonical request → Anthropic Messages request body.
    fn to_wire(&self, req: &ChatRequest, stream: bool) -> serde_json::Value {
        let mut system = String::new();
        let mut messages = Vec::new();
        for m in &req.messages {
            if m.role == "system" {
                if !system.is_empty() {
                    system.push('\n');
                }
                system.push_str(&m.content);
            } else {
                let role = if m.role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                messages.push(serde_json::json!({ "role": role, "content": m.content }));
            }
        }
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": req.params.max_tokens.unwrap_or(1024),
            "stream": stream,
        });
        if !system.is_empty() {
            body["system"] = serde_json::Value::String(system);
        }
        if let Some(t) = req.params.temperature {
            body["temperature"] = t.into();
        }
        if let Some(p) = req.params.top_p {
            body["top_p"] = p.into();
        }
        if !req.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(req.tools.clone());
        }
        body
    }

    fn request(&self, body: &serde_json::Value) -> reqwest::RequestBuilder {
        let mut r = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(body);
        if let Some(key) = &self.api_key {
            r = r.header("x-api-key", key);
        }
        r
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn caps(&self) -> &Caps {
        &self.caps
    }
    fn price(&self, prompt_tokens: u32, completion_tokens: u32) -> f64 {
        (prompt_tokens as f64 * self.price_in + completion_tokens as f64 * self.price_out)
            / 1_000_000.0
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let resp = self
            .request(&self.to_wire(&req, false))
            .send()
            .await
            .map_err(|e| GatewayError::UpstreamUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamUnavailable(format!(
                "provider {} returned HTTP {}: {}",
                self.id, status, b
            )));
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| {
            GatewayError::Internal(format!("failed to parse anthropic response: {e}"))
        })?;
        let content = v["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let prompt_tokens = v["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let completion_tokens = v["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
        Ok(ChatResponse {
            id: v["id"].as_str().unwrap_or("").to_string(),
            model_used: v["model"].as_str().unwrap_or(&self.model).to_string(),
            choices: vec![Choice {
                index: 0,
                message: Message {
                    role: "assistant".into(),
                    content,
                    tool_calls: vec![],
                },
                finish_reason: v["stop_reason"].as_str().unwrap_or("stop").to_string(),
            }],
            usage: Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
            cortiq: RouteInfo {
                task_label: String::new(),
                complexity_score: 0.0,
                complexity_tier: String::new(),
                selected_model: self.id.clone(),
                route_source: String::new(),
                router_request_id: None,
                cost_usd: 0.0,
                failover: false,
            },
        })
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<super::ChatStream> {
        let resp = self
            .request(&self.to_wire(&req, true))
            .send()
            .await
            .map_err(|e| GatewayError::UpstreamUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let b = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamUnavailable(format!(
                "provider {} returned HTTP {}: {}",
                self.id, status, b
            )));
        }
        Ok(Box::pin(anthropic_to_openai_sse(resp.bytes_stream())))
    }
}

/// Translate an Anthropic Messages SSE stream into OpenAI `chat.completion.chunk`
/// SSE so the gateway's unified streaming path works for Anthropic too.
fn anthropic_to_openai_sse(
    upstream: impl futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
) -> impl futures::Stream<Item = Result<bytes::Bytes>> + Send + 'static {
    use futures::StreamExt;
    async_stream::stream! {
        futures::pin_mut!(upstream);
        let mut buf = String::new();
        let mut input_tokens = 0u64;
        let mut role_sent = false;
        let emit = |obj: serde_json::Value| bytes::Bytes::from(format!("data: {obj}\n\n"));

        while let Some(chunk) = upstream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(GatewayError::UpstreamUnavailable(e.to_string()));
                    return;
                }
            };
            if let Ok(s) = std::str::from_utf8(&chunk) {
                buf.push_str(s);
            }
            while let Some(idx) = buf.find("\n\n") {
                let evt: String = buf.drain(..idx + 2).collect();
                for line in evt.lines() {
                    let Some(data) = line.trim_start().strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };
                    match v["type"].as_str().unwrap_or("") {
                        "message_start" => {
                            input_tokens =
                                v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0);
                            if !role_sent {
                                role_sent = true;
                                yield Ok(emit(serde_json::json!({
                                    "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
                                })));
                            }
                        }
                        "content_block_delta" => {
                            if let Some(t) = v["delta"]["text"].as_str() {
                                yield Ok(emit(serde_json::json!({
                                    "choices": [{"index": 0, "delta": {"content": t}, "finish_reason": null}]
                                })));
                            }
                        }
                        "message_delta" => {
                            let out = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
                            let stop = v["delta"]["stop_reason"].as_str().unwrap_or("stop").to_string();
                            yield Ok(emit(serde_json::json!({
                                "choices": [{"index": 0, "delta": {}, "finish_reason": stop}]
                            })));
                            yield Ok(emit(serde_json::json!({
                                "choices": [],
                                "usage": {
                                    "prompt_tokens": input_tokens,
                                    "completion_tokens": out,
                                    "total_tokens": input_tokens + out
                                }
                            })));
                        }
                        _ => {}
                    }
                }
            }
        }
        yield Ok(bytes::Bytes::from("data: [DONE]\n\n"));
    }
}
