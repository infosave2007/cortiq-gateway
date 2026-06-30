//! Semantic (embedding-based) response cache.
//!
//! Each entry stores an L2-normalized embedding of the request plus the response.
//! A lookup returns a cached answer when the cosine similarity to a previous request
//! (with the same routing signature, within TTL) meets the configured threshold —
//! skipping the expensive model call. In-memory ring buffer; counters track hit
//! rate and the model cost saved.

use crate::config::CacheCfg;
use crate::stats::parse_duration_secs;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct Entry {
    sig: String,
    vec: Vec<f32>, // L2-normalized
    ts: u64,
    model_used: String,
    content: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    cost: f64,
}

pub struct CacheHit {
    pub model_used: String,
    pub content: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub similarity: f32,
}

pub struct SemanticCache {
    enabled: bool,
    threshold: f32,
    ttl_secs: u64,
    max_entries: usize,
    embed_model: Option<String>,
    entries: Mutex<VecDeque<Entry>>,
    hits: AtomicU64,
    misses: AtomicU64,
    saved_micro_usd: AtomicU64,
}

impl SemanticCache {
    pub fn new(cfg: &CacheCfg) -> Arc<Self> {
        Arc::new(Self {
            enabled: cfg.enabled,
            threshold: cfg.threshold,
            ttl_secs: parse_duration_secs(&cfg.ttl).unwrap_or(3600),
            max_entries: cfg.max_entries.max(1),
            embed_model: cfg.embed_model.clone(),
            entries: Mutex::new(VecDeque::new()),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            saved_micro_usd: AtomicU64::new(0),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn embed_model(&self) -> Option<&str> {
        self.embed_model.as_deref()
    }

    /// Look up a cached response by routing signature + query embedding.
    pub fn lookup(&self, sig: &str, query: &[f32]) -> Option<CacheHit> {
        if !self.enabled {
            return None;
        }
        let q = normalized(query);
        let cutoff = now_secs().saturating_sub(self.ttl_secs);
        let inner = self.entries.lock().unwrap();
        let mut best: Option<(f32, &Entry)> = None;
        for e in inner.iter() {
            if e.sig != sig || e.ts < cutoff {
                continue;
            }
            let sim = dot(&e.vec, &q);
            if sim >= self.threshold && best.map(|(b, _)| sim > b).unwrap_or(true) {
                best = Some((sim, e));
            }
        }
        match best {
            Some((sim, e)) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                self.saved_micro_usd
                    .fetch_add((e.cost * 1_000_000.0) as u64, Ordering::Relaxed);
                Some(CacheHit {
                    model_used: e.model_used.clone(),
                    content: e.content.clone(),
                    prompt_tokens: e.prompt_tokens,
                    completion_tokens: e.completion_tokens,
                    similarity: sim,
                })
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Store a response under its routing signature + query embedding.
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &self,
        sig: String,
        query: Vec<f32>,
        model_used: String,
        content: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        cost: f64,
    ) {
        if !self.enabled || query.is_empty() {
            return;
        }
        let vec = normalized(&query);
        let mut inner = self.entries.lock().unwrap();
        inner.push_back(Entry {
            sig,
            vec,
            ts: now_secs(),
            model_used,
            content,
            prompt_tokens,
            completion_tokens,
            cost,
        });
        while inner.len() > self.max_entries {
            inner.pop_front();
        }
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let h = self.hits.load(Ordering::Relaxed);
        let m = self.misses.load(Ordering::Relaxed);
        serde_json::json!({
            "enabled": self.enabled,
            "hits": h,
            "misses": m,
            "hit_rate": if h + m > 0 { h as f64 / (h + m) as f64 } else { 0.0 },
            "entries": self.entries.lock().unwrap().len(),
            "saved_usd": self.saved_micro_usd.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        })
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn normalized(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter().map(|x| x / n).collect()
    } else {
        v.to_vec()
    }
}
