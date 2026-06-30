//! Inbound OpenAI Embeddings adapter: `POST /v1/embeddings`.
//! Resolves an embedding model (explicit id, or the first `kind = "embedding"`),
//! calls the provider, and forwards the OpenAI-style response body.

use crate::error::{GatewayError, Result};
use crate::state::SharedState;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<SharedState> {
    Router::new().route("/v1/embeddings", post(handler))
}

#[derive(Deserialize)]
struct EmbeddingsRequest {
    #[serde(default)]
    model: String,
    input: serde_json::Value,
}

async fn handler(
    State(state): State<SharedState>,
    Json(req): Json<EmbeddingsRequest>,
) -> Result<impl IntoResponse> {
    let live = state.live();
    if !live.cfg.protocols.openai_embeddings {
        return Err(GatewayError::InvalidRequest(
            "openai_embeddings protocol is disabled".into(),
        ));
    }

    // resolve the embedding model: explicit real id, else first kind == "embedding"
    let model_id = if !req.model.is_empty()
        && req.model != "cortiq-auto"
        && live.registry.get(&req.model).is_some()
    {
        req.model.clone()
    } else {
        live.cfg
            .models
            .iter()
            .find(|m| m.kind == "embedding")
            .map(|m| m.id.clone())
            .ok_or_else(|| GatewayError::InvalidRequest("no embedding model configured".into()))?
    };

    let provider = live.registry.get(&model_id).ok_or_else(|| {
        GatewayError::UpstreamUnavailable(format!("embedding model '{model_id}' is not available"))
    })?;
    let body = provider.embed(req.input).await?;
    Ok(Json(body))
}
