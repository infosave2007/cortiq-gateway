//! OpenAI-compatible provider. Covers OpenAI, OpenRouter, Together, Groq,
//! and local servers such as vLLM / llama.cpp / LM Studio / Ollama (`/v1`).

use super::{Caps, Provider};
use crate::config::ModelCfg;
use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, ChatResponse};
use async_trait::async_trait;

pub struct OpenAiProvider {
    id: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
    caps: Caps,
    price_in: f64,
    price_out: f64,
    http: reqwest::Client,
}

impl OpenAiProvider {
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

    /// Canonical request → OpenAI Chat Completions request body.
    fn to_wire(&self, req: &ChatRequest) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": req.messages,
            "stream": req.stream,
        });
        if let Some(t) = req.params.temperature {
            body["temperature"] = t.into();
        }
        if let Some(m) = req.params.max_tokens {
            body["max_tokens"] = m.into();
        }
        if !req.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(req.tools.clone());
        }
        // uninterpreted fields are proxied as-is
        for (k, v) in &req.params.passthrough {
            body[k] = v.clone();
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
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
        let mut r = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&self.to_wire(&req));
        if let Some(key) = &self.api_key {
            r = r.bearer_auth(key);
        }
        let resp = r
            .send()
            .await
            .map_err(|e| GatewayError::UpstreamUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamUnavailable(format!(
                "provider {} returned HTTP {}: {}",
                self.id, status, err_body
            )));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::Internal(format!("Failed to parse JSON response: {}", e)))?;

        let id = v["id"].as_str().unwrap_or("").to_string();
        let model_used = v["model"].as_str().unwrap_or(&self.model).to_string();

        let choices_arr = v["choices"].as_array().ok_or_else(|| {
            GatewayError::Internal(
                "invalid openai response: choices is missing or not an array".to_string(),
            )
        })?;

        let mut choices = Vec::new();
        for (i, c) in choices_arr.iter().enumerate() {
            let msg_val = &c["message"];
            let role = msg_val["role"].as_str().unwrap_or("assistant").to_string();
            let content = msg_val["content"].as_str().unwrap_or("").to_string();

            let tool_calls = msg_val["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            choices.push(crate::model::Choice {
                index: c["index"].as_u64().unwrap_or(i as u64) as u32,
                message: crate::model::Message {
                    role,
                    content,
                    tool_calls,
                },
                finish_reason: c["finish_reason"].as_str().unwrap_or("stop").to_string(),
            });
        }

        let usage_val = &v["usage"];
        let prompt_tokens = usage_val["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let completion_tokens = usage_val["completion_tokens"].as_u64().unwrap_or(0) as u32;
        let total_tokens = usage_val["total_tokens"]
            .as_u64()
            .unwrap_or((prompt_tokens + completion_tokens) as u64)
            as u32;

        Ok(ChatResponse {
            id,
            model_used,
            choices,
            usage: crate::model::Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            },
            cortiq: crate::model::RouteInfo {
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
        use futures::StreamExt;
        let mut body = self.to_wire(&req);
        body["stream"] = serde_json::Value::Bool(true);
        // ask OpenAI-compatible servers to include token usage in the final chunk
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let mut r = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            r = r.bearer_auth(key);
        }
        let resp = r
            .send()
            .await
            .map_err(|e| GatewayError::UpstreamUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            return Err(GatewayError::UpstreamUnavailable(format!(
                "provider {} returned HTTP {}: {}",
                self.id, status, err_body
            )));
        }
        let stream = resp
            .bytes_stream()
            .map(|res| res.map_err(|e| GatewayError::UpstreamUnavailable(e.to_string())));
        Ok(Box::pin(stream))
    }
}
