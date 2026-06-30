//! Client for cortiq-router. Issues `POST /v1/route` and (optionally) `POST /v1/feedback`.
//! Returns the decision as a [`RouteDecision`]. When the router is unavailable the caller
//! falls back to `routing.default` (graceful degradation).

use crate::config::RouterCfg;
use crate::model::RouteDecision;

pub struct RouterClient {
    http: reqwest::Client,
    url: String,
    api_key: Option<String>,
    taxonomy_id: Option<String>,
}

impl RouterClient {
    pub fn new(cfg: &RouterCfg) -> anyhow::Result<Self> {
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
            .and_then(|name| std::env::var(name).ok());
        Ok(Self {
            http,
            url: cfg.url.trim_end_matches('/').to_string(),
            api_key,
            taxonomy_id: cfg.taxonomy_id.clone(),
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
            Err(_) => return Ok(None), // degradation
        };
        if !resp.status().is_success() {
            return Ok(None);
        }
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
