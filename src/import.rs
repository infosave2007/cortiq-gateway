//! CMF model factory: search HuggingFace, then convert a chosen repo to a
//! local quantized `.cmf` via the Python converter — as a tracked, streamed
//! background job so the admin UI can show live progress.

use crate::config::CmfCfg;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Notify;

/// Parse a `@PROGRESS <0..1> <phase>` marker emitted by the converter.
fn parse_progress(line: &str) -> Option<(f32, String)> {
    let rest = line.strip_prefix("@PROGRESS ")?;
    let mut it = rest.splitn(2, ' ');
    let frac: f32 = it.next()?.trim().parse().ok()?;
    let phase = it.next().unwrap_or("").trim().to_string();
    Some((frac.clamp(0.0, 1.0), phase))
}

/// Remove a partial/aborted output — `.cmf` may be a file or a tensor-dir.
fn cleanup_output(path: &str) {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Job id from a hash of (repo, nanos) — unique without a uuid dependency.
fn gen_id(repo: &str) -> String {
    use std::hash::{Hash, Hasher};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    repo.hash(&mut h);
    nanos.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Parameters chosen in the import wizard.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportParams {
    pub repo: String,
    #[serde(default = "default_quant")]
    pub quant: String,
    #[serde(default)]
    pub name: String, // output basename; empty → derived from repo
    // ── advanced ──
    #[serde(default)]
    pub linear_core: Option<String>, // gated_delta_net | vmf_phase
    #[serde(default)]
    pub nphase: Option<u32>,
    #[serde(default)]
    pub vbit_shape: Option<String>, // log2 | cubic
    #[serde(default)]
    pub mean_bits: Option<f32>,
    #[serde(default)]
    pub shard_max_gb: Option<f32>,
    #[serde(default)]
    pub skip_mtp: bool,
    /// O(1) Nyström attention hint (native converter, cortiq ≥ 0.2.0):
    /// `all` | `deepN` | `i,j,k`. Weights pass through unchanged — the runtime
    /// reads the hint at load. Empty/None/"off" = exact attention.
    #[serde(default)]
    pub o1: Option<String>,
    /// Landmark budget for the o1 hint (validated default 32).
    #[serde(default)]
    pub o1_m: Option<usize>,
    /// Exact-window width for the o1 hint (validated default 128).
    #[serde(default)]
    pub o1_window: Option<usize>,
    /// Permanent exact sink keys for the o1 hint (validated default 4).
    #[serde(default)]
    pub o1_sink: Option<usize>,
}
fn default_quant() -> String {
    "Q8_2F".into()
}

/// Live state of one conversion job.
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub repo: String,
    pub quant: String,
    pub output: String,
    pub state: String, // running | done | error | cancelled
    pub log: Vec<String>,
    pub started: u64,
    pub finished: Option<u64>,
    pub size_bytes: Option<u64>,
    pub progress: Option<f32>, // 0..1, parsed from converter @PROGRESS
    pub phase: Option<String>, // current phase label
}

#[derive(Default)]
pub struct JobStore {
    jobs: Mutex<HashMap<String, Job>>,
    /// Cancel handles, keyed by job id (not serialized). Notifying aborts the
    /// converter process and cleans up its partial output.
    cancels: Mutex<HashMap<String, Arc<Notify>>>,
}

impl JobStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn insert(&self, job: Job) {
        self.jobs.lock().unwrap().insert(job.id.clone(), job);
    }
    fn set_cancel(&self, id: &str, n: Arc<Notify>) {
        self.cancels.lock().unwrap().insert(id.to_string(), n);
    }
    fn drop_cancel(&self, id: &str) {
        self.cancels.lock().unwrap().remove(id);
    }
    /// Signal a running job to abort. Returns false if the job isn't cancellable.
    pub fn cancel(&self, id: &str) -> bool {
        let running = matches!(
            self.jobs.lock().unwrap().get(id).map(|j| j.state.as_str()),
            Some("running")
        );
        if !running {
            return false;
        }
        if let Some(n) = self.cancels.lock().unwrap().get(id) {
            n.notify_one();
            true
        } else {
            false
        }
    }
    fn push_line(&self, id: &str, line: String) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(id) {
            j.log.push(line);
            let n = j.log.len();
            if n > 200 {
                j.log.drain(0..n - 200); // keep the tail
            }
        }
    }
    fn set_progress(&self, id: &str, frac: f32, phase: String) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(id) {
            j.progress = Some(frac);
            if !phase.is_empty() {
                j.phase = Some(phase);
            }
        }
    }
    fn set_state(&self, id: &str, state: &str) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(id) {
            j.state = state.into();
            j.finished = Some(now());
        }
    }
    fn finish(&self, id: &str, ok: bool) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(id) {
            if j.state == "cancelled" {
                return; // a concurrent cancel already settled this job
            }
            j.finished = Some(now());
            j.size_bytes = std::fs::metadata(&j.output).ok().map(|m| m.len());
            let done = ok && j.size_bytes.unwrap_or(0) > 0;
            j.state = if done { "done" } else { "error" }.into();
            if done {
                j.progress = Some(1.0);
            }
        }
    }
    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }
    /// Delete a finished job together with its converted output file(s).
    /// Refuses a running job (cancel it first). `Ok(false)` if unknown.
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let job = self.jobs.lock().unwrap().get(id).cloned();
        let Some(job) = job else { return Ok(false) };
        if job.state == "running" {
            return Err("cancel the running job before deleting".into());
        }
        cleanup_output(&job.output);
        self.cancels.lock().unwrap().remove(id);
        self.jobs.lock().unwrap().remove(id);
        Ok(true)
    }
    pub fn list(&self) -> Vec<Job> {
        let mut v: Vec<Job> = self.jobs.lock().unwrap().values().cloned().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.started));
        v.truncate(20);
        v
    }
}

/// Proxy HuggingFace model search (avoids browser CORS + adds our token).
pub async fn hf_search(
    query: &str,
    limit: usize,
    token: Option<&str>,
) -> Result<serde_json::Value, String> {
    let q: String = url_escape(query);
    let sort = if query.trim().is_empty() {
        "trendingScore"
    } else {
        "downloads"
    };
    let url = format!(
        "https://huggingface.co/api/models?search={q}&sort={sort}&direction=-1&limit={limit}&full=false"
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&url).header("User-Agent", "cortiq-gateway");
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HuggingFace API {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

fn url_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '/' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Kick off a conversion. Returns the job id immediately; progress streams
/// Map the UI quant label to the native `cortiq` quant flag.
fn map_quant(q: &str) -> &'static str {
    match q.to_ascii_uppercase().as_str() {
        s if s.starts_with("Q4T") || s == "Q4_TILED" => "q4t",
        s if s.starts_with("Q4") => "q4",
        "Q8_2F" | "Q82F" => "q8_2f",
        "F16" | "FP16" => "f16",
        "VBIT" => "vbit",
        "Q1" => "q1",
        "Q1P" | "Q1_PTQ" => "q1p",
        "Q1S" | "Q1_MASK" => "q1s",
        "Q1T" | "Q1_TERNARY" => "q1t",
        _ => "q8",
    }
}

fn is_cmf_repo(repo: &str) -> bool {
    let lower = repo.to_ascii_lowercase();
    lower.contains("cmf") || lower.ends_with(".cmf")
}

async fn download_cmf_repo(
    repo: String,
    output_abs: String,
    hf_token: Option<String>,
    store: Arc<JobStore>,
    id: String,
    cancel: Arc<Notify>,
) {
    store.push_line(&id, format!("→ downloading ready .cmf model from {}", repo));
    store.set_progress(&id, 0.0, "listing files".into());

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            store.push_line(&id, format!("✗ failed to build HTTP client: {e}"));
            store.finish(&id, false);
            store.drop_cancel(&id);
            return;
        }
    };

    let tree_url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let mut req = client.get(&tree_url).header("User-Agent", "cortiq-gateway");
    if let Some(t) = &hf_token {
        req = req.bearer_auth(t);
    }

    let mut cmf_rel_path = None;
    if let Ok(resp) = req.send().await {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = json.as_array() {
                    for item in arr {
                        if let Some(path) = item.get("path").and_then(|p| p.as_str()) {
                            if path.to_ascii_lowercase().ends_with(".cmf") {
                                cmf_rel_path = Some(path.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    let cmf_path = match cmf_rel_path {
        Some(p) => p,
        None => {
            let repo_name = repo.split('/').last().unwrap_or(&repo);
            format!("{repo_name}.cmf")
        }
    };

    store.push_line(&id, format!("→ downloading {cmf_path} from HuggingFace..."));
    store.set_progress(&id, 0.05, "downloading".into());

    let download_url = format!("https://huggingface.co/{repo}/resolve/main/{cmf_path}");
    let mut req = client
        .get(&download_url)
        .header("User-Agent", "cortiq-gateway");
    if let Some(t) = &hf_token {
        req = req.bearer_auth(t);
    }

    let mut resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            store.push_line(&id, format!("✗ HF download error: HTTP {}", r.status()));
            store.finish(&id, false);
            store.drop_cancel(&id);
            return;
        }
        Err(e) => {
            store.push_line(&id, format!("✗ network error: {e}"));
            store.finish(&id, false);
            store.drop_cancel(&id);
            return;
        }
    };

    let total_bytes = resp.content_length();
    let mut file = match tokio::fs::File::create(&output_abs).await {
        Ok(f) => f,
        Err(e) => {
            store.push_line(&id, format!("✗ failed to create file {output_abs}: {e}"));
            store.finish(&id, false);
            store.drop_cancel(&id);
            return;
        }
    };

    use tokio::io::AsyncWriteExt;
    let mut downloaded: u64 = 0;

    loop {
        tokio::select! {
            _ = cancel.notified() => {
                let _ = tokio::fs::remove_file(&output_abs).await;
                store.push_line(&id, "✗ cancelled by user — partial output removed".into());
                store.set_state(&id, "cancelled");
                store.drop_cancel(&id);
                return;
            }
            chunk_opt = resp.chunk() => {
                match chunk_opt {
                    Ok(Some(chunk)) => {
                        if let Err(e) = file.write_all(&chunk).await {
                            store.push_line(&id, format!("✗ write error: {e}"));
                            cleanup_output(&output_abs);
                            store.finish(&id, false);
                            store.drop_cancel(&id);
                            return;
                        }
                        downloaded += chunk.len() as u64;
                        if let Some(total) = total_bytes {
                            if total > 0 {
                                let frac = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
                                store.set_progress(&id, frac, "downloading".into());
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        store.push_line(&id, format!("✗ download stream error: {e}"));
                        cleanup_output(&output_abs);
                        store.finish(&id, false);
                        store.drop_cancel(&id);
                        return;
                    }
                }
            }
        }
    }

    if let Err(e) = file.flush().await {
        store.push_line(&id, format!("✗ flush error: {e}"));
        cleanup_output(&output_abs);
        store.finish(&id, false);
        store.drop_cancel(&id);
        return;
    }

    store.push_line(&id, "✓ done".into());
    store.finish(&id, true);
    store.drop_cancel(&id);
}

/// into the job log. Fails fast if the converter script is missing.
pub fn start_import(store: Arc<JobStore>, cfg: &CmfCfg, p: ImportParams) -> Result<String, String> {
    if p.repo.trim().is_empty() {
        return Err("empty repo id".into());
    }
    // Native `cortiq convert` (from crates.io, no Python) handles standard + MoE
    // models; the bundled Python converter is used only for advanced options it
    // doesn't support yet (linear-attention folding, v-bit shaping).
    let use_python = p.linear_core.is_some() || p.vbit_shape.is_some() || p.mean_bits.is_some();
    if use_python && !std::path::Path::new(&cfg.converter).exists() {
        return Err(format!(
            "advanced options need the Python converter, not found: {}",
            cfg.converter
        ));
    }
    // O(1) attention hint — native-converter feature (cortiq ≥ 0.2.0). Fail fast
    // with a clear message instead of a mid-job "unexpected argument" error.
    let o1 =
        p.o1.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "off")
            .map(str::to_string);
    if o1.is_some() {
        if use_python {
            return Err(
                "O(1) attention is supported by the native converter only — remove the \
                 linear-core / v-bit shape / mean-bits options to use it"
                    .into(),
            );
        }
        if p.repo.to_ascii_lowercase().contains("gguf") {
            return Err("O(1) attention is not supported for GGUF imports yet".into());
        }
        let ver = crate::cmf_runtime::installed_version(&cfg.cortiq_bin);
        if let Some(v) = &ver {
            if crate::cmf_runtime::version_lt(v, "0.2.0") {
                return Err(format!(
                    "O(1) attention needs cortiq ≥ 0.2.0 (installed: {v}) — update the runtime \
                     in Settings → Local models"
                ));
            }
        }
    }
    let _ = std::fs::create_dir_all(&cfg.models_dir);
    let base = if p.name.trim().is_empty() {
        sanitize(p.repo.rsplit('/').next().unwrap_or(&p.repo))
    } else {
        sanitize(&p.name)
    };
    let output = std::path::Path::new(&cfg.models_dir)
        .join(format!("{base}.cmf"))
        .to_string_lossy()
        .to_string();
    let output_abs = std::fs::canonicalize(&cfg.models_dir)
        .map(|d| d.join(format!("{base}.cmf")).to_string_lossy().to_string())
        .unwrap_or_else(|_| output.clone());

    let id = gen_id(&p.repo);
    store.insert(Job {
        id: id.clone(),
        repo: p.repo.clone(),
        quant: p.quant.clone(),
        output: output_abs.clone(),
        state: "running".into(),
        log: vec![format!("→ converting {} to {} ({})", p.repo, base, p.quant)],
        started: now(),
        finished: None,
        size_bytes: None,
        progress: Some(0.0),
        phase: Some("starting".into()),
    });
    let cancel = Arc::new(Notify::new());
    store.set_cancel(&id, cancel.clone());

    let hf_token = if cfg.hf_token_env.is_empty() {
        None
    } else {
        std::env::var(&cfg.hf_token_env).ok()
    };

    if is_cmf_repo(&p.repo) && !use_python {
        let ret_id = id.clone();
        tokio::spawn(async move {
            download_cmf_repo(p.repo, output_abs, hf_token, store, id, cancel).await;
        });
        return Ok(ret_id);
    }

    // Program + args: `cortiq convert` by default, or the Python converter for
    // advanced options. Both stream `@PROGRESS <frac>` markers into the log.
    let (program, args, workdir): (String, Vec<String>, std::path::PathBuf) = if use_python {
        let conv = std::path::Path::new(&cfg.converter);
        let mut a = vec![
            std::fs::canonicalize(conv)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| cfg.converter.clone()),
            "--model".into(),
            p.repo.clone(),
            "--quant".into(),
            p.quant.clone(),
            "--output".into(),
            output_abs.clone(),
        ];
        if let Some(lc) = &p.linear_core {
            a.push("--linear-core".into());
            a.push(lc.clone());
        }
        if let Some(n) = p.nphase {
            a.push("--nphase".into());
            a.push(n.to_string());
        }
        if let Some(vs) = &p.vbit_shape {
            a.push("--vbit-shape".into());
            a.push(vs.clone());
        }
        if let Some(mb) = p.mean_bits {
            a.push("--mean-bits".into());
            a.push(mb.to_string());
        }
        if let Some(g) = p.shard_max_gb {
            a.push("--shard-max-gb".into());
            a.push(g.to_string());
        }
        if p.skip_mtp {
            a.push("--skip-mtp".into());
        }
        let wd = conv
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        (cfg.python_bin.clone(), a, wd)
    } else if p.repo.to_ascii_lowercase().contains("gguf") {
        // GGUF repo → native `cortiq import-gguf` (downloads + dequantizes any
        // common ggml quant type: Q4_0/1, Q5_0/1, Q8_0, Q2_K..Q6_K, IQ4_NL/XS).
        let mut a = vec![
            "import-gguf".into(),
            p.repo.clone(),
            "--quant".into(),
            map_quant(&p.quant).into(),
            "--output".into(),
            output_abs.clone(),
        ];
        if let Some(t) = &hf_token {
            a.push("--hf-token".into());
            a.push(t.clone());
        }
        (
            cfg.cortiq_bin.clone(),
            a,
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        )
    } else {
        // safetensors → native `cortiq convert` (dense / MoE / GatedDeltaNet).
        let mut a = vec![
            "convert".into(),
            "--model".into(),
            p.repo.clone(),
            "--quant".into(),
            map_quant(&p.quant).into(),
            "--output".into(),
            output_abs.clone(),
        ];
        if let Some(t) = &hf_token {
            a.push("--hf-token".into());
            a.push(t.clone());
        }
        // O(1) attention hint (weights unchanged; the runtime reads it at load)
        if let Some(spec) = &o1 {
            a.push("--o1".into());
            a.push(spec.clone());
            if let Some(m) = p.o1_m {
                a.push("--o1-m".into());
                a.push(m.to_string());
            }
            if let Some(w) = p.o1_window {
                a.push("--o1-window".into());
                a.push(w.to_string());
            }
            if let Some(s) = p.o1_sink {
                a.push("--o1-sink".into());
                a.push(s.to_string());
            }
        }
        (
            cfg.cortiq_bin.clone(),
            a,
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        )
    };

    let ret_id = id.clone();
    let output_for_task = output_abs.clone();
    tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&args)
            .current_dir(&workdir)
            .env("PYTHONUNBUFFERED", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // HF token for the Python converter (native cortiq gets --hf-token).
        if use_python {
            if let Some(tok) = hf_token {
                cmd.env("HF_TOKEN", &tok).env("HUGGING_FACE_HUB_TOKEN", tok);
            }
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                store.push_line(&id, format!("✗ spawn failed: {e}"));
                store.finish(&id, false);
                store.drop_cancel(&id);
                return;
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        // stdout: parse @PROGRESS markers into progress/phase; everything else logs.
        let s1 = store.clone();
        let id1 = id.clone();
        let t_out = tokio::spawn(async move {
            if let Some(o) = stdout {
                let mut lines = BufReader::new(o).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if let Some((frac, phase)) = parse_progress(&l) {
                        s1.set_progress(&id1, frac, phase);
                    } else {
                        s1.push_line(&id1, l);
                    }
                }
            }
        });
        let s2 = store.clone();
        let id2 = id.clone();
        let t_err = tokio::spawn(async move {
            if let Some(e) = stderr {
                let mut lines = BufReader::new(e).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    s2.push_line(&id2, l);
                }
            }
        });

        tokio::select! {
            status = child.wait() => {
                let _ = tokio::join!(t_out, t_err);
                let ok = status.map(|s| s.success()).unwrap_or(false);
                if !ok {
                    cleanup_output(&output_for_task); // drop partial/corrupt output
                }
                store.push_line(
                    &id,
                    if ok { "✓ done".into() } else { "✗ converter exited with error".into() },
                );
                store.finish(&id, ok);
            }
            _ = cancel.notified() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                t_out.abort();
                t_err.abort();
                cleanup_output(&output_for_task);
                store.push_line(&id, "✗ cancelled by user — partial output removed".into());
                store.set_state(&id, "cancelled");
            }
        }
        store.drop_cancel(&id);
    });

    Ok(ret_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_quant_variants() {
        assert_eq!(map_quant("Q8_2F"), "q8_2f");
        assert_eq!(map_quant("q82f"), "q8_2f");
        assert_eq!(map_quant("Q8_ROW"), "q8");
        assert_eq!(map_quant("Q4_BLOCK"), "q4");
        assert_eq!(map_quant("Q4_TILED"), "q4t");
        assert_eq!(map_quant("q4t"), "q4t");
        assert_eq!(map_quant("vbit"), "vbit");
        assert_eq!(map_quant("q1"), "q1");
        assert_eq!(map_quant("Q1P"), "q1p");
        assert_eq!(map_quant("q1s"), "q1s");
        assert_eq!(map_quant("Q1T"), "q1t");
        assert_eq!(map_quant("F16"), "f16");
    }

    #[test]
    fn test_is_cmf_repo_check() {
        assert!(is_cmf_repo("infosave/Bonsai-8B_2bit_cmf"));
        assert!(is_cmf_repo("infosave/Bonsai-1.7Bcmf"));
        assert!(is_cmf_repo("user/model.cmf"));
        assert!(!is_cmf_repo("Qwen/Qwen2.5-0.5B-Instruct"));
    }
}
