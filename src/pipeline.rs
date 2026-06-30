//! Canonical request processing pipeline (Pipeline).
//! Coordinates stages: Extract -> Route -> Select -> Translate & Call,
//! and also measures latency and records per-request statistics.

use crate::error::{GatewayError, Result};
use crate::model::{
    ChatRequest, ChatResponse, Message, RouteDecision, RouteInfo, RoutingDirective,
};
use crate::state::SharedState;
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

        result
    }

    async fn run_inner(&self, req: ChatRequest, state: &SharedState) -> Result<ChatResponse> {
        let live = state.live();

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
