//! Outgoing provider adapters. The [`Provider`] trait is the single call site for LLMs;
//! adapters translate the canonical [`ChatRequest`] to and from each provider's API format.
//!
//! Most local servers (vLLM, llama.cpp, LM Studio, Ollama `/v1`) are OpenAI-compatible
//! → [`openai::OpenAiProvider`] is sufficient for all of them.

pub mod anthropic;
pub mod openai;

use crate::model::{ChatRequest, ChatResponse};
use async_trait::async_trait;

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

    // TODO(v0.1): async fn chat_stream(...) -> Result<ChatStream> (SSE).
    // TODO(v0.1): async fn embed(...) -> Result<EmbedResponse>.
}
