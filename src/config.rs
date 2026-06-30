//! Configuration loading from TOML. Structs mirror config/gateway.example.toml.
//! Secrets are read from environment variables by the names given in `*_env` fields.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub listen: String,
    pub router: RouterCfg,
    #[serde(default)]
    pub route: RouteCfg,
    #[serde(default)]
    pub models: Vec<ModelCfg>,
    pub routing: RoutingCfg,
    #[serde(default)]
    pub breaker: BreakerCfg,
    #[serde(default)]
    pub protocols: ProtocolsCfg,
    #[serde(default)]
    pub cortiq: CortiqCfg,
    #[serde(default)]
    pub api_keys: Vec<ApiKeyCfg>,
    #[serde(default)]
    pub idempotency: IdempotencyCfg,
    #[serde(default)]
    pub log: LogCfg,
    #[serde(default)]
    pub telemetry: TelemetryCfg,
    #[serde(default)]
    pub admin: AdminCfg,
    #[serde(default)]
    pub stats: StatsCfg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterCfg {
    pub url: String,
    #[serde(default = "default_router_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_verify_tls")]
    pub verify_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taxonomy_id: Option<String>,
}
fn default_router_timeout() -> u64 {
    800
}
fn default_verify_tls() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteCfg {
    #[serde(default = "default_text_strategy")]
    pub text_strategy: String,
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl: String,
    #[serde(default = "default_profile")]
    pub profile: String,
}
impl Default for RouteCfg {
    fn default() -> Self {
        Self {
            text_strategy: default_text_strategy(),
            max_chars: default_max_chars(),
            cache_ttl: default_cache_ttl(),
            profile: default_profile(),
        }
    }
}
fn default_text_strategy() -> String {
    "last_user".into()
}
fn default_max_chars() -> usize {
    4000
}
fn default_cache_ttl() -> String {
    "60s".into()
}
fn default_profile() -> String {
    "balanced".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCfg {
    pub id: String,
    pub provider: String, // openai | anthropic | ollama | http
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub cost_tier: String,
    #[serde(default)]
    pub price_in: f64,
    #[serde(default)]
    pub price_out: f64,
    #[serde(default = "default_kind")]
    pub kind: String, // chat | embedding
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caps: Vec<String>,
}
fn default_kind() -> String {
    "chat".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingCfg {
    #[serde(flatten)]
    pub tiers: HashMap<String, TierTargets>,
    pub default: String,
    #[serde(default)]
    pub policy: RoutingPolicy,
}

/// A routing tier is either a list of model_ids or the string `default`.
/// (`default` is handled by a separate field; only tier lists appear here.)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TierTargets {
    List(Vec<String>),
    One(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingPolicy {
    #[serde(default = "default_mode")]
    pub mode: String, // fixed_table | cost_aware
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd_per_request: Option<f64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub min_class: HashMap<String, String>,
}
impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            min_class: HashMap::new(),
            max_cost_usd_per_request: None,
        }
    }
}
fn default_mode() -> String {
    "fixed_table".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BreakerCfg {
    #[serde(default = "default_breaker_threshold")]
    pub threshold: u32,
    #[serde(default = "default_breaker_cooldown")]
    pub cooldown: String,
}
impl Default for BreakerCfg {
    fn default() -> Self {
        Self {
            threshold: default_breaker_threshold(),
            cooldown: default_breaker_cooldown(),
        }
    }
}
fn default_breaker_threshold() -> u32 {
    5
}
fn default_breaker_cooldown() -> String {
    "30s".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProtocolsCfg {
    #[serde(default)]
    pub openai_chat: bool,
    #[serde(default)]
    pub openai_completions: bool,
    #[serde(default)]
    pub openai_embeddings: bool,
    #[serde(default)]
    pub openai_models: bool,
    #[serde(default)]
    pub anthropic_messages: bool,
    #[serde(default)]
    pub mcp: bool,
    #[serde(default)]
    pub native_passthrough: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CortiqCfg {
    #[serde(default)]
    pub echo: bool,
}
impl Default for CortiqCfg {
    fn default() -> Self {
        Self { echo: true }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKeyCfg {
    pub key: String,
    pub account: String,
    #[serde(default)]
    pub rate_per_min: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_models: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdempotencyCfg {
    #[serde(default = "default_idem_ttl")]
    pub ttl: String,
}
impl Default for IdempotencyCfg {
    fn default() -> Self {
        Self {
            ttl: default_idem_ttl(),
        }
    }
}
fn default_idem_ttl() -> String {
    "10m".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogCfg {
    #[serde(default)]
    pub bodies: bool,
    #[serde(default = "default_log_level")]
    pub level: String,
}
impl Default for LogCfg {
    fn default() -> Self {
        Self {
            bodies: false,
            level: default_log_level(),
        }
    }
}
fn default_log_level() -> String {
    "info".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TelemetryCfg {
    #[serde(default)]
    pub metrics: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp_endpoint_env: Option<String>,
}

/// Access to the web management panel and admin API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdminCfg {
    /// Whether the admin panel and admin API (`/admin`, `/admin/api/*`) are enabled.
    #[serde(default = "default_admin_enabled")]
    pub enabled: bool,
    /// Name of the env variable holding the admin token. If unset (or empty),
    /// a token is generated at startup and printed to the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    /// Optional separate bind address for the admin surface (e.g. `127.0.0.1:9001`).
    /// If unset, the admin interface lives on the main `listen` address under `/admin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
}
impl Default for AdminCfg {
    fn default() -> Self {
        Self {
            enabled: default_admin_enabled(),
            token_env: None,
            listen: None,
        }
    }
}
fn default_admin_enabled() -> bool {
    true
}

/// Request statistics tracking (for the dashboard and `/metrics`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatsCfg {
    /// Whether to track request statistics.
    #[serde(default = "default_stats_enabled")]
    pub enabled: bool,
    /// Append-only request log file (JSONL). Empty means in-memory only.
    #[serde(default = "default_stats_file")]
    pub file: String,
    /// How many recent requests to keep in memory (the "recent" ring buffer).
    #[serde(default = "default_stats_ring")]
    pub ring_size: usize,
    /// Retention window for aggregates/time-series replayed on startup, e.g. `7d`.
    #[serde(default = "default_stats_retention")]
    pub retention: String,
}
impl Default for StatsCfg {
    fn default() -> Self {
        Self {
            enabled: default_stats_enabled(),
            file: default_stats_file(),
            ring_size: default_stats_ring(),
            retention: default_stats_retention(),
        }
    }
}
fn default_stats_enabled() -> bool {
    true
}
fn default_stats_file() -> String {
    "config/stats.jsonl".into()
}
fn default_stats_ring() -> usize {
    500
}
fn default_stats_retention() -> String {
    "7d".into()
}

impl Config {
    /// Load and validate the config from a file.
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read config {path}: {e}"))?;
        let cfg: Config = toml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Integrity validation: non-empty model pool, valid routing targets.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.models.is_empty() {
            anyhow::bail!("config: at least one [[models]] entry is required");
        }
        // uniqueness of model ids
        let mut seen = std::collections::HashSet::new();
        for m in &self.models {
            if !seen.insert(m.id.as_str()) {
                anyhow::bail!("config: duplicate model id '{}'", m.id);
            }
        }
        // every target in [routing] must exist in the model pool
        let ids: std::collections::HashSet<&str> =
            self.models.iter().map(|m| m.id.as_str()).collect();
        if !ids.contains(self.routing.default.as_str()) {
            anyhow::bail!(
                "config: routing.default '{}' is not a known model id",
                self.routing.default
            );
        }
        for (tier, targets) in &self.routing.tiers {
            let list = match targets {
                TierTargets::List(v) => v.clone(),
                TierTargets::One(s) => vec![s.clone()],
            };
            for id in &list {
                if !ids.contains(id.as_str()) {
                    anyhow::bail!(
                        "config: routing.{} references unknown model id '{}'",
                        tier,
                        id
                    );
                }
            }
        }
        Ok(())
    }

    /// Serialize the config to a TOML string (no secrets — they are intentionally absent here).
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        let header = "# Cortiq Gateway — configuration (managed via the admin panel).\n\
                      # This file is rewritten by the panel; manual edits are preserved\n\
                      # only until the next change made through the UI.\n\n";
        let body = toml::to_string_pretty(self)?;
        Ok(format!("{header}{body}"))
    }

    /// Atomically save the config to a file (write to a temporary file then rename).
    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        self.validate()?;
        let data = self.to_toml_string()?;
        let p = std::path::Path::new(path);
        let dir = p.parent().unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(dir).ok();
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, data.as_bytes())
            .map_err(|e| anyhow::anyhow!("cannot write config tmp {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, p)
            .map_err(|e| anyhow::anyhow!("cannot replace config {path}: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
listen = "0.0.0.0:9000"

[router]
url = "http://localhost:8080"
api_key_env = "CORTIQ_ROUTER_KEY"

[[models]]
id = "local-qwen"
provider = "openai"
base_url = "http://localhost:8000/v1"
model = "qwen2.5-7b-instruct"
cost_tier = "cheap"
caps = ["tools"]

[[models]]
id = "gpt-4o-mini"
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
cost_tier = "mid"
price_in = 0.15
price_out = 0.6

[routing]
low = ["local-qwen"]
medium = ["gpt-4o-mini", "local-qwen"]
default = "gpt-4o-mini"

[routing.policy]
mode = "cost_aware"
max_cost_usd_per_request = 0.5
min_class = { low = "cheap", medium = "mid" }
"#;

    #[test]
    fn roundtrip_preserves_routing_and_models() {
        let cfg: Config = toml::from_str(SAMPLE).expect("parse sample");
        cfg.validate().expect("sample valid");

        // key risk: serialization of flattened [routing] + nested policy
        let serialized = cfg.to_toml_string().expect("serialize");
        let back: Config = toml::from_str(&serialized).expect("re-parse serialized");
        back.validate().expect("roundtrip valid");

        assert_eq!(back.listen, "0.0.0.0:9000");
        assert_eq!(back.models.len(), 2);
        assert_eq!(back.routing.default, "gpt-4o-mini");
        assert_eq!(back.routing.policy.mode, "cost_aware");
        assert_eq!(back.routing.policy.max_cost_usd_per_request, Some(0.5));
        assert_eq!(
            back.routing.policy.min_class.get("low").map(String::as_str),
            Some("cheap")
        );

        let low = back.routing.tiers.get("low").expect("low tier present");
        match low {
            TierTargets::List(v) => assert_eq!(v, &vec!["local-qwen".to_string()]),
            TierTargets::One(s) => assert_eq!(s, "local-qwen"),
        }

        // admin/stats sections receive defaults
        assert!(back.admin.enabled);
        assert!(back.stats.enabled);
    }

    #[test]
    fn validate_rejects_unknown_routing_target() {
        let mut cfg: Config = toml::from_str(SAMPLE).unwrap();
        cfg.routing.default = "does-not-exist".into();
        assert!(cfg.validate().is_err());
    }
}
