//! Cortiq Gateway — entry point.
//!
//! Loads the config, builds the shared state, assembles routes from protocols and
//! the admin panel, and starts the HTTP server. See docs/ARCHITECTURE.md.

// Part of the canonical model / error API surface (auth, rate-limit, idempotency,
// traceparent, vision/streaming caps) is intentionally reserved for upcoming phases;
// allow the unused items until those features land.
#![allow(dead_code)]

mod admin;
mod cache;
mod cmf_runtime;
mod config;
mod error;
mod import;
mod model;
mod pipeline;
mod promotion;
mod protocols;
mod providers;
mod registry;
mod router_client;
mod routing;
mod secrets;
mod state;
mod stats;

use axum::{routing::get, Router};
use clap::Parser;
use config::Config;
use state::SharedState;

#[derive(Parser)]
#[command(
    name = "cortiq-gateway",
    version,
    about = "Universal LLM gateway with intelligent routing"
)]
struct Args {
    /// Path to the TOML config file.
    #[arg(long, default_value = "config/gateway.toml")]
    config: String,
    /// Admin token for the web panel (overrides `[admin].token_env`).
    #[arg(long)]
    admin_token: Option<String>,
}

/// Resolve the admin token: env from `[admin].token_env` → `--admin-token` →
/// generate a random one. Returns `(token, generated)`.
fn resolve_admin_token(cfg: &Config, args: &Args) -> (String, bool) {
    if let Some(env) = &cfg.admin.token_env {
        if let Ok(v) = std::env::var(env) {
            if !v.is_empty() {
                return (v, false);
            }
        }
    }
    if let Some(t) = &args.admin_token {
        if !t.is_empty() {
            return (t.clone(), false);
        }
    }
    (admin::random_token(24), true)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let cfg = Config::load(&args.config)?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log.level.clone().into()),
        )
        .init();

    let listen = cfg.listen.clone();
    let admin_enabled = cfg.admin.enabled;
    let (admin_token, generated) = resolve_admin_token(&cfg, &args);

    let state = SharedState::build(cfg, args.config.clone())?;

    // Close the loop with the CMF format (github.com/infosave2007/cmf): install
    // / update cortiq-cli and run a local `cortiq serve` if configured. Runs in
    // the background so startup is never blocked by a cargo install or a model
    // load; the served model is registered in the pool and usable offline.
    {
        let cmf_rt = state.cmf.clone();
        let cmf_cfg = state.live().cfg.cmf.clone();
        tokio::spawn(async move { cmf_runtime::manage(cmf_rt, cmf_cfg).await });
    }

    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .route("/metrics", get(admin::metrics))
        .merge(protocols::build_router());

    if admin_enabled {
        app = app
            .merge(admin::api_routes(admin_token.clone()))
            .fallback(admin::assets::fallback);
    }

    let app = app.with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("cortiq-gateway listening on {listen}");
    if admin_enabled {
        tracing::info!("admin console: http://{listen}/admin");
        if generated {
            tracing::warn!(
                "admin token (auto-generated, set [admin].token_env to persist): {admin_token}"
            );
        }
    }
    axum::serve(listener, app).await?;
    Ok(())
}
