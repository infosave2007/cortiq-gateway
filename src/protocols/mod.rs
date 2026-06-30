//! Incoming protocol adapters. Each one builds its own axum routes and translates
//! its protocol to/from the canonical model. Enabled via flags in `[protocols]`.

pub mod openai_chat;

use crate::state::SharedState;
use axum::Router;

/// Build the protocols router. Implemented adapters are **always** mounted;
/// each handler checks the live `protocols.*` flag from config itself, so
/// toggles in the admin panel take effect without a restart (hot switching).
pub fn build_router() -> Router<SharedState> {
    Router::new().merge(openai_chat::routes())
    // TODO: openai_completions, openai_embeddings, openai_models,
    //       anthropic_messages, mcp, native_passthrough — add here as well,
    //       with a check of the corresponding flag inside the handler.
}
