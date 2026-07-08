//! Cloud-as-judge promotion table (self-warming local model, MVP).
//!
//! Per task-label we accumulate a judge/checker PASS stream and gate the
//! per-label lifecycle on the **Wilson-95 LOWER bound** of the pass rate —
//! never on the point estimate, so a lucky streak of easy requests can NOT
//! promote a label (small n → wide interval → low bound → stays SHADOW).
//! The full design + honest holes: docs/SELF_WARMING_GATEWAY.md.
//!
//! This module is pure bookkeeping (like `stats::Stats`): it decides the
//! STATE; the pipeline consults the state to decide who serves. Nothing here
//! serves a local answer — that is the caller's decision, guarded by these
//! states plus a per-request veto.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// One judgment of a local answer (JSONL line, replayed on startup).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JudgeRecord {
    pub ts: u64,
    pub task_label: String,
    /// 1 = local answer accepted (checker pass OR judge score≥bar & !hard_fail).
    pub pass: bool,
    /// Fact hallucination / broken format / unsafe — blocks promotion hard.
    #[serde(default)]
    pub hard_fail: bool,
    /// Raw judge score 0..4 (diagnostic only); 4 for a $0 programmatic checker.
    #[serde(default)]
    pub score: u8,
    /// "checker" ($0 ground-truth) or "judge" (cloud LLM-as-judge).
    #[serde(default)]
    pub source: String,
    /// Local recon-E (novelty) at judge time — diagnostic; NOT a correctness gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recon_e: Option<f32>,
    /// Local calibrated Born-mass confidence — for the negative-control study.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born_conf: Option<f32>,
    /// Cloud tokens spent judging (0 for a programmatic checker).
    #[serde(default)]
    pub judge_tokens: u32,
}

/// Per-label lifecycle. Serving-locally is only ever Canary/LocalServed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromotionState {
    CloudOnly,
    ShadowJudged,
    Canary,
    LocalServed,
    Demoted,
}

/// Tuning (defaults from the memo; refit to measured ECE/traffic).
#[derive(Clone, Debug)]
pub struct PromotionCfg {
    pub enabled: bool,
    pub window: usize,       // rolling judgments per label (W)
    pub n_min: usize,        // min judgments before promotion is even considered
    pub promote_lb: f64,     // Wilson-95 LB of pass must clear this
    pub demote_pass: f64,    // point pass-rate below this → demote a served label
    pub hardfail_ub_max: f64,// Wilson-95 UPPER bound of hard_fail must stay under this
    pub soak: usize,         // extra Canary judgments before LocalServed
    pub z: f64,              // 1.96 = 95%
    pub file: Option<String>,// judge.jsonl path (append + replay)
}

impl Default for PromotionCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            window: 500,
            n_min: 200,
            promote_lb: 0.95,
            demote_pass: 0.90,
            hardfail_ub_max: 0.015,
            soak: 100,
            z: 1.96,
            file: None,
        }
    }
}

struct LabelState {
    passes: VecDeque<bool>,     // rolling window of pass/fail
    hard_fails: VecDeque<bool>, // rolling window of hard_fail flags
    state: PromotionState,
    soaked: usize,              // judgments accumulated while in Canary
}

impl LabelState {
    fn new() -> Self {
        Self {
            passes: VecDeque::new(),
            hard_fails: VecDeque::new(),
            state: PromotionState::ShadowJudged,
            soaked: 0,
        }
    }
}

pub struct Promotion {
    cfg: PromotionCfg,
    labels: Mutex<HashMap<String, LabelState>>,
}

/// Wilson score interval bound for a Bernoulli rate. `upper=false` → lower.
/// s successes of n trials, z the normal quantile. Distribution-aware: small
/// n gives a wide interval (low lower bound / high upper bound) by construction.
pub fn wilson_bound(s: usize, n: usize, z: f64, upper: bool) -> f64 {
    if n == 0 {
        return if upper { 1.0 } else { 0.0 };
    }
    let n = n as f64;
    let p = s as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = p + z2 / (2.0 * n);
    let margin = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
    let v = if upper { centre + margin } else { centre - margin };
    (v / denom).clamp(0.0, 1.0)
}

impl Promotion {
    pub fn new(cfg: PromotionCfg) -> std::sync::Arc<Self> {
        let p = std::sync::Arc::new(Self { cfg, labels: Mutex::new(HashMap::new()) });
        p.replay();
        p
    }

    fn replay(&self) {
        let Some(path) = &self.cfg.file else { return };
        let Ok(content) = std::fs::read_to_string(path) else { return };
        for line in content.lines() {
            if let Ok(rec) = serde_json::from_str::<JudgeRecord>(line) {
                self.apply(&rec);
            }
        }
    }

    /// Record a judgment: update the rolling windows, advance the state
    /// machine, append to judge.jsonl. Returns the (possibly new) state.
    pub fn record(&self, rec: JudgeRecord) -> PromotionState {
        let st = self.apply(&rec);
        if let Some(path) = &self.cfg.file {
            if let Ok(line) = serde_json::to_string(&rec) {
                if let Ok(mut f) =
                    std::fs::OpenOptions::new().create(true).append(true).open(path)
                {
                    let _ = writeln!(f, "{line}");
                }
            }
        }
        st
    }

    fn apply(&self, rec: &JudgeRecord) -> PromotionState {
        let mut map = self.labels.lock().unwrap();
        let ls = map.entry(rec.task_label.clone()).or_insert_with(LabelState::new);
        push_cap(&mut ls.passes, rec.pass, self.cfg.window);
        push_cap(&mut ls.hard_fails, rec.hard_fail, self.cfg.window);
        self.advance(ls);
        ls.state
    }

    fn advance(&self, ls: &mut LabelState) {
        let n = ls.passes.len();
        let s = ls.passes.iter().filter(|&&p| p).count();
        let hf = ls.hard_fails.iter().filter(|&&h| h).count();
        let lb = wilson_bound(s, n, self.cfg.z, false);
        let point = if n > 0 { s as f64 / n as f64 } else { 0.0 };
        let recent_hf = ls.hard_fails.iter().rev().take(50).filter(|&&h| h).count();

        // Zero-tolerance on hard-fails in the window: a Wilson UPPER bound on
        // hard-fail rate ≤ 0.015 is UNREACHABLE at n=200 even with 0 fails
        // (rule-of-three: 0/200 → ~0.019) — the panel flagged this. Requiring
        // ZERO recent hard-fails is achievable AND safer (a recent
        // hallucination/unsafe answer → stay on cloud until it ages out).
        let bar = n >= self.cfg.n_min && lb >= self.cfg.promote_lb && hf == 0;
        let regressed = point < self.cfg.demote_pass || recent_hf >= 2;

        ls.state = match ls.state {
            PromotionState::CloudOnly | PromotionState::ShadowJudged => {
                if bar { ls.soaked = 0; PromotionState::Canary } else { PromotionState::ShadowJudged }
            }
            PromotionState::Canary => {
                if regressed {
                    PromotionState::Demoted
                } else {
                    ls.soaked += 1;
                    if ls.soaked >= self.cfg.soak && bar {
                        PromotionState::LocalServed
                    } else {
                        PromotionState::Canary
                    }
                }
            }
            PromotionState::LocalServed => {
                if regressed { PromotionState::Demoted } else { PromotionState::LocalServed }
            }
            // Demoted goes back to SHADOW to re-earn promotion (skill stays in .cmf).
            PromotionState::Demoted => PromotionState::ShadowJudged,
        };
    }

    /// Current lifecycle state of a label.
    pub fn state(&self, label: &str) -> PromotionState {
        self.labels
            .lock()
            .unwrap()
            .get(label)
            .map(|l| l.state)
            .unwrap_or(PromotionState::CloudOnly)
    }

    /// Should a request of this label be SERVED by the local model?
    /// (Canary/LocalServed; caller still applies the per-request veto.)
    pub fn serves_local(&self, label: &str) -> bool {
        matches!(self.state(label), PromotionState::Canary | PromotionState::LocalServed)
    }

    /// Snapshot for /admin: (state, n, pass-rate, Wilson-LB) per label.
    pub fn snapshot(&self) -> Vec<(String, PromotionState, usize, f64, f64)> {
        let map = self.labels.lock().unwrap();
        let mut out: Vec<_> = map
            .iter()
            .map(|(k, ls)| {
                let n = ls.passes.len();
                let s = ls.passes.iter().filter(|&&p| p).count();
                let p = if n > 0 { s as f64 / n as f64 } else { 0.0 };
                (k.clone(), ls.state, n, p, wilson_bound(s, n, self.cfg.z, false))
            })
            .collect();
        out.sort_by(|a, b| b.2.cmp(&a.2));
        out
    }

    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }
}

fn push_cap<T>(q: &mut VecDeque<T>, v: T, cap: usize) {
    q.push_back(v);
    while q.len() > cap {
        q.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PromotionCfg {
        PromotionCfg { enabled: true, n_min: 200, soak: 0, ..Default::default() }
    }
    fn rec(label: &str, pass: bool, hard_fail: bool) -> JudgeRecord {
        JudgeRecord {
            ts: 0, task_label: label.into(), pass, hard_fail, score: if pass { 4 } else { 0 },
            source: "checker".into(), recon_e: None, born_conf: None, judge_tokens: 0,
        }
    }

    #[test]
    fn wilson_lb_penalizes_small_n() {
        // 5/5 perfect but tiny n → lower bound far below 0.95.
        assert!(wilson_bound(5, 5, 1.96, false) < 0.6);
        // 195/200 at 97.5% → lower bound clears 0.95.
        assert!(wilson_bound(195, 200, 1.96, false) >= 0.94);
        // upper bound of 0 hard-fails over 200 ≈ 0.019 (rule-of-three-ish).
        assert!(wilson_bound(0, 200, 1.96, true) < 0.025);
    }

    #[test]
    fn no_promotion_on_a_lucky_streak() {
        let p = Promotion::new(cfg());
        // 50 straight passes: point-rate 100% but n < n_min AND LB < bar.
        for _ in 0..50 {
            p.record(rec("code", true, false));
        }
        assert_eq!(p.state("code"), PromotionState::ShadowJudged);
        assert!(!p.serves_local("code"));
    }

    #[test]
    fn promotes_only_with_enough_high_evidence() {
        let p = Promotion::new(cfg());
        // 198/200 (99%) + zero hard-fails → Wilson-LB ≈ 0.964 ≥ 0.95 → Canary.
        // (195/200 = 97.5% gives LB ≈ 0.943 < 0.95 and would NOT promote —
        // the bound is deliberately strict.)
        for i in 0..200 {
            p.record(rec("code", i % 100 != 0, false)); // 2 fails / 200
        }
        assert_eq!(p.state("code"), PromotionState::Canary); // soak=0 → straight to Canary
        assert!(p.serves_local("code"));
    }

    /// Runnable narrative of the full loop:
    ///   cargo test --release demo_full_lifecycle -- --nocapture
    /// Two labels side by side — a mature skill that promotes to local
    /// serving, and a task the local can't do (stays on cloud). Proves the
    /// state machine both PROMOTES on real evidence and REFUSES safely.
    #[test]
    fn demo_full_lifecycle() {
        let p = Promotion::new(PromotionCfg { soak: 50, ..cfg() });
        let show = |label: &str| {
            let snap = p.snapshot();
            let row = snap.iter().find(|r| r.0 == label);
            match row {
                Some((_, st, n, pr, lb)) => println!(
                    "    {label:<12} {:<13} n={n:<4} pass={:.0}% LB={:.3} serves_local={}",
                    format!("{st:?}"), pr * 100.0, lb, p.serves_local(label)
                ),
                None => println!("    {label:<12} (cloud-only, no judgments)"),
            }
        };

        println!("\n── self-warming loop, live ──");
        // A mature skill (e.g. ru-tech): local answers pass the judge ~99%.
        println!("  1) SHADOW→CANARY: судим локаль 'ru_tech' (сильный скилл ~99% pass), 210 судейств:");
        for i in 0..210 {
            p.record(rec("ru_tech", i % 100 != 0, false));
        }
        show("ru_tech");
        assert_eq!(p.state("ru_tech"), PromotionState::Canary, "n≥200 & LB≥0.95 → CANARY");

        println!("  2) CANARY soak (ещё 60 судейств держат планку) → LOCAL_SERVED:");
        for i in 0..60 {
            p.record(rec("ru_tech", i % 100 != 0, false));
        }
        show("ru_tech");
        assert_eq!(p.state("ru_tech"), PromotionState::LocalServed, "soak → LOCAL_SERVED");
        assert!(p.serves_local("ru_tech"), "теперь обслуживается локально");

        // The code dataset: local fails the checker → never promotes.
        println!("  3) параллельно 'code' (0.8B не тянет генерацию кода, 0% pass):");
        for _ in 0..250 {
            p.record(rec("code", false, false));
        }
        show("code");
        assert_eq!(p.state("code"), PromotionState::ShadowJudged, "остаётся на облаке — безопасно");
        assert!(!p.serves_local("code"));

        // Drift on the promoted label → auto-demote back to cloud.
        println!("  4) деградация 'ru_tech' (дрейф) → авто-DEMOTED, назад на облако:");
        for _ in 0..120 {
            p.record(rec("ru_tech", false, false));
        }
        show("ru_tech");
        assert!(!p.serves_local("ru_tech"), "демоушен снял с локали");
        println!("  ── петля: промоушен по доказательству, отказ и откат — безопасно ──\n");
    }

    #[test]
    fn hard_fail_and_regression_block_or_demote() {
        // A hard-fail spike keeps hf upper bound high → no promotion.
        let p = Promotion::new(cfg());
        for i in 0..200 {
            p.record(rec("risky", true, i % 20 == 0)); // 5% hard_fail
        }
        assert_ne!(p.state("risky"), PromotionState::LocalServed);

        // Promote a clean label, then feed a regression → demote.
        let p2 = Promotion::new(PromotionCfg { soak: 0, ..cfg() });
        for i in 0..200 {
            p2.record(rec("t", i % 100 != 0, false)); // 198/200, 0 hard-fails
        }
        assert_eq!(p2.state("t"), PromotionState::Canary);
        for _ in 0..300 {
            p2.record(rec("t", false, false)); // quality collapses
        }
        assert!(matches!(p2.state("t"), PromotionState::Demoted | PromotionState::ShadowJudged));
    }
}
