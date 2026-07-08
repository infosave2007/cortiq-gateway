//! Local CMF model server lifecycle.
//!
//! Closes the loop with <https://github.com/infosave2007/cmf>: install the
//! `cortiq` CLI from crates.io, keep it up to date, and run `cortiq serve` as a
//! managed child process. The served model exposes an OpenAI-compatible API, so
//! the registry registers it as an ordinary provider and the gateway routes to
//! it like any other backend.

use crate::config::CmfCfg;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::{Child, Command};

/// Observable state of the managed local CMF server (surfaced to the admin API).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CmfStatus {
    /// `manage_server` is on and a `local_model` is configured.
    pub enabled: bool,
    /// Version reported by `cortiq --version` (None = not installed).
    pub installed_version: Option<String>,
    /// Newest `cortiq-cli` on crates.io (populated when `auto_update` runs).
    pub latest_version: Option<String>,
    /// The child `cortiq serve` process has been spawned.
    pub running: bool,
    /// `/healthz` on the local server answered 200.
    pub healthy: bool,
    /// OpenAI base URL the local server is registered under.
    pub base_url: String,
    /// Path of the `.cmf` being served.
    pub model: String,
    /// Last fatal error, if any.
    pub last_error: Option<String>,
    /// Recent lifecycle log lines (bounded).
    pub log: Vec<String>,
}

/// Holds the managed child process and its live status.
pub struct CmfRuntime {
    status: Mutex<CmfStatus>,
    child: Mutex<Option<Child>>,
}

impl CmfRuntime {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(CmfStatus::default()),
            child: Mutex::new(None),
        })
    }

    pub fn status(&self) -> CmfStatus {
        self.status.lock().unwrap().clone()
    }

    fn log(&self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!(target: "cmf", "{msg}");
        let mut s = self.status.lock().unwrap();
        s.log.push(msg);
        let n = s.log.len();
        if n > 50 {
            s.log.drain(0..n - 50);
        }
    }

    fn fail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::warn!(target: "cmf", "{msg}");
        let mut s = self.status.lock().unwrap();
        s.last_error = Some(msg.clone());
        s.log.push(format!("✗ {msg}"));
    }

    fn set_child(&self, child: Child) {
        *self.child.lock().unwrap() = Some(child);
        self.status.lock().unwrap().running = true;
    }

    /// Stop the managed server (used on shutdown / reconfigure).
    pub async fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
        let mut s = self.status.lock().unwrap();
        s.running = false;
        s.healthy = false;
    }
}

impl Default for CmfRuntime {
    fn default() -> Self {
        Self {
            status: Mutex::new(CmfStatus::default()),
            child: Mutex::new(None),
        }
    }
}

/// crates.io `newest_version` for `cortiq-cli`.
async fn latest_crates_version() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let body = client
        .get("https://crates.io/api/v1/crates/cortiq-cli")
        .header("User-Agent", "cortiq-gateway")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v["crate"]["newest_version"]
        .as_str()
        .map(|s| s.to_string())
}

/// `<bin> --version` → the trailing version token (e.g. "cortiq 0.1.2" → "0.1.2").
fn installed_version(bin: &str) -> Option<String> {
    let out = std::process::Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().last().map(|s| s.trim().to_string())
}

fn parse_ver(s: &str) -> (u32, u32, u32) {
    let mut it = s.split('.').map(|x| x.trim().parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn version_lt(a: &str, b: &str) -> bool {
    parse_ver(a) < parse_ver(b)
}

/// `cargo install cortiq-cli [--force]` — blocking compile, run off-thread.
async fn cargo_install(force: bool) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.args(["install", "cortiq-cli", "--locked"]);
    if force {
        cmd.arg("--force");
    }
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Spawn `cortiq serve <model> --host <host> --port <port>`; the child is killed
/// when its handle is dropped (i.e. when the gateway shuts down).
fn spawn_serve(cfg: &CmfCfg) -> Result<Child, String> {
    let mut cmd = Command::new(&cfg.cortiq_bin);
    cmd.arg("serve")
        .arg(&cfg.local_model)
        .arg("--host")
        .arg(&cfg.local_host)
        .arg("--port")
        .arg(cfg.local_port.to_string());
    // Parallelise decode matvecs across cores (CMF_THREADS). 0 = leave the
    // runtime's own default; N>1 gave ~2.8x on a 15B CPU model.
    if cfg.threads > 1 {
        cmd.env("CMF_THREADS", cfg.threads.to_string());
    }
    // Offload large matvecs to the GPU (Metal on macOS). Memory-bandwidth-bound
    // on unified memory, so mostly a prefill win; opt-in.
    if cfg.gpu {
        cmd.env("CMF_GPU", "1");
    }
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())
}

/// Poll `/healthz` until the local server answers 200 (or the deadline passes).
async fn wait_healthy(host: &str, port: u16, tries: u32) -> bool {
    let url = format!("http://{host}:{port}/healthz");
    let client = reqwest::Client::new();
    for _ in 0..tries {
        if let Ok(r) = client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Orchestrate the whole lifecycle: ensure installed → maybe update → spawn →
/// wait healthy. Safe to call in a background task; it never panics.
pub async fn manage(rt: Arc<CmfRuntime>, cfg: CmfCfg) {
    if !cfg.manage_server || cfg.local_model.trim().is_empty() {
        return;
    }
    {
        let mut s = rt.status.lock().unwrap();
        s.enabled = true;
        s.base_url = format!("http://{}:{}/v1", cfg.local_host, cfg.local_port);
        s.model = cfg.local_model.clone();
    }
    if !std::path::Path::new(&cfg.local_model).exists() {
        rt.fail(format!("local_model not found: {}", cfg.local_model));
        return;
    }

    // 1. Ensure the `cortiq` binary is available.
    let mut ver = installed_version(&cfg.cortiq_bin);
    if ver.is_none() {
        if cfg.auto_install {
            rt.log("cortiq not found — installing cortiq-cli from crates.io (this compiles, may take a few minutes)…");
            if let Err(e) = cargo_install(false).await {
                rt.fail(format!("cargo install cortiq-cli failed: {e}"));
                return;
            }
            ver = installed_version(&cfg.cortiq_bin);
            if ver.is_some() {
                rt.log("installed cortiq-cli");
            }
        } else {
            rt.fail("cortiq binary not found (set [cmf].auto_install = true, or install cortiq-cli)");
            return;
        }
    }
    rt.status.lock().unwrap().installed_version = ver.clone();

    // 2. Optionally update to the newest crates.io release.
    if cfg.auto_update {
        if let Some(latest) = latest_crates_version().await {
            rt.status.lock().unwrap().latest_version = Some(latest.clone());
            if ver.as_deref().map(|c| version_lt(c, &latest)).unwrap_or(false) {
                rt.log(format!("updating cortiq-cli {} → {latest}…", ver.as_deref().unwrap_or("?")));
                if let Err(e) = cargo_install(true).await {
                    rt.log(format!("update failed (keeping current): {e}"));
                } else {
                    ver = installed_version(&cfg.cortiq_bin);
                    rt.status.lock().unwrap().installed_version = ver.clone();
                    rt.log(format!("updated to {}", ver.as_deref().unwrap_or("?")));
                }
            }
        }
    }

    // 3. Spawn the local server.
    rt.log(format!(
        "starting: cortiq serve {} --host {} --port {}",
        cfg.local_model, cfg.local_host, cfg.local_port
    ));
    match spawn_serve(&cfg) {
        Ok(child) => rt.set_child(child),
        Err(e) => {
            rt.fail(format!("failed to spawn cortiq serve: {e}"));
            return;
        }
    }

    // 4. Wait until it is serving (model load can take a while for big models).
    if wait_healthy(&cfg.local_host, cfg.local_port, 240).await {
        rt.status.lock().unwrap().healthy = true;
        rt.log(format!(
            "local CMF server ready — registered as model '{}' at http://{}:{}/v1",
            cfg.model_id, cfg.local_host, cfg.local_port
        ));
    } else {
        rt.fail("local CMF server did not become healthy in time");
    }
}
