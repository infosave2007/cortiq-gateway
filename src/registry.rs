//! Model pool registry: builds providers from `[[models]]` and resolves
//! `model_id → Arc<dyn Provider>`.

use crate::config::{Config, ModelCfg};
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
                // openrouter + lmstudio are OpenAI-compatible (Bearer + /chat/completions)
                "openai" | "ollama" | "http" | "openrouter" | "lmstudio" => {
                    Arc::new(OpenAiProvider::new(m, api_key)?)
                }
                "anthropic" => Arc::new(AnthropicProvider::new(m, api_key)?),
                other => anyhow::bail!("unknown provider '{}' for model '{}'", other, m.id),
            };
            by_id.insert(m.id.clone(), provider);
        }

        // Managed local CMF servers: register each as an OpenAI-compatible
        // provider so it can be routed to like any other backend — including when
        // the router is unavailable or the gateway is in local-only mode.
        for s in cfg.cmf.effective_servers() {
            if by_id.contains_key(&s.id) {
                continue; // a static [[models]] entry with the same id wins
            }
            let m = ModelCfg {
                id: s.id.clone(),
                provider: "openai".into(),
                base_url: format!("http://{}:{}/v1", cfg.cmf.local_host, s.port),
                model: "cortiq".into(),
                cost_tier: "local".into(),
                price_in: 0.0,
                price_out: 0.0,
                kind: "chat".into(),
                api_key_env: None,
                caps: Vec::new(),
                temperature: s.temperature,
                top_p: s.top_p,
                max_tokens: s.max_tokens,
                think_budget: s.think_budget,
                system_prompt: s.system_prompt.clone(),
                o1: s.o1.clone(),
                skip_mtp: s.skip_mtp,
            };
            by_id.insert(m.id.clone(), Arc::new(OpenAiProvider::new(&m, None)?));
        }
        Ok(Self { by_id })
    }

    /// Any model id, preferring a real registered one (used as a local-first
    /// fallback target when routing cannot pick a specific model).
    pub fn any_id(&self) -> Option<String> {
        self.by_id.keys().next().cloned()
    }

    pub fn get(&self, model_id: &str) -> Option<Arc<dyn Provider>> {
        self.by_id.get(model_id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.by_id.keys()
    }
}
