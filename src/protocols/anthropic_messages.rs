//! Inbound Anthropic Messages adapter: `POST /v1/messages`.
//! Translates the Anthropic request into the canonical [`ChatRequest`], runs the
//! pipeline, and converts the result back to the Anthropic Messages format
//! (non-streaming JSON or streaming SSE).

use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, GenParams, Message, RequestMeta, RoutingDirective};
use crate::protocols::openai_chat::cortiq_headers;
use crate::state::SharedState;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<SharedState> {
    Router::new().route("/v1/messages", post(handler))
}

#[derive(Deserialize)]
struct MessagesRequest {
    model: String,
    #[serde(default)]
    system: serde_json::Value,
    #[serde(default)]
    messages: Vec<serde_json::Value>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    tools: Vec<serde_json::Value>,
}

/// Anthropic content is a string or an array of blocks; flatten to text.
fn content_to_string(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

fn parse_routing(model: &str) -> RoutingDirective {
    if let Some(rest) = model.strip_prefix("cortiq-auto") {
        RoutingDirective::Auto {
            profile: rest.strip_prefix(':').map(|p| p.to_string()),
        }
    } else {
        RoutingDirective::Pinned {
            model_id: model.to_string(),
        }
    }
}

async fn handler(
    State(state): State<SharedState>,
    Json(req): Json<MessagesRequest>,
) -> Result<Response> {
    if !state.live().cfg.protocols.anthropic_messages {
        return Err(GatewayError::InvalidRequest(
            "anthropic_messages protocol is disabled".into(),
        ));
    }
    if req.messages.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "messages must not be empty".into(),
        ));
    }

    let mut messages = Vec::new();
    let sys = content_to_string(&req.system);
    if !sys.is_empty() {
        messages.push(Message {
            role: "system".into(),
            content: sys,
            tool_calls: vec![],
        });
    }
    for m in &req.messages {
        messages.push(Message {
            role: m["role"].as_str().unwrap_or("user").to_string(),
            content: content_to_string(&m["content"]),
            tool_calls: vec![],
        });
    }

    let canonical = ChatRequest {
        routing: parse_routing(&req.model),
        messages,
        tools: req.tools,
        params: GenParams {
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            stop: Vec::new(),
            passthrough: Default::default(),
        },
        stream: req.stream,
        meta: RequestMeta {
            protocol: "anthropic_messages".into(),
            ..Default::default()
        },
    };

    if req.stream {
        let (info, stream) = state.pipeline.run_stream(canonical, &state).await?;
        let mut headers = cortiq_headers(&info);
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        let translated = openai_to_anthropic_sse(stream);
        return Ok((headers, axum::body::Body::from_stream(translated)).into_response());
    }

    let resp = state.pipeline.run(canonical, &state).await?;
    let answer = resp
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();
    let headers = cortiq_headers(&resp.cortiq);
    let body = serde_json::json!({
        "id": resp.id,
        "type": "message",
        "role": "assistant",
        "model": resp.model_used,
        "content": [{ "type": "text", "text": answer }],
        "stop_reason": "end_turn",
        "stop_sequence": serde_json::Value::Null,
        "usage": {
            "input_tokens": resp.usage.prompt_tokens,
            "output_tokens": resp.usage.completion_tokens,
        },
    });
    Ok((headers, Json(body)).into_response())
}

/// Translate the gateway's internal OpenAI-format SSE into Anthropic Messages SSE
/// events, so Anthropic-native clients can stream regardless of the upstream provider.
fn openai_to_anthropic_sse(
    stream: crate::providers::ChatStream,
) -> impl futures::Stream<Item = Result<bytes::Bytes>> + Send + 'static {
    use futures::StreamExt;
    async_stream::stream! {
        fn evt(event: &str, data: serde_json::Value) -> bytes::Bytes {
            bytes::Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
        }

        yield Ok(evt("message_start", serde_json::json!({
            "type": "message_start",
            "message": {"id": "msg_gw", "type": "message", "role": "assistant",
                        "content": [], "model": "cortiq-auto", "stop_reason": null,
                        "usage": {"input_tokens": 0, "output_tokens": 0}}
        })));
        yield Ok(evt("content_block_start", serde_json::json!({
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}
        })));

        let mut buf = String::new();
        let mut completion_tokens = 0u64;
        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => { yield Err(e); break; }
            };
            if let Ok(s) = std::str::from_utf8(&bytes) {
                buf.push_str(s);
            }
            while let Some(idx) = buf.find("\n\n") {
                let chunk: String = buf.drain(..idx + 2).collect();
                for line in chunk.lines() {
                    let Some(data) = line.trim_start().strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data.is_empty() || data == "[DONE]" { continue; }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                        completion_tokens = u["completion_tokens"].as_u64().unwrap_or(completion_tokens);
                    }
                    if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                        yield Ok(evt("content_block_delta", serde_json::json!({
                            "type": "content_block_delta", "index": 0,
                            "delta": {"type": "text_delta", "text": t}
                        })));
                    }
                }
            }
        }

        yield Ok(evt("content_block_stop", serde_json::json!({"type": "content_block_stop", "index": 0})));
        yield Ok(evt("message_delta", serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": {"output_tokens": completion_tokens}
        })));
        yield Ok(evt("message_stop", serde_json::json!({"type": "message_stop"})));
    }
}
