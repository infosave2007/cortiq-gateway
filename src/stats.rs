//! Request statistics tracking for the dashboard and `GET /metrics`.
//!
//! Everything is held in memory (aggregates + per-minute buckets for the retention
//! window + a ring buffer of recent requests) and optionally appended to a JSONL
//! file that is replayed on startup — so statistics survive a restart with no
//! database dependency.

use crate::config::StatsCfg;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A single record for a completed request (one JSONL line and one "recent" ring entry).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestRecord {
    pub ts: u64,
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub directive: String, // auto | pinned
    #[serde(default)]
    pub task_label: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub score: f32,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub route_source: String,
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub outcome: String, // ok | error
    #[serde(default)]
    pub failover: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Totals {
    pub requests: u64,
    pub ok: u64,
    pub errors: u64,
    pub failovers: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms_sum: u64,
}

impl Totals {
    fn apply(&mut self, r: &RequestRecord) {
        self.requests += 1;
        if r.outcome == "ok" {
            self.ok += 1;
        } else {
            self.errors += 1;
        }
        if r.failover {
            self.failovers += 1;
        }
        self.prompt_tokens += r.prompt_tokens as u64;
        self.completion_tokens += r.completion_tokens as u64;
        self.cost_usd += r.cost_usd;
        self.latency_ms_sum += r.latency_ms;
    }
    /// Accumulate another Totals into this one (for windowed aggregation).
    fn merge(&mut self, o: &Totals) {
        self.requests += o.requests;
        self.ok += o.ok;
        self.errors += o.errors;
        self.failovers += o.failovers;
        self.prompt_tokens += o.prompt_tokens;
        self.completion_tokens += o.completion_tokens;
        self.cost_usd += o.cost_usd;
        self.latency_ms_sum += o.latency_ms_sum;
    }
}

/// Per-minute bucket. Beyond the chart fields it carries the per-group breakdown
/// and failovers so the admin snapshot can be computed for ANY time window by
/// summing the in-window buckets (the internal fields are not serialized into
/// the `series` payload).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Bucket {
    pub minute: u64, // unix seconds, rounded down to the minute
    pub requests: u64,
    pub errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms_sum: u64,
    #[serde(skip)]
    pub failovers: u64,
    #[serde(skip)]
    pub by_model: HashMap<String, Totals>,
    #[serde(skip)]
    pub by_account: HashMap<String, Totals>,
    #[serde(skip)]
    pub by_tier: HashMap<String, Totals>,
    #[serde(skip)]
    pub by_label: HashMap<String, Totals>,
}

#[derive(Default)]
struct Inner {
    /// All-time totals + per-model (used by the Prometheus counters, which must
    /// be monotonic — the windowed admin snapshot is derived from the buckets).
    total: Totals,
    by_model: HashMap<String, Totals>,
    /// Per-minute buckets — the single source for the windowed admin snapshot.
    buckets: VecDeque<Bucket>,
    recent: VecDeque<RequestRecord>,
}

pub struct Stats {
    enabled: bool,
    file: Option<String>,
    ring_size: usize,
    retention_secs: u64,
    inner: Mutex<Inner>,
}

impl Stats {
    pub fn new(cfg: &StatsCfg) -> std::sync::Arc<Self> {
        let file = if cfg.file.trim().is_empty() {
            None
        } else {
            Some(cfg.file.clone())
        };
        let s = Stats {
            enabled: cfg.enabled,
            file,
            ring_size: cfg.ring_size.max(1),
            retention_secs: parse_duration_secs(&cfg.retention).unwrap_or(7 * 86_400),
            inner: Mutex::new(Inner::default()),
        };
        s.replay();
        std::sync::Arc::new(s)
    }

    /// Replay the JSONL file into aggregates (within the retention window only).
    fn replay(&self) {
        let Some(path) = &self.file else { return };
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        let cutoff = now_secs().saturating_sub(self.retention_secs);
        let mut inner = self.inner.lock().unwrap();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<RequestRecord>(line) {
                // push everything to the "recent" ring (it self-limits by size),
                // but apply to aggregates/series only entries within the window.
                Self::push_recent(&mut inner, &rec, self.ring_size);
                if rec.ts >= cutoff {
                    Self::apply_aggregates(&mut inner, &rec, self.retention_secs);
                }
            }
        }
    }

    fn push_recent(inner: &mut Inner, rec: &RequestRecord, ring_size: usize) {
        inner.recent.push_back(rec.clone());
        while inner.recent.len() > ring_size {
            inner.recent.pop_front();
        }
    }

    fn apply_aggregates(inner: &mut Inner, rec: &RequestRecord, retention_secs: u64) {
        // all-time counters for Prometheus; the windowed snapshot is derived
        // from the buckets below.
        inner.total.apply(rec);
        inner
            .by_model
            .entry(rec.model_id.clone())
            .or_default()
            .apply(rec);

        let minute = rec.ts - (rec.ts % 60);
        match inner.buckets.back_mut() {
            Some(b) if b.minute == minute => fill_bucket(b, rec),
            Some(b) if b.minute > minute => { /* arrived older than the tail — ignore for series */
            }
            _ => {
                let mut b = Bucket {
                    minute,
                    ..Default::default()
                };
                fill_bucket(&mut b, rec);
                inner.buckets.push_back(b);
            }
        }
        // trim old buckets outside the retention window
        let cutoff = now_secs().saturating_sub(retention_secs);
        while let Some(front) = inner.buckets.front() {
            if front.minute < cutoff {
                inner.buckets.pop_front();
            } else {
                break;
            }
        }
    }

    /// Record a new request event.
    pub fn record(&self, rec: RequestRecord) {
        if !self.enabled {
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            Self::apply_aggregates(&mut inner, &rec, self.retention_secs);
            Self::push_recent(&mut inner, &rec, self.ring_size);
        }
        if let Some(path) = &self.file {
            if let Ok(line) = serde_json::to_string(&rec) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
    }

    /// Snapshot for the admin API: aggregates + time series for `range_secs` + breakdown by `groupby`.
    pub fn snapshot(&self, range_secs: u64, groupby: &str) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        let cutoff = now_secs().saturating_sub(range_secs);
        let series: Vec<&Bucket> = inner
            .buckets
            .iter()
            .filter(|b| b.minute >= cutoff)
            .collect();

        // Windowed totals + breakdown = sum of the in-window buckets, so the KPIs
        // and the breakdown recompute whenever the range changes.
        let mut win = Totals::default();
        let mut group: HashMap<String, Totals> = HashMap::new();
        for b in &series {
            win.requests += b.requests;
            win.errors += b.errors;
            win.ok += b.requests.saturating_sub(b.errors);
            win.failovers += b.failovers;
            win.prompt_tokens += b.prompt_tokens;
            win.completion_tokens += b.completion_tokens;
            win.cost_usd += b.cost_usd;
            win.latency_ms_sum += b.latency_ms_sum;
            let src = match groupby {
                "account" => &b.by_account,
                "tier" => &b.by_tier,
                "label" => &b.by_label,
                _ => &b.by_model,
            };
            for (k, t) in src {
                group.entry(k.clone()).or_default().merge(t);
            }
        }

        let mut breakdown: Vec<serde_json::Value> = group
            .iter()
            .map(|(k, t)| {
                serde_json::json!({
                    "key": k,
                    "requests": t.requests,
                    "ok": t.ok,
                    "errors": t.errors,
                    "prompt_tokens": t.prompt_tokens,
                    "completion_tokens": t.completion_tokens,
                    "cost_usd": t.cost_usd,
                    "avg_latency_ms": avg(t.latency_ms_sum, t.requests),
                })
            })
            .collect();
        breakdown.sort_by(|a, b| {
            b["requests"]
                .as_u64()
                .unwrap_or(0)
                .cmp(&a["requests"].as_u64().unwrap_or(0))
        });

        serde_json::json!({
            "totals": {
                "requests": win.requests,
                "ok": win.ok,
                "errors": win.errors,
                "failovers": win.failovers,
                "prompt_tokens": win.prompt_tokens,
                "completion_tokens": win.completion_tokens,
                "total_tokens": win.prompt_tokens + win.completion_tokens,
                "cost_usd": win.cost_usd,
                "avg_latency_ms": avg(win.latency_ms_sum, win.requests),
                "success_rate": if win.requests > 0 {
                    win.ok as f64 / win.requests as f64
                } else { 0.0 },
            },
            "groupby": groupby,
            "breakdown": breakdown,
            "series": series,
        })
    }

    /// Reset all in-memory stats and truncate the JSONL log — the "clear logs"
    /// admin action.
    pub fn clear(&self) {
        {
            let mut inner = self.inner.lock().unwrap();
            *inner = Inner::default();
        }
        if let Some(path) = &self.file {
            let _ = std::fs::write(path, "");
        }
    }

    /// Recent requests (newest first), with pagination.
    pub fn recent(&self, limit: usize, offset: usize) -> Vec<RequestRecord> {
        let inner = self.inner.lock().unwrap();
        inner
            .recent
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Metrics text in Prometheus format.
    pub fn prometheus(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut out = String::new();
        let t = &inner.total;
        out.push_str("# HELP gw_requests_total Total gateway requests.\n");
        out.push_str("# TYPE gw_requests_total counter\n");
        out.push_str(&format!("gw_requests_total {}\n", t.requests));
        out.push_str("# HELP gw_failovers_total Total provider failovers.\n");
        out.push_str("# TYPE gw_failovers_total counter\n");
        out.push_str(&format!("gw_failovers_total {}\n", t.failovers));
        out.push_str("# HELP gw_tokens_total Total tokens by direction.\n");
        out.push_str("# TYPE gw_tokens_total counter\n");
        out.push_str(&format!(
            "gw_tokens_total{{direction=\"in\"}} {}\n",
            t.prompt_tokens
        ));
        out.push_str(&format!(
            "gw_tokens_total{{direction=\"out\"}} {}\n",
            t.completion_tokens
        ));
        out.push_str("# HELP gw_cost_usd_total Total estimated cost in USD.\n");
        out.push_str("# TYPE gw_cost_usd_total counter\n");
        out.push_str(&format!("gw_cost_usd_total {:.6}\n", t.cost_usd));
        out.push_str("# HELP gw_provider_calls_total Calls by model and outcome.\n");
        out.push_str("# TYPE gw_provider_calls_total counter\n");
        for (model, mt) in &inner.by_model {
            let model = escape_label(model);
            out.push_str(&format!(
                "gw_provider_calls_total{{model_id=\"{model}\",outcome=\"ok\"}} {}\n",
                mt.ok
            ));
            out.push_str(&format!(
                "gw_provider_calls_total{{model_id=\"{model}\",outcome=\"error\"}} {}\n",
                mt.errors
            ));
        }
        out
    }
}

fn fill_bucket(b: &mut Bucket, r: &RequestRecord) {
    b.requests += 1;
    if r.outcome != "ok" {
        b.errors += 1;
    }
    if r.failover {
        b.failovers += 1;
    }
    b.prompt_tokens += r.prompt_tokens as u64;
    b.completion_tokens += r.completion_tokens as u64;
    b.cost_usd += r.cost_usd;
    b.latency_ms_sum += r.latency_ms;
    // per-group breakdown, so the snapshot can be windowed by summing buckets
    b.by_model.entry(r.model_id.clone()).or_default().apply(r);
    b.by_account
        .entry(if r.account.is_empty() {
            "anonymous".to_string()
        } else {
            r.account.clone()
        })
        .or_default()
        .apply(r);
    b.by_tier.entry(r.tier.clone()).or_default().apply(r);
    b.by_label.entry(r.task_label.clone()).or_default().apply(r);
}

fn avg(sum: u64, n: u64) -> u64 {
    sum.checked_div(n).unwrap_or(0)
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

/// Parse a duration string of the form `7d` / `24h` / `90m` / `3600s` into seconds.
pub fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = if let Some(n) = s.strip_suffix('d') {
        (n, 86_400)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1)
    } else {
        (s, 1)
    };
    num.trim().parse::<u64>().ok().map(|v| v * mult)
}
