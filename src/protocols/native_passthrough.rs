//! Native passthrough adapter: `POST /route`.
//! Returns the gateway's routing decision (task type, complexity, candidate models)
//! without calling a model — direct access for clients that route themselves.

use crate::error::{GatewayError, Result};
use crate::state::SharedState;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<SharedState> {
    Router::new().route("/route", post(handler))
}

#[derive(Deserialize)]
struct RouteRequest {
    /// `{ "input": { "text": "..." } }` (router-style) — optional.
    #[serde(default)]
    input: serde_json::Value,
    /// `{ "text": "..." }` — convenience shorthand.
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    profile: Option<String>,
}

async fn handler(
    State(state): State<SharedState>,
    Json(req): Json<RouteRequest>,
) -> Result<impl IntoResponse> {
    let live = state.live();
    if !live.cfg.protocols.native_passthrough {
        return Err(GatewayError::InvalidRequest(
            "native_passthrough protocol is disabled".into(),
        ));
    }

    let text = req
        .text
        .clone()
        .or_else(|| req.input["text"].as_str().map(|s| s.to_string()))
        .or_else(|| req.input.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if text.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "missing input text (use {\"text\": ...} or {\"input\": {\"text\": ...}})".into(),
        ));
    }

    let profile = req
        .profile
        .clone()
        .unwrap_or_else(|| live.cfg.route.profile.clone());

    let (decision, source, candidates) = match live.router.route(&text, &profile).await {
        Ok(Some(d)) => {
            let cands = live.routing.candidates(&d.complexity_tier);
            (Some(d), "router", cands)
        }
        _ => (
            None,
            "fallback",
            vec![live.routing.default_model().to_string()],
        ),
    };

    let selected = candidates.first().cloned().unwrap_or_default();
    let decision_json = match &decision {
        Some(d) => serde_json::json!({
            "task_label": d.task_label,
            "complexity": { "score": d.complexity_score, "tier": d.complexity_tier },
            "router_request_id": d.router_request_id,
        }),
        None => serde_json::json!({
            "task_label": "degraded",
            "complexity": { "score": 0.5, "tier": "degraded" },
            "router_request_id": serde_json::Value::Null,
        }),
    };

    Ok(Json(serde_json::json!({
        "decision": decision_json,
        "source": source,
        "candidates": candidates,
        "selected_model": selected,
    })))
}
