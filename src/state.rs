//! Shared gateway state accessible across all request handlers.
//!
//! The config-derived part (`Live`) lives behind an `ArcSwap`, which allows
//! hot-reloading the model pool, routing table, and router connection without
//! a restart: the admin API validates the new config, builds a new `Live`,
//! atomically writes the TOML, and swaps the pointer.

use crate::cache::SemanticCache;
use crate::config::Config;
use crate::registry::Registry;
use crate::router_client::RouterClient;
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
    pub fn build(cfg: Config, secrets: &SecretStore) -> anyhow::Result<Self> {
        let router = RouterClient::new(&cfg.router)?;
        let registry = Registry::from_config(&cfg, secrets)?;
        let routing = RoutingTable::from_config(&cfg);
        Ok(Self {
            cfg,
            registry,
            routing,
            router,
        })
    }
}

pub struct AppState {
    pub live: ArcSwap<Live>,
    pub stats: Arc<Stats>,
    pub cache: Arc<SemanticCache>,
    pub secrets: SecretStore,
    pub config_path: String,
    pub pipeline: crate::pipeline::Pipeline,
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
        let cache = SemanticCache::new(&cfg.cache);
        let live = Live::build(cfg, &secrets)?;
        Ok(SharedState(Arc::new(AppState {
            live: ArcSwap::from_pointee(live),
            stats,
            cache,
            secrets,
            config_path,
            pipeline: crate::pipeline::Pipeline::new(),
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
        let live = Live::build(new_cfg.clone(), &self.secrets)?;
        new_cfg.save(&self.config_path)?;
        self.live.store(Arc::new(live));
        Ok(())
    }

    /// Rebuild `Live` from the current config without writing to disk
    /// (e.g. after a secret change — only provider keys changed).
    pub fn rebuild(&self) -> anyhow::Result<()> {
        let cfg = self.live.load_full().cfg.clone();
        let live = Live::build(cfg, &self.secrets)?;
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
