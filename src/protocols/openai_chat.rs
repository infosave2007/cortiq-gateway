//! Incoming adapter for OpenAI Chat Completions: `POST /v1/chat/completions`.
//! Translates the OpenAI request body into a canonical [`ChatRequest`], passes it
//! to the pipeline, then converts the canonical [`ChatResponse`] back to OpenAI format.

use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, GenParams, Message, RequestMeta, RouteInfo, RoutingDirective};
use crate::state::SharedState;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

/// Build the `X-Cortiq-*` response headers from routing metadata.
pub(crate) fn cortiq_headers(c: &RouteInfo) -> axum::http::HeaderMap {
    use axum::http::{HeaderMap, HeaderValue};
    fn put(h: &mut HeaderMap, k: &'static str, v: &str) {
        if let Ok(val) = HeaderValue::from_str(v) {
            h.insert(k, val);
        }
    }
    let mut h = HeaderMap::new();
    put(&mut h, "X-Cortiq-Task-Label", &c.task_label);
    put(
        &mut h,
        "X-Cortiq-Complexity-Score",
        &c.complexity_score.to_string(),
    );
    put(&mut h, "X-Cortiq-Complexity-Tier", &c.complexity_tier);
    put(&mut h, "X-Cortiq-Selected-Model", &c.selected_model);
    put(&mut h, "X-Cortiq-Route-Source", &c.route_source);
    put(&mut h, "X-Cortiq-Cost-Usd", &c.cost_usd.to_string());
    if let Some(id) = &c.router_request_id {
        put(&mut h, "X-Cortiq-Request-Id", id);
    }
    h
}

pub fn routes() -> Router<SharedState> {
    Router::new().route("/v1/chat/completions", post(handler))
}

#[derive(Deserialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    tools: Vec<serde_json::Value>,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// Parse the `model` field value into a routing directive.
/// `cortiq-auto[:profile]` → Auto; otherwise → Pinned(real id).
fn parse_routing(model: &str) -> RoutingDirective {
    if let Some(rest) = model.strip_prefix("cortiq-auto") {
        let profile = rest.strip_prefix(':').map(|p| p.to_string());
        RoutingDirective::Auto { profile }
    } else {
        RoutingDirective::Pinned {
            model_id: model.to_string(),
        }
    }
}

async fn handler(
    State(state): State<SharedState>,
    Json(req): Json<OpenAiChatRequest>,
) -> Result<Response> {
    // hot protocol toggle: if the adapter is disabled in config — return 404
    if !state.live().cfg.protocols.openai_chat {
        return Err(GatewayError::InvalidRequest(
            "openai_chat protocol is disabled".into(),
        ));
    }
    if req.messages.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "messages must not be empty".into(),
        ));
    }

    let canonical = ChatRequest {
        routing: parse_routing(&req.model),
        messages: req.messages,
        tools: req.tools,
        params: GenParams {
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stop: Vec::new(),
            passthrough: req.rest,
        },
        stream: req.stream,
        meta: RequestMeta {
            protocol: "openai_chat".into(),
            ..Default::default()
        },
    };

    // streaming: forward provider SSE chunks verbatim with X-Cortiq-* headers
    if req.stream {
        let (info, stream) = state.pipeline.run_stream(canonical, &state).await?;
        let mut headers = cortiq_headers(&info);
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        return Ok((headers, axum::body::Body::from_stream(stream)).into_response());
    }

    let resp = state.pipeline.run(canonical, &state).await?;
    let headers = cortiq_headers(&resp.cortiq);

    let mut body = serde_json::json!({
        "id": resp.id,
        "object": "chat.completion",
        "created": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "model": resp.model_used,
        "choices": resp.choices.iter().map(|c| {
            serde_json::json!({
                "index": c.index,
                "message": {
                    "role": c.message.role,
                    "content": c.message.content,
                    "tool_calls": if c.message.tool_calls.is_empty() { serde_json::Value::Null } else { serde_json::Value::Array(c.message.tool_calls.clone()) },
                },
                "finish_reason": c.finish_reason,
            })
        }).collect::<Vec<_>>(),
        "usage": {
            "prompt_tokens": resp.usage.prompt_tokens,
            "completion_tokens": resp.usage.completion_tokens,
            "total_tokens": resp.usage.total_tokens,
        }
    });

    if state.live().cfg.cortiq.echo {
        body["cortiq"] = serde_json::json!({
            "task_label": resp.cortiq.task_label,
            "complexity": {
                "score": resp.cortiq.complexity_score,
                "tier": resp.cortiq.complexity_tier,
            },
            "selected_model": resp.cortiq.selected_model,
            "route_source": resp.cortiq.route_source,
            "router_request_id": resp.cortiq.router_request_id,
            "cost_usd": resp.cortiq.cost_usd,
        });
    }

    Ok((headers, Json(body)).into_response())
}
