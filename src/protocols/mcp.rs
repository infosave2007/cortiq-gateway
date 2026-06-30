//! MCP (Model Context Protocol) server: `POST /mcp` (JSON-RPC 2.0).
//!
//! Exposes the gateway's routing to MCP-native orchestrators as two tools:
//! `cortiq_route` (classify a prompt → routing decision) and `cortiq_chat`
//! (run a prompt through intelligent routing → answer). Single JSON endpoint;
//! notifications are accepted with `202`. Gated by `protocols.mcp`.

use crate::model::{ChatRequest, GenParams, Message, RequestMeta, RoutingDirective};
use crate::state::SharedState;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};

pub fn routes() -> Router<SharedState> {
    Router::new().route("/mcp", post(handler))
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

fn tools_list() -> Value {
    json!([
        {
            "name": "cortiq_route",
            "description": "Classify a prompt and return the routing decision (task type, complexity tier, score, and the model the gateway would select).",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string", "description": "The prompt to classify." } },
                "required": ["text"]
            }
        },
        {
            "name": "cortiq_chat",
            "description": "Run a prompt through the gateway's intelligent routing and return the selected model's answer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The user prompt." },
                    "model": { "type": "string", "description": "Optional: cortiq-auto[:profile] or a model id (default cortiq-auto)." }
                },
                "required": ["prompt"]
            }
        }
    ])
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

async fn handle_tool_call(
    state: &SharedState,
    params: &Value,
) -> std::result::Result<Value, (i64, String)> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];
    let live = state.live();
    match name {
        "cortiq_route" => {
            let text = args["text"].as_str().unwrap_or("");
            let prof = live.cfg.route.profile.clone();
            match live.router.route(text, &prof).await {
                Ok(Some(d)) => Ok(tool_text(
                    format!(
                        "task_label={}\ncomplexity_tier={}\ncomplexity_score={:.3}\ncandidates={:?}",
                        d.task_label,
                        d.complexity_tier,
                        d.complexity_score,
                        live.routing.candidates(&d.complexity_tier)
                    ),
                    false,
                )),
                _ => Ok(tool_text(
                    "router unavailable — would use the default model".to_string(),
                    false,
                )),
            }
        }
        "cortiq_chat" => {
            let prompt = args["prompt"].as_str().unwrap_or("");
            let model = args["model"].as_str().unwrap_or("cortiq-auto");
            let req = ChatRequest {
                routing: parse_routing(model),
                messages: vec![Message {
                    role: "user".into(),
                    content: prompt.to_string(),
                    tool_calls: vec![],
                }],
                tools: vec![],
                params: GenParams::default(),
                stream: false,
                meta: RequestMeta {
                    account: "mcp".into(),
                    protocol: "mcp".into(),
                    ..Default::default()
                },
            };
            match state.pipeline.run(req, state).await {
                Ok(resp) => {
                    let answer = resp
                        .choices
                        .first()
                        .map(|c| c.message.content.clone())
                        .unwrap_or_default();
                    Ok(tool_text(answer, false))
                }
                Err(e) => Ok(tool_text(e.to_string(), true)),
            }
        }
        _ => Err((-32602, format!("unknown tool '{name}'"))),
    }
}

async fn handler(State(state): State<SharedState>, Json(req): Json<Value>) -> Response {
    if !state.live().cfg.protocols.mcp {
        return (StatusCode::NOT_FOUND, "mcp protocol is disabled").into_response();
    }

    let method = req["method"].as_str().unwrap_or("").to_string();
    let id = req.get("id").cloned();

    // JSON-RPC notifications carry no id and expect no response
    if id.is_none() || method.starts_with("notifications/") {
        return StatusCode::ACCEPTED.into_response();
    }
    let id = id.unwrap_or(Value::Null);

    let result: std::result::Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => {
            let pv = req["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2024-11-05")
                .to_string();
            Ok(json!({
                "protocolVersion": pv,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "cortiq-gateway", "version": env!("CARGO_PKG_VERSION") }
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools_list() })),
        "tools/call" => handle_tool_call(&state, &req["params"]).await,
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let body = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    };
    Json(body).into_response()
}
