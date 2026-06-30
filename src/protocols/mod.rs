//! Incoming protocol adapters. Each one builds its own axum routes and translates
//! its protocol to/from the canonical model. Enabled via flags in `[protocols]`.

pub mod anthropic_messages;
pub mod mcp;
pub mod native_passthrough;
pub mod openai_chat;
pub mod openai_completions;
pub mod openai_embeddings;
pub mod openai_models;

use crate::state::SharedState;
use axum::Router;

/// Build the protocols router. Implemented adapters are **always** mounted;
/// each handler checks the live `protocols.*` flag from config itself, so
/// toggles in the admin panel take effect without a restart (hot switching).
pub fn build_router() -> Router<SharedState> {
    Router::new()
        .merge(openai_chat::routes())
        .merge(openai_completions::routes())
        .merge(openai_embeddings::routes())
        .merge(openai_models::routes())
        .merge(anthropic_messages::routes())
        .merge(mcp::routes())
        .merge(native_passthrough::routes())
}
