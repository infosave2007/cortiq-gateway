//! Client for cortiq-router. Issues `POST /v1/route` and (optionally) `POST /v1/feedback`.
//! Returns the decision as a [`RouteDecision`]. When the router is unavailable the caller
//! falls back to `routing.default` (graceful degradation).

use crate::config::RouterCfg;
use crate::model::RouteDecision;
use crate::secrets::SecretStore;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

/// Outcome of the most recent real `/v1/route` call, shared with the admin health
/// endpoint so the panel can tell "no money / bad key" apart from "router down".
/// 0 = ok (or no calls yet); [`STATUS_NETWORK`] / [`STATUS_TIMEOUT`] for transport
/// failures; otherwise the upstream HTTP status code.
pub type RouterLastStatus = Arc<AtomicU16>;

pub const STATUS_NETWORK: u16 = 1;
pub const STATUS_TIMEOUT: u16 = 2;

/// Error kind for the admin panel.
pub fn classify_status(code: u16) -> &'static str {
    match code {
        STATUS_NETWORK => "unreachable",
        STATUS_TIMEOUT => "timeout",
        401 | 403 => "auth",
        402 => "payment",
        429 => "quota",
        _ => "error",
    }
}

pub struct RouterClient {
    http: reqwest::Client,
    url: String,
    api_key: Option<String>,
    taxonomy_id: Option<String>,
    last_status: RouterLastStatus,
}

impl RouterClient {
    /// The key is resolved through the secret store (admin panel value first,
    /// then the environment variable) so a key pasted in the console works
    /// without a restart.
    pub fn new(
        cfg: &RouterCfg,
        secrets: &SecretStore,
        last_status: RouterLastStatus,
    ) -> anyhow::Result<Self> {
        let mut builder =
            reqwest::Client::builder().timeout(std::time::Duration::from_millis(cfg.timeout_ms));

        if !cfg.verify_tls {
            builder = builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }

        let http = builder.build()?;
        let api_key = cfg
            .api_key_env
            .as_ref()
            .and_then(|name| secrets.resolve(name));
        Ok(Self {
            http,
            url: cfg.url.trim_end_matches('/').to_string(),
            api_key,
            taxonomy_id: cfg.taxonomy_id.clone(),
            last_status,
        })
    }

    /// Classify text. Returns `Ok(None)` on network error/timeout —
    /// the caller treats this as "router unavailable" and uses the default tier.
    pub async fn route(&self, text: &str, profile: &str) -> anyhow::Result<Option<RouteDecision>> {
        let mut body = serde_json::json!({
            "input": { "text": text },
            "options": { "policy_profile": profile, "allow_oracle": true }
        });
        if let Some(tax) = &self.taxonomy_id {
            body["taxonomy_id"] = serde_json::Value::String(tax.clone());
        }

        let mut req = self.http.post(format!("{}/v1/route", self.url)).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let code = if e.is_timeout() {
                    STATUS_TIMEOUT
                } else {
                    STATUS_NETWORK
                };
                self.last_status.store(code, Ordering::Relaxed);
                return Ok(None); // degradation
            }
        };
        if !resp.status().is_success() {
            self.last_status
                .store(resp.status().as_u16(), Ordering::Relaxed);
            return Ok(None);
        }
        self.last_status.store(0, Ordering::Relaxed);
        let v: serde_json::Value = resp.json().await?;
        let d = &v["decision"];
        Ok(Some(RouteDecision {
            task_label: d["task_label"]
                .as_str()
                .unwrap_or("__unknown__")
                .to_string(),
            complexity_score: d["complexity"]["score"].as_f64().unwrap_or(0.5) as f32,
            complexity_tier: d["complexity"]["tier"]
                .as_str()
                .unwrap_or("medium")
                .to_string(),
            router_request_id: v["request_id"].as_str().map(|s| s.to_string()),
            source: "router".into(),
        }))
    }
}
