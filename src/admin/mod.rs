//! Admin API for the web management panel (`/admin/api/*`) + SPA serving (`/admin/*`).
//!
//! All endpoints under `/admin/api` are protected by a Bearer admin token. Config changes
//! go through `AppState::reload` (validation → build `Live` → atomic TOML write
//! → swap), so they are applied without a restart. Secrets are never returned —
//! only their presence status.

pub mod assets;

use crate::config::{
    AdminCfg, ApiKeyCfg, BreakerCfg, CacheCfg, CmfCfg, Config, CortiqCfg, LogCfg, ModelCfg,
    ProtocolsCfg, RouteCfg, RouterCfg, RoutingCfg, RoutingPolicy, StatsCfg, TelemetryCfg,
    TierTargets,
};
use crate::model::{ChatRequest, GenParams, Message, RequestMeta, RoutingDirective};
use crate::state::SharedState;
use crate::stats::parse_duration_secs;
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

// ───────────────────────────── common types / helpers ───────────────────────

type ApiResult = Result<Json<Value>, ApiError>;

pub struct ApiError {
    status: StatusCode,
    message: String,
}
impl ApiError {
    fn bad(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::bad(e.to_string())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": {"message": self.message, "type": "admin_error"}})),
        )
            .into_response()
    }
}

fn ok(v: Value) -> ApiResult {
    Ok(Json(v))
}

fn current_cfg(state: &SharedState) -> Config {
    state.live().cfg.clone()
}

/// Random hex token from `n_bytes` bytes of cryptographic randomness.
/// Uses the OS CSPRNG via `getrandom` (works on Linux/macOS/Windows); falls back
/// to a time-based generator only in the extremely unlikely event that fails.
pub fn random_token(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    if getrandom::getrandom(&mut buf).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((t >> ((i % 16) * 8)) as u8) ^ (i as u8).wrapping_mul(31);
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n <= 8 {
        return "•".repeat(n.max(3));
    }
    let head: String = key.chars().take(6).collect();
    let tail: String = key.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

fn tiers_to_map(routing: &RoutingCfg) -> HashMap<String, Vec<String>> {
    routing
        .tiers
        .iter()
        .map(|(k, v)| {
            let list = match v {
                TierTargets::List(l) => l.clone(),
                TierTargets::One(s) => vec![s.clone()],
            };
            (k.clone(), list)
        })
        .collect()
}

fn parse_routing(model: &str) -> RoutingDirective {
    if let Some(rest) = model.strip_prefix("cortiq-auto") {
        let profile = rest.strip_prefix(':').map(|p| p.to_string());
        RoutingDirective::Auto { profile }
    } else {
        RoutingDirective::Pinned {
            model_id: model.to_string(),
        }
    }
}

// ───────────────────────────── router ───────────────────────────────────────

/// Admin API routes (token-protected). Mounted into the main router.
/// The token is captured in a middleware closure so it does not depend on State.
pub fn api_routes(admin_token: String) -> Router<SharedState> {
    Router::new()
        .route("/admin/api/health", get(health))
        .route("/admin/api/router/probe", post(router_probe))
        .route("/admin/api/meta", get(meta))
        .route("/admin/api/config", get(get_config).put(put_config))
        .route("/admin/api/models", get(list_models).post(create_model))
        .route(
            "/admin/api/models/:id",
            axum::routing::put(update_model).delete(delete_model),
        )
        .route("/admin/api/models/:id/probe", post(probe_model))
        .route("/admin/api/routing", get(get_routing).put(put_routing))
        .route(
            "/admin/api/protocols",
            get(get_protocols).put(put_protocols),
        )
        .route("/admin/api/settings", get(get_settings).put(put_settings))
        .route("/admin/api/keys", get(list_keys).post(create_key))
        .route("/admin/api/keys/:key", axum::routing::delete(delete_key))
        .route(
            "/admin/api/secrets",
            get(list_secrets).put(set_secret).delete(clear_secret),
        )
        .route("/admin/api/stats", get(get_stats))
        .route("/admin/api/shadow", get(get_shadow))
        .route("/admin/api/hf/search", get(hf_search))
        .route("/admin/api/import", get(list_imports).post(start_import))
        .route("/admin/api/import/:job", get(import_status).delete(delete_import))
        .route("/admin/api/import/:job/cancel", post(cancel_import))
        .route("/admin/api/import/:job/register", post(register_import))
        .route("/admin/api/requests", get(get_requests))
        .route("/admin/api/test", post(run_test))
        .route("/admin/api/test/stream", post(run_test_stream))
        .route_layer(middleware::from_fn(move |req: Request, next: Next| {
            let expected = admin_token.clone();
            async move { auth(expected, req, next).await }
        }))
}

/// Extract Bearer admin token from the `Authorization` header.
fn bearer_token(req: &Request) -> String {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
                .unwrap_or(v)
                .trim()
        })
        .unwrap_or("")
        .to_string()
}

/// Admin authentication middleware (constant-time comparison).
async fn auth(expected: String, req: Request, next: Next) -> Response {
    let token = bearer_token(&req);
    if !expected.is_empty() && constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "invalid or missing admin token", "type": "authentication_error", "code": "unauthorized"}})),
        )
            .into_response()
    }
}

// ───────────────────────────── metrics ──────────────────────────────────────

/// `GET /metrics` (Prometheus). Exposed when `telemetry.metrics = true`.
pub async fn metrics(State(state): State<SharedState>) -> Response {
    let live = state.live();
    if !live.cfg.telemetry.metrics {
        return (StatusCode::NOT_FOUND, "metrics disabled").into_response();
    }
    let body = state.stats.prometheus();
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

// ───────────────────────────── health / meta ───────────────────────────────

async fn probe_router(url: &str) -> (bool, u64) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .danger_accept_invalid_certs(true)
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, 0),
    };
    let started = Instant::now();
    // cortiq-router exposes health at /v1/healthz (auth-exempt); a bare /healthz
    // hits the auth middleware and 401s, which would read as a false "reachable".
    let target = format!("{}/v1/healthz", url.trim_end_matches('/'));
    let ok = client
        .get(&target)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    (ok, started.elapsed().as_millis() as u64)
}

async fn health(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    let (router_ok, router_ms) = probe_router(&live.cfg.router.url).await;
    let key_source = match &live.cfg.router.api_key_env {
        Some(env) => state.secrets.source(env),
        None => "none",
    };
    let last = state
        .router_status
        .load(std::sync::atomic::Ordering::Relaxed);
    let last_error = if last == 0 {
        Value::Null
    } else {
        json!({ "kind": crate::router_client::classify_status(last), "http": last })
    };
    let models: Vec<Value> = live
        .cfg
        .models
        .iter()
        .map(|m| {
            let key = match &m.api_key_env {
                Some(env) => state.secrets.source(env),
                None => "none",
            };
            json!({
                "id": m.id,
                "provider": m.provider,
                "model": m.model,
                "kind": m.kind,
                "cost_tier": m.cost_tier,
                "key_source": key,
                "in_registry": live.registry.get(&m.id).is_some(),
            })
        })
        .collect();
    ok(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "listen": live.cfg.listen,
        "router": {
            "url": live.cfg.router.url,
            "reachable": router_ok,
            "latency_ms": router_ms,
            "key_env": live.cfg.router.api_key_env,
            "key_source": key_source,
            "last_error": last_error,
        },
        "models": models,
    }))
}

/// `POST /admin/api/router/probe` — a real (authenticated) `/v1/route` call so the
/// panel can tell "key missing / rejected / out of quota" apart from "router down".
/// User-triggered only: it consumes one routing decision. Also fetches
/// `GET /v1/usage` best-effort — not part of the documented contract yet; the
/// panel shows balance/usage automatically once the router starts serving it.
async fn router_probe(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    let rcfg = live.cfg.router.clone();
    let key = rcfg
        .api_key_env
        .as_deref()
        .and_then(|n| state.secrets.resolve(n));
    if rcfg.api_key_env.is_some() && key.is_none() {
        return ok(json!({ "ok": false, "status": "no_key" }));
    }

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(rcfg.timeout_ms.max(5000)));
    if !rcfg.verify_tls {
        builder = builder
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
    }
    let client = builder.build().map_err(|e| ApiError::bad(e.to_string()))?;
    let url = rcfg.url.trim_end_matches('/').to_string();

    let mut body = json!({
        "input": { "text": "ping" },
        "options": { "policy_profile": "balanced", "allow_oracle": false }
    });
    if let Some(tax) = &rcfg.taxonomy_id {
        body["taxonomy_id"] = json!(tax);
    }
    let mut req = client.post(format!("{url}/v1/route")).json(&body);
    if let Some(k) = &key {
        req = req.bearer_auth(k);
    }

    let started = Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let status = if e.is_timeout() {
                "timeout"
            } else {
                "unreachable"
            };
            return ok(json!({
                "ok": false,
                "status": status,
                "latency_ms": started.elapsed().as_millis() as u64,
            }));
        }
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    let code = resp.status().as_u16();
    if !resp.status().is_success() {
        let message = resp
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()));
        return ok(json!({
            "ok": false,
            "status": crate::router_client::classify_status(code),
            "http": code,
            "latency_ms": latency_ms,
            "message": message,
        }));
    }

    let mut usage = Value::Null;
    if let Some(k) = &key {
        if let Ok(r) = client
            .get(format!("{url}/v1/usage"))
            .bearer_auth(k)
            .send()
            .await
        {
            if r.status().is_success() {
                usage = r.json::<Value>().await.unwrap_or(Value::Null);
            }
        }
    }
    ok(json!({ "ok": true, "status": "ok", "latency_ms": latency_ms, "usage": usage }))
}

async fn meta() -> ApiResult {
    ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "providers": ["openai", "anthropic", "ollama", "http"],
        "provider_impl": { "openai": true, "ollama": true, "http": true, "anthropic": true },
        "cost_tiers": ["cheap", "mid", "expensive"],
        "kinds": ["chat", "embedding"],
        "profiles": ["cost-saver", "balanced", "quality-first"],
        "text_strategies": ["last_user", "last_user_plus_system", "concat_all"],
        "policy_modes": ["fixed_table", "cost_aware"],
        "tiers": ["low", "medium", "high"],
        "caps": ["tools", "vision"],
        "protocols_impl": {
            "openai_chat": true, "openai_completions": true, "openai_embeddings": true,
            "openai_models": true, "anthropic_messages": true, "mcp": true, "native_passthrough": true
        },
        "languages": ["en", "ru", "de", "fr", "es", "zh", "tr"],
    }))
}

// ───────────────────────────── config ──────────────────────────────────────

async fn get_config(State(state): State<SharedState>) -> ApiResult {
    let cfg = current_cfg(&state);
    ok(serde_json::to_value(&cfg).map_err(|e| ApiError::bad(e.to_string()))?)
}

async fn put_config(State(state): State<SharedState>, Json(cfg): Json<Config>) -> ApiResult {
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

// ───────────────────────────── models ──────────────────────────────────────

async fn list_models(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    let models: Vec<Value> = live
        .cfg
        .models
        .iter()
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(json!({}));
            let key_source = match &m.api_key_env {
                Some(env) => state.secrets.source(env),
                None => "none",
            };
            v["key_source"] = json!(key_source);
            v
        })
        .collect();
    ok(json!({ "models": models }))
}

async fn create_model(State(state): State<SharedState>, Json(m): Json<ModelCfg>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    if cfg.models.iter().any(|x| x.id == m.id) {
        return Err(ApiError::bad(format!("model id '{}' already exists", m.id)));
    }
    cfg.models.push(m);
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

async fn update_model(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(m): Json<ModelCfg>,
) -> ApiResult {
    let mut cfg = current_cfg(&state);
    let idx = cfg
        .models
        .iter()
        .position(|x| x.id == id)
        .ok_or_else(|| ApiError::not_found(format!("model '{id}' not found")))?;
    // if the id was renamed — must not conflict with an existing one
    if m.id != id && cfg.models.iter().any(|x| x.id == m.id) {
        return Err(ApiError::bad(format!("model id '{}' already exists", m.id)));
    }
    cfg.models[idx] = m;
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

async fn delete_model(State(state): State<SharedState>, Path(id): Path<String>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    let before = cfg.models.len();
    cfg.models.retain(|x| x.id != id);
    if cfg.models.len() == before {
        return Err(ApiError::not_found(format!("model '{id}' not found")));
    }
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

async fn probe_model(State(state): State<SharedState>, Path(id): Path<String>) -> ApiResult {
    let live = state.live();
    let mcfg = live
        .cfg
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| ApiError::not_found(format!("model '{id}' not found")))?
        .clone();

    if mcfg.provider == "anthropic" {
        return ok(json!({
            "ok": false,
            "error": "anthropic provider not yet implemented (planned v0.2)",
        }));
    }
    let provider = live
        .registry
        .get(&id)
        .ok_or_else(|| ApiError::bad(format!("model '{id}' has no runtime provider")))?;

    let req = ChatRequest {
        routing: RoutingDirective::Pinned {
            model_id: id.clone(),
        },
        messages: vec![Message {
            role: "user".into(),
            content: "ping".into(),
            tool_calls: vec![],
        }],
        tools: vec![],
        params: GenParams {
            temperature: Some(0.0),
            max_tokens: Some(1),
            ..Default::default()
        },
        stream: false,
        meta: RequestMeta {
            protocol: "probe".into(),
            ..Default::default()
        },
    };
    let started = Instant::now();
    let res = provider.chat(req).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(_) => ok(json!({ "ok": true, "latency_ms": latency_ms })),
        Err(e) => ok(json!({ "ok": false, "latency_ms": latency_ms, "error": e.to_string() })),
    }
}

// ───────────────────────────── routing ─────────────────────────────────────

async fn get_routing(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    ok(json!({
        "tiers": tiers_to_map(&live.cfg.routing),
        "default": live.cfg.routing.default,
        "policy": live.cfg.routing.policy,
    }))
}

#[derive(Deserialize)]
struct RoutingBody {
    #[serde(default)]
    tiers: HashMap<String, Vec<String>>,
    default: String,
    #[serde(default)]
    policy: RoutingPolicy,
}

async fn put_routing(State(state): State<SharedState>, Json(body): Json<RoutingBody>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    cfg.routing = RoutingCfg {
        tiers: body
            .tiers
            .into_iter()
            .map(|(k, v)| (k, TierTargets::List(v)))
            .collect(),
        default: body.default,
        policy: body.policy,
    };
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

// ───────────────────────────── protocols ───────────────────────────────────

async fn get_protocols(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    ok(serde_json::to_value(&live.cfg.protocols).unwrap_or(json!({})))
}

async fn put_protocols(State(state): State<SharedState>, Json(p): Json<ProtocolsCfg>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    cfg.protocols = p;
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

// ───────────────────────────── settings ────────────────────────────────────

async fn get_settings(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    let c = &live.cfg;
    // admin section without token value (not stored here — only the env name)
    ok(json!({
        "listen": c.listen,
        "router": c.router,
        "route": c.route,
        "breaker": c.breaker,
        "log": c.log,
        "telemetry": c.telemetry,
        "cortiq": c.cortiq,
        "stats": c.stats,
        "admin": c.admin,
        "cache": c.cache,
        "cmf": c.cmf,
    }))
}

#[derive(Deserialize)]
struct SettingsBody {
    listen: Option<String>,
    router: Option<RouterCfg>,
    route: Option<RouteCfg>,
    breaker: Option<BreakerCfg>,
    log: Option<LogCfg>,
    telemetry: Option<TelemetryCfg>,
    cortiq: Option<CortiqCfg>,
    stats: Option<StatsCfg>,
    admin: Option<AdminCfg>,
    cache: Option<CacheCfg>,
    cmf: Option<CmfCfg>,
}

async fn put_settings(State(state): State<SharedState>, Json(b): Json<SettingsBody>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    let mut needs_restart = false;
    if let Some(v) = b.listen {
        if v != cfg.listen {
            needs_restart = true;
        }
        cfg.listen = v;
    }
    if let Some(v) = b.router {
        cfg.router = v;
    }
    if let Some(v) = b.route {
        cfg.route = v;
    }
    if let Some(v) = b.breaker {
        cfg.breaker = v;
    }
    if let Some(v) = b.log {
        cfg.log = v;
    }
    if let Some(v) = b.telemetry {
        cfg.telemetry = v;
    }
    if let Some(v) = b.cortiq {
        cfg.cortiq = v;
    }
    if let Some(v) = b.stats {
        cfg.stats = v;
    }
    if let Some(v) = b.admin {
        if v.listen != cfg.admin.listen {
            needs_restart = true;
        }
        cfg.admin = v;
    }
    if let Some(v) = b.cache {
        // the cache is built at startup; persist now, apply on restart
        needs_restart = true;
        cfg.cache = v;
    }
    if let Some(v) = b.cmf {
        // manage_server / local_model changes apply on restart (the server is
        // spawned at startup); local_only / router routing apply immediately.
        if v.manage_server != cfg.cmf.manage_server || v.local_model != cfg.cmf.local_model {
            needs_restart = true;
        }
        cfg.cmf = v;
    }
    state.reload(cfg)?;
    ok(json!({ "ok": true, "needs_restart": needs_restart }))
}

// ───────────────────────────── keys ────────────────────────────────────────

async fn list_keys(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    let keys: Vec<Value> = live
        .cfg
        .api_keys
        .iter()
        .map(|k| {
            json!({
                "key": k.key,
                "key_masked": mask_key(&k.key),
                "account": k.account,
                "rate_per_min": k.rate_per_min,
                "allow_models": k.allow_models,
            })
        })
        .collect();
    ok(json!({ "keys": keys, "open_mode": live.cfg.api_keys.is_empty() }))
}

#[derive(Deserialize)]
struct NewKey {
    #[serde(default)]
    key: String,
    account: String,
    #[serde(default)]
    rate_per_min: u32,
    #[serde(default)]
    allow_models: Vec<String>,
}

async fn create_key(State(state): State<SharedState>, Json(b): Json<NewKey>) -> ApiResult {
    let key = if b.key.trim().is_empty() {
        format!("sk-gw-{}", random_token(18))
    } else {
        b.key.trim().to_string()
    };
    let mut cfg = current_cfg(&state);
    if cfg.api_keys.iter().any(|k| k.key == key) {
        return Err(ApiError::bad("key already exists"));
    }
    cfg.api_keys.push(ApiKeyCfg {
        key: key.clone(),
        account: b.account,
        rate_per_min: b.rate_per_min,
        allow_models: b.allow_models,
    });
    state.reload(cfg)?;
    // show the full key only once
    ok(json!({ "ok": true, "key": key }))
}

async fn delete_key(State(state): State<SharedState>, Path(key): Path<String>) -> ApiResult {
    let mut cfg = current_cfg(&state);
    let before = cfg.api_keys.len();
    cfg.api_keys.retain(|k| k.key != key);
    if cfg.api_keys.len() == before {
        return Err(ApiError::not_found("key not found"));
    }
    state.reload(cfg)?;
    ok(json!({ "ok": true }))
}

// ───────────────────────────── secrets ─────────────────────────────────────

async fn list_secrets(State(state): State<SharedState>) -> ApiResult {
    let live = state.live();
    // collect all env names referenced in the config
    let mut names: Vec<String> = Vec::new();
    for m in &live.cfg.models {
        if let Some(env) = &m.api_key_env {
            if !names.contains(env) {
                names.push(env.clone());
            }
        }
    }
    if let Some(env) = &live.cfg.router.api_key_env {
        if !names.contains(env) {
            names.push(env.clone());
        }
    }
    let secrets: Vec<Value> = names
        .iter()
        .map(|n| json!({ "name": n, "source": state.secrets.source(n) }))
        .collect();
    ok(json!({ "secrets": secrets }))
}

#[derive(Deserialize)]
struct SecretBody {
    name: String,
    value: String,
}

async fn set_secret(State(state): State<SharedState>, Json(b): Json<SecretBody>) -> ApiResult {
    if b.name.trim().is_empty() {
        return Err(ApiError::bad("secret name is required"));
    }
    state.secrets.set(b.name.trim(), &b.value)?;
    // rebuild providers with the new key (without writing TOML)
    state.rebuild()?;
    ok(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

async fn clear_secret(State(state): State<SharedState>, Query(q): Query<NameQuery>) -> ApiResult {
    state.secrets.clear(q.name.trim())?;
    state.rebuild()?;
    ok(json!({ "ok": true }))
}

// ───────────────────────────── stats / requests ────────────────────────────

#[derive(Deserialize)]
struct StatsQuery {
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    groupby: Option<String>,
}

async fn get_stats(State(state): State<SharedState>, Query(q): Query<StatsQuery>) -> ApiResult {
    let range_secs = q
        .range
        .as_deref()
        .and_then(parse_duration_secs)
        .unwrap_or(24 * 3600);
    let groupby = q.groupby.as_deref().unwrap_or("model");
    let mut snap = state.stats.snapshot(range_secs, groupby);
    snap["cache"] = state.cache.snapshot();
    ok(snap)
}

/// Self-warming shadow loop: per-task-type promotion state, Wilson-LB of the
/// judged pass-rate, and whether the local model is serving it yet.
async fn get_shadow(State(state): State<SharedState>) -> ApiResult {
    let labels: Vec<Value> = state
        .promotion
        .snapshot()
        .into_iter()
        .map(|(label, st, n, pass_rate, lb)| {
            json!({
                "label": label,
                "state": format!("{st:?}"),
                "n": n,
                "pass_rate": pass_rate,
                "wilson_lb": lb,
                "serves_local": state.promotion.serves_local(&label),
            })
        })
        .collect();
    ok(json!({ "enabled": state.promotion.enabled(), "labels": labels }))
}

// ── CMF model factory: HF search → convert → register ──

#[derive(Deserialize)]
struct HfQuery {
    q: Option<String>,
    limit: Option<usize>,
}

/// Proxy HuggingFace model search (server-side: avoids CORS, adds token).
async fn hf_search(State(state): State<SharedState>, Query(q): Query<HfQuery>) -> ApiResult {
    let live = state.live();
    let token = if live.cfg.cmf.hf_token_env.is_empty() {
        None
    } else {
        state.secrets.resolve(&live.cfg.cmf.hf_token_env)
    };
    let query = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(24).min(50);
    let models = crate::import::hf_search(&query, limit, token.as_deref())
        .await
        .map_err(ApiError::bad)?;
    ok(json!({ "models": models }))
}

async fn list_imports(State(state): State<SharedState>) -> ApiResult {
    ok(json!({ "jobs": state.imports.list() }))
}

/// Start a conversion job (returns the job id; progress via `import/:job`).
async fn start_import(
    State(state): State<SharedState>,
    Json(p): Json<crate::import::ImportParams>,
) -> ApiResult {
    let live = state.live();
    let id = crate::import::start_import(state.imports.clone(), &live.cfg.cmf, p)
        .map_err(ApiError::bad)?;
    ok(json!({ "job": id }))
}

async fn import_status(
    State(state): State<SharedState>,
    Path(job): Path<String>,
) -> ApiResult {
    match state.imports.get(&job) {
        Some(j) => ok(serde_json::to_value(j).unwrap_or_else(|_| json!({}))),
        None => Err(ApiError::not_found(format!("job '{job}' not found"))),
    }
}

/// Cancel a running conversion: kills the converter and removes partial output.
async fn cancel_import(
    State(state): State<SharedState>,
    Path(job): Path<String>,
) -> ApiResult {
    if state.imports.get(&job).is_none() {
        return Err(ApiError::not_found(format!("job '{job}' not found")));
    }
    let cancelled = state.imports.cancel(&job);
    ok(json!({ "ok": cancelled }))
}

/// Delete a finished conversion and its converted `.cmf` file(s) from disk.
async fn delete_import(
    State(state): State<SharedState>,
    Path(job): Path<String>,
) -> ApiResult {
    if state.imports.get(&job).is_none() {
        return Err(ApiError::not_found(format!("job '{job}' not found")));
    }
    let removed = state.imports.delete(&job).map_err(ApiError::bad)?;
    ok(json!({ "ok": removed }))
}

/// Register a finished `.cmf` as a local model (OpenAI provider → cortiq-server).
async fn register_import(
    State(state): State<SharedState>,
    Path(job): Path<String>,
) -> ApiResult {
    let j = state
        .imports
        .get(&job)
        .ok_or_else(|| ApiError::not_found(format!("job '{job}' not found")))?;
    if j.state != "done" {
        return Err(ApiError::bad("conversion not finished"));
    }
    let base = std::path::Path::new(&j.output)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "cmf-local".into());
    let id = format!("cmf-{base}");
    let mut cfg = current_cfg(&state);
    if cfg.models.iter().any(|m| m.id == id) {
        return Err(ApiError::bad(format!("model '{id}' already registered")));
    }
    cfg.models.push(crate::config::ModelCfg {
        id: id.clone(),
        provider: "openai".into(),
        base_url: cfg.cmf.cortiq_server_url.clone(),
        model: base,
        cost_tier: "local".into(),
        price_in: 0.0,
        price_out: 0.0,
        kind: "chat".into(),
        api_key_env: None,
        caps: Vec::new(),
    });
    state.reload(cfg)?;
    ok(json!({ "ok": true, "model_id": id }))
}

#[derive(Deserialize)]
struct RecentQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

async fn get_requests(State(state): State<SharedState>, Query(q): Query<RecentQuery>) -> ApiResult {
    let limit = q.limit.unwrap_or(50).min(500);
    let offset = q.offset.unwrap_or(0);
    let items = state.stats.recent(limit, offset);
    ok(json!({ "requests": items }))
}

// ───────────────────────────── playground (test) ───────────────────────────

#[derive(Deserialize)]
struct TestBody {
    #[serde(default = "default_test_model")]
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
}
fn default_test_model() -> String {
    "cortiq-auto".into()
}

async fn run_test(State(state): State<SharedState>, Json(b): Json<TestBody>) -> ApiResult {
    if b.messages.is_empty() {
        return Err(ApiError::bad("messages must not be empty"));
    }
    let req = ChatRequest {
        routing: parse_routing(&b.model),
        messages: b.messages,
        tools: vec![],
        params: GenParams {
            temperature: b.temperature,
            max_tokens: b.max_tokens,
            ..Default::default()
        },
        stream: false,
        meta: RequestMeta {
            account: "playground".into(),
            protocol: "playground".into(),
            ..Default::default()
        },
    };
    let started = Instant::now();
    let res = state.pipeline.run(req, &state).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(resp) => {
            let answer = resp
                .choices
                .first()
                .map(|c| c.message.content.clone())
                .unwrap_or_default();
            ok(json!({
                "ok": true,
                "latency_ms": latency_ms,
                "answer": answer,
                "model_used": resp.model_used,
                "usage": {
                    "prompt_tokens": resp.usage.prompt_tokens,
                    "completion_tokens": resp.usage.completion_tokens,
                    "total_tokens": resp.usage.total_tokens,
                },
                "cortiq": {
                    "task_label": resp.cortiq.task_label,
                    "complexity": { "score": resp.cortiq.complexity_score, "tier": resp.cortiq.complexity_tier },
                    "selected_model": resp.cortiq.selected_model,
                    "route_source": resp.cortiq.route_source,
                    "router_request_id": resp.cortiq.router_request_id,
                    "cost_usd": resp.cortiq.cost_usd,
                    "failover": resp.cortiq.failover,
                },
            }))
        }
        Err(e) => ok(json!({ "ok": false, "latency_ms": latency_ms, "error": e.to_string() })),
    }
}

/// Streaming playground: forwards provider SSE chunks; routing decision is exposed
/// via X-Cortiq-* response headers (the SPA reads them after the stream ends).
async fn run_test_stream(State(state): State<SharedState>, Json(b): Json<TestBody>) -> Response {
    if b.messages.is_empty() {
        return ApiError::bad("messages must not be empty").into_response();
    }
    let req = ChatRequest {
        routing: parse_routing(&b.model),
        messages: b.messages,
        tools: vec![],
        params: GenParams {
            temperature: b.temperature,
            max_tokens: b.max_tokens,
            ..Default::default()
        },
        stream: true,
        meta: RequestMeta {
            account: "playground".into(),
            protocol: "playground".into(),
            ..Default::default()
        },
    };
    match state.pipeline.run_stream(req, &state).await {
        Ok((info, stream)) => {
            let mut headers = crate::protocols::openai_chat::cortiq_headers(&info);
            headers.insert(
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("text/event-stream"),
            );
            (headers, axum::body::Body::from_stream(stream)).into_response()
        }
        Err(e) => ApiError::bad(e.to_string()).into_response(),
    }
}
