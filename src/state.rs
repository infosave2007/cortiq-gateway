//! Shared gateway state accessible across all request handlers.
//!
//! The config-derived part (`Live`) lives behind an `ArcSwap`, which allows
//! hot-reloading the model pool, routing table, and router connection without
//! a restart: the admin API validates the new config, builds a new `Live`,
//! atomically writes the TOML, and swaps the pointer.

use crate::cache::SemanticCache;
use crate::config::Config;
use crate::registry::Registry;
use crate::router_client::{RouterClient, RouterLastStatus};
use crate::routing::RoutingTable;
use crate::secrets::SecretStore;
use crate::stats::Stats;
use arc_swap::ArcSwap;
use std::sync::Arc;

#[derive(Clone)]
pub struct SharedState(pub Arc<AppState>);

/// The portion of state that is rebuilt whenever the config changes.
pub struct Live {
    pub cfg: Config,
    pub registry: Registry,
    pub routing: RoutingTable,
    pub router: RouterClient,
}

impl Live {
    pub fn build(
        cfg: Config,
        secrets: &SecretStore,
        router_status: RouterLastStatus,
    ) -> anyhow::Result<Self> {
        let router = RouterClient::new(&cfg.router, secrets, router_status)?;
        let registry = Registry::from_config(&cfg, secrets)?;
        let routing = RoutingTable::from_config(&cfg);
        Ok(Self {
            cfg,
            registry,
            routing,
            router,
        })
    }

    /// Local-first fallback targets, in order: the managed local CMF model (if
    /// configured), then the routing default, then any registered model — only
    /// registered ids are returned. Used whenever the router is bypassed
    /// (local-only / disabled) or unavailable, so CMF models serve with no router.
    pub fn local_candidates(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.cfg.cmf.manage_server && !self.cfg.cmf.local_model.trim().is_empty() {
            v.push(self.cfg.cmf.model_id.clone());
        }
        let def = self.routing.default_model().to_string();
        if !def.is_empty() && !v.contains(&def) {
            v.push(def);
        }
        v.retain(|id| self.registry.get(id).is_some());
        if v.is_empty() {
            if let Some(a) = self.registry.any_id() {
                v.push(a);
            }
        }
        v
    }
}

pub struct AppState {
    pub live: ArcSwap<Live>,
    pub stats: Arc<Stats>,
    pub promotion: Arc<crate::promotion::Promotion>,
    pub imports: Arc<crate::import::JobStore>,
    /// Managed local CMF model server (install/update/spawn lifecycle + status).
    pub cmf: Arc<crate::cmf_runtime::CmfRuntime>,
    pub cache: Arc<SemanticCache>,
    pub secrets: SecretStore,
    pub config_path: String,
    pub pipeline: crate::pipeline::Pipeline,
    /// Outcome of the most recent router call (survives config reloads) —
    /// lets the admin panel distinguish a bad/expired key from a down router.
    pub router_status: RouterLastStatus,
}

/// Map the user-facing `[shadow]` config to the promotion-table tuning.
fn promotion_cfg(s: &crate::config::ShadowCfg) -> crate::promotion::PromotionCfg {
    crate::promotion::PromotionCfg {
        enabled: s.enabled && !s.local_model_id.is_empty(),
        window: s.window,
        n_min: s.n_min,
        promote_lb: s.promote_lb,
        soak: s.soak,
        file: if s.file.is_empty() { None } else { Some(s.file.clone()) },
        ..Default::default()
    }
}

impl std::ops::Deref for SharedState {
    type Target = AppState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SharedState {
    pub fn build(cfg: Config, config_path: String) -> anyhow::Result<Self> {
        let secrets_path = sibling(&config_path, "secrets.toml");
        let secrets = SecretStore::load(&secrets_path);
        let stats = Stats::new(&cfg.stats);
        let promotion = crate::promotion::Promotion::new(promotion_cfg(&cfg.shadow));
        let imports = crate::import::JobStore::new();
        let cmf = crate::cmf_runtime::CmfRuntime::new();
        let cache = SemanticCache::new(&cfg.cache);
        let router_status = RouterLastStatus::default();
        let live = Live::build(cfg, &secrets, router_status.clone())?;
        Ok(SharedState(Arc::new(AppState {
            live: ArcSwap::from_pointee(live),
            stats,
            promotion,
            imports,
            cmf,
            cache,
            secrets,
            config_path,
            pipeline: crate::pipeline::Pipeline::new(),
            router_status,
        })))
    }

    /// Current snapshot of the rebuildable state (safe to hold across await points).
    pub fn live(&self) -> Arc<Live> {
        self.0.live.load_full()
    }
}

impl AppState {
    /// Replace the config entirely: validate → build → write to disk → swap.
    /// If any step fails, the active state and file are left unchanged.
    pub fn reload(&self, new_cfg: Config) -> anyhow::Result<()> {
        new_cfg.validate()?;
        let live = Live::build(new_cfg.clone(), &self.secrets, self.router_status.clone())?;
        new_cfg.save(&self.config_path)?;
        self.live.store(Arc::new(live));
        Ok(())
    }

    /// Rebuild `Live` from the current config without writing to disk
    /// (e.g. after a secret change — only provider keys changed).
    pub fn rebuild(&self) -> anyhow::Result<()> {
        let cfg = self.live.load_full().cfg.clone();
        let live = Live::build(cfg, &self.secrets, self.router_status.clone())?;
        self.live.store(Arc::new(live));
        Ok(())
    }
}

/// Path to file `name` next to `path` (in the same directory).
fn sibling(path: &str, name: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .map(|d| d.join(name))
        .unwrap_or_else(|| std::path::PathBuf::from(name))
        .to_string_lossy()
        .into_owned()
}
