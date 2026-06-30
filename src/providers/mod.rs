//! Outgoing provider adapters. The [`Provider`] trait is the single call site for LLMs;
//! adapters translate the canonical [`ChatRequest`] to and from each provider's API format.
//!
//! Most local servers (vLLM, llama.cpp, LM Studio, Ollama `/v1`) are OpenAI-compatible
//! → [`openai::OpenAiProvider`] is sufficient for all of them.

pub mod anthropic;
pub mod openai;

use crate::error::GatewayError;
use crate::model::{ChatRequest, ChatResponse};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

/// A streaming chat response: raw Server-Sent-Events byte chunks in OpenAI wire
/// format (`data: {json}\n\n` … `data: [DONE]\n\n`). The gateway forwards these
/// to the client verbatim while tapping the final `usage` for statistics.
pub type ChatStream = BoxStream<'static, crate::error::Result<Bytes>>;

/// Model capabilities — checked at selection time (whether tools/vision are needed).
#[derive(Clone, Debug, Default)]
pub struct Caps {
    pub tools: bool,
    pub vision: bool,
    pub streaming: bool,
}

impl Caps {
    pub fn from_list(list: &[String]) -> Self {
        Self {
            tools: list.iter().any(|c| c == "tools"),
            vision: list.iter().any(|c| c == "vision"),
            streaming: true,
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// The model_id from config (not the provider's model name).
    fn id(&self) -> &str;

    fn caps(&self) -> &Caps;

    /// Cost of a request in USD — computed from price_in/price_out × tokens (for billing).
    fn price(&self, prompt_tokens: u32, completion_tokens: u32) -> f64;

    /// Non-streaming chat call.
    async fn chat(&self, req: ChatRequest) -> crate::error::Result<ChatResponse>;

    /// Streaming chat call (SSE). Default: not supported (override per provider).
    async fn chat_stream(&self, _req: ChatRequest) -> crate::error::Result<ChatStream> {
        Err(GatewayError::UpstreamUnavailable(format!(
            "streaming is not supported by provider '{}'",
            self.id()
        )))
    }

    /// Embeddings call. `input` is the OpenAI `input` field (string or array of
    /// strings); returns the provider's OpenAI-style embeddings response body.
    /// Default: not supported (override per provider).
    async fn embed(&self, _input: serde_json::Value) -> crate::error::Result<serde_json::Value> {
        Err(GatewayError::UpstreamUnavailable(format!(
            "embeddings are not supported by provider '{}'",
            self.id()
        )))
    }
}
