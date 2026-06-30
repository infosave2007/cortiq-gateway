//! Model pool registry: builds providers from `[[models]]` and resolves
//! `model_id → Arc<dyn Provider>`.

use crate::config::Config;
use crate::providers::{anthropic::AnthropicProvider, openai::OpenAiProvider, Provider};
use crate::secrets::SecretStore;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Registry {
    by_id: HashMap<String, Arc<dyn Provider>>,
}

impl Registry {
    pub fn from_config(cfg: &Config, secrets: &SecretStore) -> anyhow::Result<Self> {
        let mut by_id: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        for m in &cfg.models {
            // provider key: secret store → environment variable
            let api_key = m
                .api_key_env
                .as_ref()
                .and_then(|name| secrets.resolve(name));
            let provider: Arc<dyn Provider> = match m.provider.as_str() {
                "openai" | "ollama" | "http" => Arc::new(OpenAiProvider::new(m, api_key)?),
                "anthropic" => Arc::new(AnthropicProvider::new(m, api_key)?),
                other => anyhow::bail!("unknown provider '{}' for model '{}'", other, m.id),
            };
            by_id.insert(m.id.clone(), provider);
        }
        Ok(Self { by_id })
    }

    pub fn get(&self, model_id: &str) -> Option<Arc<dyn Provider>> {
        self.by_id.get(model_id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.by_id.keys()
    }
}
