//! Secret store (provider API key values) kept outside the main config.
//!
//! The file `config/secrets.toml` (in `.gitignore`, mode `0600`) is a map of
//! `env_name -> value`. Key resolution order: store entry > environment variable.
//! This allows setting keys directly from the admin panel without a restart,
//! without writing secrets into `gateway.toml`. Values are never exposed via the
//! API — only their presence status is returned (see `source`).

use std::collections::BTreeMap;
use std::sync::RwLock;

pub struct SecretStore {
    path: String,
    values: RwLock<BTreeMap<String, String>>,
}

impl SecretStore {
    /// Load the store from a file (missing file or parse error → empty store).
    pub fn load(path: &str) -> Self {
        let values = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| toml::from_str::<BTreeMap<String, String>>(&raw).ok())
            .unwrap_or_default();
        Self {
            path: path.to_string(),
            values: RwLock::new(values),
        }
    }

    /// Secret value by env name: store first, then environment variable.
    pub fn resolve(&self, env_name: &str) -> Option<String> {
        if let Some(v) = self.values.read().unwrap().get(env_name) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
        std::env::var(env_name).ok().filter(|v| !v.is_empty())
    }

    /// Value source for display in the UI: `store` | `env` | `missing`.
    pub fn source(&self, env_name: &str) -> &'static str {
        let in_store = self
            .values
            .read()
            .unwrap()
            .get(env_name)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if in_store {
            "store"
        } else if std::env::var(env_name)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
        {
            "env"
        } else {
            "missing"
        }
    }

    /// Set a secret (persisted to file with mode 0600).
    pub fn set(&self, env_name: &str, value: &str) -> anyhow::Result<()> {
        self.values
            .write()
            .unwrap()
            .insert(env_name.to_string(), value.to_string());
        self.persist()
    }

    /// Remove a secret from the store (an environment variable value, if present, remains).
    pub fn clear(&self, env_name: &str) -> anyhow::Result<()> {
        self.values.write().unwrap().remove(env_name);
        self.persist()
    }

    fn persist(&self) -> anyhow::Result<()> {
        let data = {
            let map = self.values.read().unwrap();
            toml::to_string(&*map)?
        };
        let p = std::path::Path::new(&self.path);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, data.as_bytes())?;
        std::fs::rename(&tmp, p)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}
