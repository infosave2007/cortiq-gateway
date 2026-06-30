//! Inbound OpenAI Models adapter: `GET /v1/models`.
//! Lists the configured pool (plus the virtual `cortiq-auto`) for client discovery.

use crate::error::{GatewayError, Result};
use crate::state::SharedState;
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};

pub fn routes() -> Router<SharedState> {
    Router::new().route("/v1/models", get(handler))
}

async fn handler(State(state): State<SharedState>) -> Result<impl IntoResponse> {
    let live = state.live();
    if !live.cfg.protocols.openai_models {
        return Err(GatewayError::InvalidRequest(
            "openai_models protocol is disabled".into(),
        ));
    }
    let mut data = vec![serde_json::json!({
        "id": "cortiq-auto",
        "object": "model",
        "owned_by": "cortiq",
    })];
    for m in &live.cfg.models {
        data.push(serde_json::json!({
            "id": m.id,
            "object": "model",
            "owned_by": m.provider,
        }));
    }
    Ok(Json(serde_json::json!({ "object": "list", "data": data })))
}
