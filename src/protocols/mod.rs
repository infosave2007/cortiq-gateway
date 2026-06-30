//! Incoming protocol adapters. Each one builds its own axum routes and translates
//! its protocol to/from the canonical model. Enabled via flags in `[protocols]`.

pub mod anthropic_messages;
pub mod mcp;
pub mod openai_chat;
pub mod openai_embeddings;

use crate::state::SharedState;
use axum::Router;

/// Build the protocols router. Implemented adapters are **always** mounted;
/// each handler checks the live `protocols.*` flag from config itself, so
/// toggles in the admin panel take effect without a restart (hot switching).
pub fn build_router() -> Router<SharedState> {
    Router::new()
        .merge(openai_chat::routes())
        .merge(openai_embeddings::routes())
        .merge(anthropic_messages::routes())
        .merge(mcp::routes())
    // TODO: openai_completions, openai_models, native_passthrough.
}
