//! Configuration loading from TOML. Structs mirror config/gateway.example.toml.
//! Secrets are read from environment variables by the names given in `*_env` fields.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Base directory for gateway-managed data (models, logs). Honors
/// `$CORTIQ_GATEWAY_HOME`, then `$XDG_DATA_HOME/cortiq-gateway`, then
/// `~/.cortiq-gateway` — so defaults work regardless of the process CWD
/// (important once the gateway is installed rather than run from its repo).
pub fn data_dir() -> PathBuf {
    for (var, sub) in [("CORTIQ_GATEWAY_HOME", ""), ("XDG_DATA_HOME", "cortiq-gateway"), ("HOME", ".cortiq-gateway")] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return if sub.is_empty() { PathBuf::from(v) } else { PathBuf::from(v).join(sub) };
            }
        }
    }
    PathBuf::from("data")
}

fn data_path(name: &str) -> String {
    data_dir().join(name).to_string_lossy().into_owned()
}

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
    #[serde(default)]
    pub cache: CacheCfg,
    #[serde(default)]
    pub shadow: ShadowCfg,
    #[serde(default)]
    pub cmf: CmfCfg,
}

/// CMF-format model factory: import a HuggingFace model, convert it to a
/// local `.cmf` (quantized), and serve it via cortiq-server. Points the
/// gateway at the Python converter + runtime on the host.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CmfCfg {
    /// Python interpreter that has torch + the converter deps.
    #[serde(default = "default_cmf_python")]
    pub python_bin: String,
    /// Path to `convert_dtgma_to_cmf.py`.
    #[serde(default = "default_cmf_converter")]
    pub converter: String,
    /// Directory where produced `.cmf` files are written.
    #[serde(default = "default_cmf_models_dir")]
    pub models_dir: String,
    /// Base URL of the cortiq-server that serves the `.cmf` (OpenAI /v1),
    /// used when registering a converted model as a provider.
    #[serde(default = "default_cmf_server_url")]
    pub cortiq_server_url: String,
    /// Secret name of a HuggingFace token (higher search rate limits +
    /// gated/private repos). Empty = anonymous public search.
    #[serde(default)]
    pub hf_token_env: String,
    /// `cortiq` CLI binary for the local server (from `cargo install cortiq-cli`).
    #[serde(default = "default_cortiq_bin")]
    pub cortiq_bin: String,
    /// Install cortiq-cli from crates.io automatically if the binary is missing.
    #[serde(default)]
    pub auto_install: bool,
    /// On startup, check crates.io and reinstall if a newer cortiq-cli exists.
    #[serde(default)]
    pub auto_update: bool,
    /// Spawn and manage a local `cortiq serve` for `local_model`, and register
    /// it in the model pool — a local `.cmf` backend the gateway can use even
    /// when no external provider (or the router) is available.
    #[serde(default)]
    pub manage_server: bool,
    /// Path to the local `.cmf` to serve (empty = disabled).
    #[serde(default)]
    pub local_model: String,
    /// Host the managed local server binds to.
    #[serde(default = "default_cmf_local_host")]
    pub local_host: String,
    /// Port the managed local server binds to.
    #[serde(default = "default_cmf_local_port")]
    pub local_port: u16,
    /// Model id the local server is registered under in the pool.
    #[serde(default = "default_cmf_model_id")]
    pub model_id: String,
    /// Use ONLY local models: route every request to the local model, ignoring
    /// the router and any external providers ("use only local models").
    #[serde(default)]
    pub local_only: bool,
}
impl Default for CmfCfg {
    fn default() -> Self {
        Self {
            python_bin: default_cmf_python(),
            converter: default_cmf_converter(),
            models_dir: default_cmf_models_dir(),
            cortiq_server_url: default_cmf_server_url(),
            hf_token_env: String::new(),
            cortiq_bin: default_cortiq_bin(),
            auto_install: false,
            auto_update: false,
            manage_server: false,
            local_model: String::new(),
            local_host: default_cmf_local_host(),
            local_port: default_cmf_local_port(),
            model_id: default_cmf_model_id(),
            local_only: false,
        }
    }
}
fn default_cortiq_bin() -> String {
    "cortiq".into()
}
fn default_cmf_local_host() -> String {
    "127.0.0.1".into()
}
fn default_cmf_local_port() -> u16 {
    8081
}
fn default_cmf_model_id() -> String {
    "cmf-local".into()
}
fn default_cmf_python() -> String {
    "python3".into()
}
fn default_cmf_converter() -> String {
    "../cmf/converter/convert_dtgma_to_cmf.py".into()
}
fn default_cmf_models_dir() -> String {
    data_path("models")
}
fn default_cmf_server_url() -> String {
    "http://127.0.0.1:8081/v1".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouterCfg {
    /// Master switch — when false the router is never called; the gateway
    /// serves from the local model pool (local `.cmf` / configured default).
    #[serde(default = "default_router_enabled")]
    pub enabled: bool,
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
fn default_router_enabled() -> bool {
    true
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
    data_path("stats.jsonl")
}
fn default_stats_ring() -> usize {
    500
}
fn default_stats_retention() -> String {
    "7d".into()
}

/// Self-warming shadow loop (docs/SELF_WARMING_GATEWAY.md). While a task
/// label is not yet promoted, the client is served the CLOUD answer; in
/// parallel (sampled, non-blocking) the local `cmf-local` model answers and
/// a cheap `judge` model scores it against the cloud answer. The per-label
/// Wilson-LB of that pass stream gates promotion to local serving.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShadowCfg {
    /// Master switch. Off = zero overhead, classic gateway behavior.
    #[serde(default)]
    pub enabled: bool,
    /// model_id (from `[[models]]`) of the local CMF model (OpenAI-compatible
    /// cortiq-server). Empty disables the loop.
    #[serde(default)]
    pub local_model_id: String,
    /// model_id of the cheap judge (different family than the answerer).
    #[serde(default)]
    pub judge_model_id: String,
    /// Fraction of eligible requests to shadow+judge (0..1). Bounds cost.
    #[serde(default = "default_shadow_sample")]
    pub sample_rate: f64,
    /// judge.jsonl append+replay path. Empty = in-memory only.
    #[serde(default = "default_shadow_file")]
    pub file: String,
    /// Wilson-95 lower bound of pass-rate required to promote.
    #[serde(default = "default_shadow_lb")]
    pub promote_lb: f64,
    /// Min judgments before promotion is considered.
    #[serde(default = "default_shadow_nmin")]
    pub n_min: usize,
    /// Rolling window of judgments per label.
    #[serde(default = "default_shadow_window")]
    pub window: usize,
    /// Extra canary judgments before full local serving.
    #[serde(default = "default_shadow_soak")]
    pub soak: usize,
    /// GATE past shadow-only: actually SERVE promoted labels from the local
    /// model (with failover to cloud on error + a complexity veto). Default
    /// OFF — the memo's 3 risks (§8) should be retired by measurement first.
    #[serde(default)]
    pub serve_when_promoted: bool,
}
impl Default for ShadowCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            local_model_id: String::new(),
            judge_model_id: String::new(),
            sample_rate: default_shadow_sample(),
            file: default_shadow_file(),
            promote_lb: default_shadow_lb(),
            n_min: default_shadow_nmin(),
            window: default_shadow_window(),
            soak: default_shadow_soak(),
            serve_when_promoted: false,
        }
    }
}
fn default_shadow_sample() -> f64 {
    0.15
}
fn default_shadow_file() -> String {
    data_path("judge.jsonl")
}
fn default_shadow_lb() -> f64 {
    0.95
}
fn default_shadow_nmin() -> usize {
    200
}
fn default_shadow_window() -> usize {
    500
}
fn default_shadow_soak() -> usize {
    100
}

/// Semantic (embedding-based) response cache — returns a cached answer for prompts
/// that are semantically near a previous one, skipping the (expensive) model call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheCfg {
    #[serde(default)]
    pub enabled: bool,
    /// Cosine-similarity threshold for a hit (0..1).
    #[serde(default = "default_semcache_threshold")]
    pub threshold: f32,
    /// Entry time-to-live, e.g. `1h`.
    #[serde(default = "default_semcache_ttl")]
    pub ttl: String,
    /// Maximum number of cached entries (ring buffer).
    #[serde(default = "default_semcache_max")]
    pub max_entries: usize,
    /// Embedding model id (defaults to the first `kind = "embedding"` model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_model: Option<String>,
}
impl Default for CacheCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: default_semcache_threshold(),
            ttl: default_semcache_ttl(),
            max_entries: default_semcache_max(),
            embed_model: None,
        }
    }
}
fn default_semcache_threshold() -> f32 {
    0.92
}
fn default_semcache_ttl() -> String {
    "1h".into()
}
fn default_semcache_max() -> usize {
    1000
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
        // A managed local CMF model counts as an available model even though it
        // is registered dynamically rather than via a [[models]] entry — this
        // is what lets the gateway run on local models alone (no cloud, no router).
        let has_local = self.cmf.manage_server && !self.cmf.local_model.trim().is_empty();
        if self.models.is_empty() && !has_local {
            anyhow::bail!("config: at least one [[models]] entry (or a managed local CMF model) is required");
        }
        // uniqueness of model ids
        let mut seen = std::collections::HashSet::new();
        for m in &self.models {
            if !seen.insert(m.id.as_str()) {
                anyhow::bail!("config: duplicate model id '{}'", m.id);
            }
        }
        // every target in [routing] must exist in the model pool
        let mut ids: std::collections::HashSet<&str> =
            self.models.iter().map(|m| m.id.as_str()).collect();
        if has_local {
            ids.insert(self.cmf.model_id.as_str());
        }
        if !self.routing.default.is_empty() && !ids.contains(self.routing.default.as_str()) {
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
