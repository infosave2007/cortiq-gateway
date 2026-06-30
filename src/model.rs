//! Canonical (protocol-neutral) request/response model.
//!
//! All incoming protocols are translated into these types, and providers translate
//! from them. This way N protocols × M providers do not become N×M translations:
//! each adapter only knows its own protocol ↔ canonical mapping. See docs/ARCHITECTURE.md §3.

use serde::{Deserialize, Serialize};

/// How to handle routing for a specific request.
#[derive(Clone, Debug)]
pub enum RoutingDirective {
    /// `model = "cortiq-auto"` — ask cortiq-router and pick a model from the pool.
    Auto { profile: Option<String> },
    /// `model = "<real id>"` — call a specific model directly, bypassing routing.
    Pinned { model_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // system | user | assistant | tool
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Default)]
pub struct GenParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub stop: Vec<String>,
    /// Fields we do not interpret but must proxy through to the provider.
    pub passthrough: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestMeta {
    pub account: String,
    pub protocol: String,
    pub idempotency_key: Option<String>,
    pub traceparent: Option<String>,
}

/// Canonical form of a generation request.
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub routing: RoutingDirective,
    pub messages: Vec<Message>,
    pub tools: Vec<serde_json::Value>,
    pub params: GenParams,
    pub stream: bool,
    pub meta: RequestMeta,
}

#[derive(Clone, Debug, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Routing metadata attached to the response (X-Cortiq-* headers and the `cortiq` field).
#[derive(Clone, Debug, Serialize)]
pub struct RouteInfo {
    pub task_label: String,
    pub complexity_score: f32,
    pub complexity_tier: String,
    pub selected_model: String,
    pub route_source: String, // router | cache | fallback | pinned
    pub router_request_id: Option<String>,
    pub cost_usd: f64,
    #[serde(default)]
    pub failover: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: String,
}

/// Canonical form of a response.
#[derive(Clone, Debug)]
pub struct ChatResponse {
    pub id: String,
    pub model_used: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    pub cortiq: RouteInfo,
}

/// Decision from cortiq-router (the subset of its contract needed by the gateway).
#[derive(Clone, Debug)]
pub struct RouteDecision {
    pub task_label: String,
    pub complexity_score: f32,
    pub complexity_tier: String,
    pub router_request_id: Option<String>,
    pub source: String, // router | cache | fallback
}
