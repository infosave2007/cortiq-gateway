//! Inbound OpenAI Completions (legacy) adapter: `POST /v1/completions`.
//! Converts the legacy `prompt` into a single user message, runs the pipeline, and
//! returns the legacy `text_completion` shape (non-streaming JSON or streaming SSE).

use crate::error::{GatewayError, Result};
use crate::model::{ChatRequest, GenParams, Message, RequestMeta, RoutingDirective};
use crate::protocols::openai_chat::cortiq_headers;
use crate::state::SharedState;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<SharedState> {
    Router::new().route("/v1/completions", post(handler))
}

#[derive(Deserialize)]
struct CompletionsRequest {
    model: String,
    prompt: serde_json::Value,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    stream: bool,
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

fn prompt_to_string(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

async fn handler(
    State(state): State<SharedState>,
    Json(req): Json<CompletionsRequest>,
) -> Result<Response> {
    if !state.live().cfg.protocols.openai_completions {
        return Err(GatewayError::InvalidRequest(
            "openai_completions protocol is disabled".into(),
        ));
    }
    let prompt = prompt_to_string(&req.prompt);
    if prompt.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "prompt must not be empty".into(),
        ));
    }

    let canonical = ChatRequest {
        routing: parse_routing(&req.model),
        messages: vec![Message {
            role: "user".into(),
            content: prompt,
            tool_calls: vec![],
        }],
        tools: vec![],
        params: GenParams {
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            top_p: req.top_p,
            think_budget: None,
            stop: Vec::new(),
            passthrough: Default::default(),
        },
        stream: req.stream,
        meta: RequestMeta {
            protocol: "openai_completions".into(),
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
        return Ok((
            headers,
            axum::body::Body::from_stream(chat_to_completions_sse(stream)),
        )
            .into_response());
    }

    let resp = state.pipeline.run(canonical, &state).await?;
    let answer = resp
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();
    let finish = resp
        .choices
        .first()
        .map(|c| c.finish_reason.clone())
        .unwrap_or_else(|| "stop".into());
    let headers = cortiq_headers(&resp.cortiq);
    let body = serde_json::json!({
        "id": resp.id,
        "object": "text_completion",
        "created": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "model": resp.model_used,
        "choices": [{ "text": answer, "index": 0, "logprobs": serde_json::Value::Null, "finish_reason": finish }],
        "usage": {
            "prompt_tokens": resp.usage.prompt_tokens,
            "completion_tokens": resp.usage.completion_tokens,
            "total_tokens": resp.usage.total_tokens,
        },
    });
    Ok((headers, Json(body)).into_response())
}

/// Translate chat `chat.completion.chunk` SSE into legacy `text_completion` SSE
/// (`delta.content` → `text`).
fn chat_to_completions_sse(
    stream: crate::providers::ChatStream,
) -> impl futures::Stream<Item = Result<bytes::Bytes>> + Send + 'static {
    use futures::StreamExt;
    async_stream::stream! {
        let mut buf = String::new();
        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            let bytes = match item { Ok(b) => b, Err(e) => { yield Err(e); break; } };
            if let Ok(s) = std::str::from_utf8(&bytes) {
                buf.push_str(s);
            }
            while let Some(idx) = buf.find("\n\n") {
                let chunk: String = buf.drain(..idx + 2).collect();
                for line in chunk.lines() {
                    let Some(data) = line.trim_start().strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data.is_empty() { continue; }
                    if data == "[DONE]" {
                        yield Ok(bytes::Bytes::from("data: [DONE]\n\n"));
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
                    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                        yield Ok(bytes::Bytes::from(format!(
                            "data: {}\n\n",
                            serde_json::json!({"object": "text_completion", "choices": [], "usage": u})
                        )));
                    }
                    if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                        yield Ok(bytes::Bytes::from(format!(
                            "data: {}\n\n",
                            serde_json::json!({
                                "object": "text_completion",
                                "choices": [{"text": t, "index": 0, "finish_reason": null}]
                            })
                        )));
                    }
                }
            }
        }
    }
}
