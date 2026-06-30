//! Anthropic Messages provider (v0.2). Translates the canonical ChatRequest to
//! `POST /v1/messages` and back. The `system` message is extracted into a separate field.

use super::{Caps, Provider};
use crate::config::ModelCfg;
use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, ChatResponse};
use async_trait::async_trait;

pub struct AnthropicProvider {
    id: String,
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    api_key: Option<String>,
    caps: Caps,
    price_in: f64,
    price_out: f64,
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
        })
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
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse> {
        // TODO(v0.2): extract system from messages → "system" field; messages → content blocks;
        // add x-api-key + anthropic-version headers; map response → canonical form.
        // Returns an error (not a panic) so that failover/playground continue to work normally.
        Err(GatewayError::UpstreamUnavailable(
            "anthropic provider not yet implemented (planned v0.2)".to_string(),
        ))
    }
}
