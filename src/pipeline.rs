//! Canonical request processing pipeline (Pipeline).
//! Coordinates stages: Extract -> Route -> Select -> Translate & Call,
//! and also measures latency and records per-request statistics.

use crate::error::{GatewayError, Result};
use crate::model::{
    ChatRequest, ChatResponse, Choice, Message, RouteDecision, RouteInfo, RoutingDirective, Usage,
};
use crate::state::{Live, SharedState};
use crate::stats::RequestRecord;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Pipeline;

impl Pipeline {
    pub fn new() -> Self {
        Self
    }

    /// Run the routing and generation pipeline.
    pub async fn run(&self, req: ChatRequest, state: &SharedState) -> Result<ChatResponse> {
        let started = Instant::now();
        let account = req.meta.account.clone();
        let protocol = req.meta.protocol.clone();
        let directive = match &req.routing {
            RoutingDirective::Pinned { .. } => "pinned",
            RoutingDirective::Auto { .. } => "auto",
        }
        .to_string();

        // --- semantic cache lookup (short-circuit before the model call) ---
        let cache = &state.cache;
        let mut cache_key: Option<(String, Vec<f32>)> = None;
        if cache.enabled() {
            let live = state.live();
            let sig = cache_sig(&req);
            let text = self.extract_text(&req.messages, "concat_all", live.cfg.route.max_chars);
            let embed_model = cache
                .embed_model()
                .map(|s| s.to_string())
                .or_else(|| first_embedding_model(&live));
            if let Some(model_id) = embed_model {
                if let Some(vec) = embed(&live, &model_id, &text).await {
                    if let Some(hit) = cache.lookup(&sig, &vec) {
                        state.stats.record(RequestRecord {
                            ts: now_secs(),
                            account,
                            protocol,
                            directive,
                            task_label: "cache".into(),
                            tier: "cache".into(),
                            score: 0.0,
                            model_id: hit.model_used.clone(),
                            route_source: "cache".into(),
                            prompt_tokens: hit.prompt_tokens,
                            completion_tokens: hit.completion_tokens,
                            cost_usd: 0.0,
                            latency_ms: started.elapsed().as_millis() as u64,
                            outcome: "ok".into(),
                            failover: false,
                            error: None,
                        });
                        return Ok(cached_response(&hit));
                    }
                    cache_key = Some((sig, vec));
                }
            }
        }

        let result = self.run_inner(req, state).await;

        // record statistics (for both success and error)
        let latency_ms = started.elapsed().as_millis() as u64;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let rec = match &result {
            Ok(resp) => RequestRecord {
                ts,
                account,
                protocol,
                directive,
                task_label: resp.cortiq.task_label.clone(),
                tier: resp.cortiq.complexity_tier.clone(),
                score: resp.cortiq.complexity_score,
                model_id: resp.cortiq.selected_model.clone(),
                route_source: resp.cortiq.route_source.clone(),
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                cost_usd: resp.cortiq.cost_usd,
                latency_ms,
                outcome: "ok".to_string(),
                failover: resp.cortiq.failover,
                error: None,
            },
            Err(err) => RequestRecord {
                ts,
                account,
                protocol,
                directive,
                task_label: String::new(),
                tier: String::new(),
                score: 0.0,
                model_id: String::new(),
                route_source: String::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: 0.0,
                latency_ms,
                outcome: "error".to_string(),
                failover: false,
                error: Some(err.to_string()),
            },
        };
        state.stats.record(rec);

        // --- store a successful response in the semantic cache ---
        if let (Ok(resp), Some((sig, vec))) = (&result, cache_key) {
            let content = resp
                .choices
                .first()
                .map(|c| c.message.content.clone())
                .unwrap_or_default();
            if !content.is_empty() {
                state.cache.insert(
                    sig,
                    vec,
                    resp.cortiq.selected_model.clone(),
                    content,
                    resp.usage.prompt_tokens,
                    resp.usage.completion_tokens,
                    resp.cortiq.cost_usd,
                );
            }
        }

        result
    }

    /// Stages 1–3: extract text → route → select & capability-filter candidates.
    /// Shared by both the non-streaming and streaming paths.
    async fn resolve(
        &self,
        req: &ChatRequest,
        live: &crate::state::Live,
    ) -> Result<(RouteDecision, String, Vec<String>)> {
        // 1. Extract text for classification
        let text = self.extract_text(
            &req.messages,
            &live.cfg.route.text_strategy,
            live.cfg.route.max_chars,
        );

        // 2. Obtain the router decision (or use the pinned model)
        let (decision, route_source) = match &req.routing {
            RoutingDirective::Pinned { model_id: _ } => {
                let dec = RouteDecision {
                    task_label: "pinned".to_string(),
                    complexity_score: 0.0,
                    complexity_tier: "pinned".to_string(),
                    router_request_id: None,
                    source: "pinned".to_string(),
                };
                (dec, "pinned".to_string())
            }
            RoutingDirective::Auto { profile } => {
                let prof = profile.as_deref().unwrap_or(&live.cfg.route.profile);
                match live.router.route(&text, prof).await {
                    Ok(Some(dec)) => {
                        let src = dec.source.clone();
                        (dec, src)
                    }
                    Ok(None) | Err(_) => {
                        tracing::warn!(
                            "Router is unavailable. Gracefully degrading to default model."
                        );
                        let dec = RouteDecision {
                            task_label: "degraded".to_string(),
                            complexity_score: 0.5,
                            complexity_tier: "degraded".to_string(),
                            router_request_id: None,
                            source: "fallback".to_string(),
                        };
                        (dec, "fallback".to_string())
                    }
                }
            }
        };

        // 3. Select candidates by complexity tier / pinned model
        let candidates = match &req.routing {
            RoutingDirective::Pinned { model_id } => vec![model_id.clone()],
            RoutingDirective::Auto { .. } => {
                if route_source == "fallback" {
                    vec![live.routing.default_model().to_string()]
                } else {
                    live.routing.candidates(&decision.complexity_tier)
                }
            }
        };

        // Filter candidates by capabilities (e.g. tools support)
        let needs_tools = !req.tools.is_empty();
        let mut final_candidates = Vec::new();
        for model_id in candidates {
            if let Some(prov) = live.registry.get(&model_id) {
                if needs_tools && !prov.caps().tools {
                    tracing::debug!(
                        "Candidate '{}' skipped: lacks support for requested tools",
                        model_id
                    );
                    continue;
                }
                final_candidates.push(model_id);
            }
        }

        if final_candidates.is_empty() {
            return Err(GatewayError::UpstreamUnavailable(
                "No healthy model available that satisfies required capabilities (e.g. tools)"
                    .to_string(),
            ));
        }
        Ok((decision, route_source, final_candidates))
    }

    async fn run_inner(&self, req: ChatRequest, state: &SharedState) -> Result<ChatResponse> {
        let live = state.live();
        let (decision, route_source, final_candidates) = self.resolve(&req, &live).await?;

        // 4. Attempt provider call with failover (fall back to alternatives on error)
        let mut last_err = None;
        let mut attempt = 0usize;
        for model_id in final_candidates {
            let provider = match live.registry.get(&model_id) {
                Some(p) => p,
                None => continue,
            };

            tracing::info!(
                "Routing request (label: {}, tier: {}) to model '{}' via source '{}'",
                decision.task_label,
                decision.complexity_tier,
                model_id,
                route_source
            );

            match provider.chat(req.clone()).await {
                Ok(mut resp) => {
                    let cost =
                        provider.price(resp.usage.prompt_tokens, resp.usage.completion_tokens);
                    resp.cortiq = RouteInfo {
                        task_label: decision.task_label.clone(),
                        complexity_score: decision.complexity_score,
                        complexity_tier: decision.complexity_tier.clone(),
                        selected_model: model_id,
                        route_source: route_source.clone(),
                        router_request_id: decision.router_request_id.clone(),
                        cost_usd: cost,
                        failover: attempt > 0,
                    };
                    return Ok(resp);
                }
                Err(err) => {
                    tracing::warn!(
                        "Provider '{}' call failed: {:?}. Attempting failover...",
                        model_id,
                        err
                    );
                    last_err = Some(err);
                    attempt += 1;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            GatewayError::UpstreamUnavailable("All candidate models failed".to_string())
        }))
    }

    /// Streaming variant: returns the routing metadata plus a stream of SSE byte
    /// chunks (OpenAI wire format). Failover applies only before the first byte —
    /// once the upstream stream has started it cannot be retried. Statistics are
    /// recorded when the stream completes (tokens taken from the final usage chunk).
    pub async fn run_stream(
        &self,
        req: ChatRequest,
        state: &SharedState,
    ) -> Result<(RouteInfo, crate::providers::ChatStream)> {
        let started = Instant::now();
        let account = req.meta.account.clone();
        let protocol = req.meta.protocol.clone();
        let directive = match &req.routing {
            RoutingDirective::Pinned { .. } => "pinned",
            RoutingDirective::Auto { .. } => "auto",
        }
        .to_string();

        let live = state.live();
        let (decision, route_source, final_candidates) = match self.resolve(&req, &live).await {
            Ok(t) => t,
            Err(e) => {
                record_error(state, &account, &protocol, &directive, started, &e);
                return Err(e);
            }
        };

        let mut last_err = None;
        let mut attempt = 0usize;
        for model_id in final_candidates {
            let provider = match live.registry.get(&model_id) {
                Some(p) => p,
                None => continue,
            };
            tracing::info!(
                "Streaming request (label: {}, tier: {}) to model '{}' via source '{}'",
                decision.task_label,
                decision.complexity_tier,
                model_id,
                route_source
            );
            match provider.chat_stream(req.clone()).await {
                Ok(stream) => {
                    let info = RouteInfo {
                        task_label: decision.task_label.clone(),
                        complexity_score: decision.complexity_score,
                        complexity_tier: decision.complexity_tier.clone(),
                        selected_model: model_id,
                        route_source: route_source.clone(),
                        router_request_id: decision.router_request_id.clone(),
                        cost_usd: 0.0,
                        failover: attempt > 0,
                    };
                    let tapped = tap_stream(
                        stream,
                        state.stats.clone(),
                        provider.clone(),
                        info.clone(),
                        account,
                        protocol,
                        directive,
                        started,
                    );
                    return Ok((info, Box::pin(tapped)));
                }
                Err(err) => {
                    tracing::warn!(
                        "Provider '{}' stream failed: {:?}. Attempting failover...",
                        model_id,
                        err
                    );
                    last_err = Some(err);
                    attempt += 1;
                }
            }
        }
        let e = last_err.unwrap_or_else(|| {
            GatewayError::UpstreamUnavailable("All candidate models failed".to_string())
        });
        record_error(state, &account, &protocol, &directive, started, &e);
        Err(e)
    }

    fn extract_text(&self, messages: &[Message], strategy: &str, max_chars: usize) -> String {
        let raw_text = match strategy {
            "last_user" => messages
                .iter()
                .rfind(|m| m.role == "user")
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            "last_user_plus_system" => {
                let system_msgs: Vec<String> = messages
                    .iter()
                    .filter(|m| m.role == "system")
                    .map(|m| m.content.clone())
                    .collect();
                let last_user = messages
                    .iter()
                    .rfind(|m| m.role == "user")
                    .map(|m| m.content.as_str())
                    .unwrap_or("");
                if system_msgs.is_empty() {
                    last_user.to_string()
                } else {
                    format!("{}\n{}", system_msgs.join("\n"), last_user)
                }
            }
            // "concat_all" and any unknown strategy fall back to concatenating all turns
            _ => messages
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<String>>()
                .join("\n"),
        };

        if raw_text.chars().count() > max_chars {
            raw_text.chars().take(max_chars).collect()
        } else {
            raw_text
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Routing signature used to partition cache entries (so a pinned model's cached
/// answer is never returned for a different model/profile).
fn cache_sig(req: &ChatRequest) -> String {
    match &req.routing {
        RoutingDirective::Auto { profile } => {
            format!("auto:{}", profile.as_deref().unwrap_or(""))
        }
        RoutingDirective::Pinned { model_id } => format!("pinned:{model_id}"),
    }
}

fn first_embedding_model(live: &Live) -> Option<String> {
    live.cfg
        .models
        .iter()
        .find(|m| m.kind == "embedding")
        .map(|m| m.id.clone())
}

/// Embed `text` with the given model; returns the embedding vector (or None on error).
async fn embed(live: &Live, model_id: &str, text: &str) -> Option<Vec<f32>> {
    let provider = live.registry.get(model_id)?;
    let body = provider.embed(serde_json::json!(text)).await.ok()?;
    let arr = body["data"][0]["embedding"].as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_f64())
            .map(|x| x as f32)
            .collect(),
    )
}

fn cached_response(hit: &crate::cache::CacheHit) -> ChatResponse {
    ChatResponse {
        id: "cache".to_string(),
        model_used: hit.model_used.clone(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".into(),
                content: hit.content.clone(),
                tool_calls: vec![],
            },
            finish_reason: "stop".into(),
        }],
        usage: Usage {
            prompt_tokens: hit.prompt_tokens,
            completion_tokens: hit.completion_tokens,
            total_tokens: hit.prompt_tokens + hit.completion_tokens,
        },
        cortiq: RouteInfo {
            task_label: "cache".into(),
            complexity_score: 0.0,
            complexity_tier: "cache".into(),
            selected_model: hit.model_used.clone(),
            route_source: "cache".into(),
            router_request_id: None,
            cost_usd: 0.0,
            failover: false,
        },
    }
}

fn record_error(
    state: &SharedState,
    account: &str,
    protocol: &str,
    directive: &str,
    started: Instant,
    e: &GatewayError,
) {
    state.stats.record(RequestRecord {
        ts: now_secs(),
        account: account.to_string(),
        protocol: protocol.to_string(),
        directive: directive.to_string(),
        task_label: String::new(),
        tier: String::new(),
        score: 0.0,
        model_id: String::new(),
        route_source: String::new(),
        prompt_tokens: 0,
        completion_tokens: 0,
        cost_usd: 0.0,
        latency_ms: started.elapsed().as_millis() as u64,
        outcome: "error".to_string(),
        failover: false,
        error: Some(e.to_string()),
    });
}

/// Forward a provider SSE stream to the client verbatim while tapping the final
/// `usage` chunk; record statistics when the stream completes.
#[allow(clippy::too_many_arguments)]
fn tap_stream(
    inner: crate::providers::ChatStream,
    stats: std::sync::Arc<crate::stats::Stats>,
    provider: std::sync::Arc<dyn crate::providers::Provider>,
    info: RouteInfo,
    account: String,
    protocol: String,
    directive: String,
    started: Instant,
) -> impl futures::Stream<Item = Result<bytes::Bytes>> {
    use futures::StreamExt;
    async_stream::stream! {
        let mut inner = inner;
        let mut buf = String::new();
        let mut prompt_tokens = 0u32;
        let mut completion_tokens = 0u32;
        let mut got_usage = false;
        let mut delta_chars = 0usize;
        let mut errored: Option<String> = None;

        while let Some(item) = inner.next().await {
            match &item {
                Ok(bytes) => {
                    if let Ok(s) = std::str::from_utf8(bytes) {
                        buf.push_str(s);
                        while let Some(idx) = buf.find("\n\n") {
                            let evt: String = buf.drain(..idx + 2).collect();
                            for line in evt.lines() {
                                if let Some(data) = line.trim_start().strip_prefix("data:") {
                                    let data = data.trim();
                                    if data.is_empty() || data == "[DONE]" {
                                        continue;
                                    }
                                    if let Ok(v) =
                                        serde_json::from_str::<serde_json::Value>(data)
                                    {
                                        let u = &v["usage"];
                                        if !u.is_null() {
                                            prompt_tokens = u["prompt_tokens"]
                                                .as_u64()
                                                .unwrap_or(prompt_tokens as u64)
                                                as u32;
                                            completion_tokens = u["completion_tokens"]
                                                .as_u64()
                                                .unwrap_or(completion_tokens as u64)
                                                as u32;
                                            got_usage = true;
                                        }
                                        if let Some(c) =
                                            v["choices"][0]["delta"]["content"].as_str()
                                        {
                                            delta_chars += c.chars().count();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => errored = Some(e.to_string()),
            }
            yield item;
        }

        if !got_usage && completion_tokens == 0 {
            // estimate when the server didn't return usage (~4 chars per token)
            completion_tokens = (delta_chars / 4) as u32;
        }
        let cost = provider.price(prompt_tokens, completion_tokens);
        stats.record(RequestRecord {
            ts: now_secs(),
            account,
            protocol,
            directive,
            task_label: info.task_label.clone(),
            tier: info.complexity_tier.clone(),
            score: info.complexity_score,
            model_id: info.selected_model.clone(),
            route_source: info.route_source.clone(),
            prompt_tokens,
            completion_tokens,
            cost_usd: cost,
            latency_ms: started.elapsed().as_millis() as u64,
            outcome: if errored.is_some() { "error" } else { "ok" }.to_string(),
            failover: info.failover,
            error: errored,
        });
    }
}
