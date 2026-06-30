//! Model selection: `(task_label, complexity_tier) → ordered list of model_ids`.
//! The list serves as both a priority order and a fallback chain. See docs/ROUTING.md.

use crate::config::{Config, TierTargets};

pub struct RoutingTable {
    tiers: std::collections::HashMap<String, Vec<String>>,
    default: String,
}

impl RoutingTable {
    pub fn from_config(cfg: &Config) -> Self {
        let tiers = cfg
            .routing
            .tiers
            .iter()
            .map(|(tier, targets)| {
                let list = match targets {
                    TierTargets::List(v) => v.clone(),
                    TierTargets::One(s) => vec![s.clone()],
                };
                (tier.clone(), list)
            })
            .collect();
        Self {
            tiers,
            default: cfg.routing.default.clone(),
        }
    }

    /// Candidates for the given complexity tier, in preference order.
    /// If the tier is not defined, the sole candidate is `default`.
    pub fn candidates(&self, tier: &str) -> Vec<String> {
        self.tiers
            .get(tier)
            .cloned()
            .unwrap_or_else(|| vec![self.default.clone()])
    }

    /// Default model (used when the router is unavailable).
    pub fn default_model(&self) -> &str {
        &self.default
    }
}

// TODO(v0.2): cost_aware mode and circuit breaker — pick the cheapest available
// model no lower than min_class[tier], respecting max_cost_usd_per_request,
// and skip models whose circuit breaker is open.
