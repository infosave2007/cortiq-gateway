//! Local CMF model server lifecycle.
//!
//! Closes the loop with <https://github.com/infosave2007/cmf>: install the
//! `cortiq` CLI from crates.io, keep it up to date, and run one `cortiq serve`
//! per configured local model as a managed child process. Each served model
//! exposes an OpenAI-compatible API, so the registry registers it as an ordinary
//! provider and the gateway routes to it like any other backend.

use crate::config::{CmfCfg, CmfServer};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::{Child, Command};

/// Observable state of one managed local model server.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CmfServerStatus {
    /// Pool id it is registered under.
    pub id: String,
    /// Path of the `.cmf` being served.
    pub model: String,
    /// Port the server binds to.
    pub port: u16,
    /// OpenAI base URL the local server is registered under.
    pub base_url: String,
    /// The child `cortiq serve` process has been spawned.
    pub running: bool,
    /// `/healthz` on the local server answered 200.
    pub healthy: bool,
    /// Last fatal error for this server, if any.
    pub last_error: Option<String>,
}

/// Observable state of the managed local CMF layer (surfaced to the admin API).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct CmfStatus {
    /// `manage_server` is on and at least one local model is configured.
    pub enabled: bool,
    /// Version reported by `cortiq --version` (None = not installed).
    pub installed_version: Option<String>,
    /// Newest `cortiq-cli` on crates.io (populated when `auto_update` runs).
    pub latest_version: Option<String>,
    /// Per-model server status.
    pub servers: Vec<CmfServerStatus>,
    /// Recent lifecycle log lines (bounded).
    pub log: Vec<String>,
}

/// A spawned server: its pool id and the child process handle.
struct Managed {
    id: String,
    child: Child,
}

/// Holds the managed child processes and their live status.
pub struct CmfRuntime {
    status: Mutex<CmfStatus>,
    children: Mutex<Vec<Managed>>,
}

impl CmfRuntime {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(CmfStatus::default()),
            children: Mutex::new(Vec::new()),
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
        if n > 80 {
            s.log.drain(0..n - 80);
        }
    }

    /// A global (install/update) failure — not tied to a single server.
    fn fail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::warn!(target: "cmf", "{msg}");
        self.status.lock().unwrap().log.push(format!("✗ {msg}"));
    }

    /// Record a fatal error against a specific server (and the log).
    fn fail_server(&self, id: &str, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::warn!(target: "cmf", "[{id}] {msg}");
        let mut s = self.status.lock().unwrap();
        if let Some(sv) = s.servers.iter_mut().find(|x| x.id == id) {
            sv.last_error = Some(msg.clone());
        }
        s.log.push(format!("✗ [{id}] {msg}"));
    }

    /// Seed the per-server status slots from the effective server list.
    fn init_servers(&self, servers: &[CmfServer], host: &str) {
        let mut s = self.status.lock().unwrap();
        s.enabled = true;
        s.servers = servers
            .iter()
            .map(|sv| CmfServerStatus {
                id: sv.id.clone(),
                model: sv.model.clone(),
                port: sv.port,
                base_url: format!("http://{host}:{}/v1", sv.port),
                running: false,
                healthy: false,
                last_error: None,
            })
            .collect();
    }

    fn set_running(&self, id: &str, running: bool) {
        if let Some(sv) = self
            .status
            .lock()
            .unwrap()
            .servers
            .iter_mut()
            .find(|x| x.id == id)
        {
            sv.running = running;
        }
    }

    fn set_healthy(&self, id: &str, healthy: bool) {
        if let Some(sv) = self
            .status
            .lock()
            .unwrap()
            .servers
            .iter_mut()
            .find(|x| x.id == id)
        {
            sv.healthy = healthy;
        }
    }

    fn add_child(&self, id: String, child: Child) {
        self.children.lock().unwrap().push(Managed {
            id: id.clone(),
            child,
        });
        self.set_running(&id, true);
    }

    fn is_running(&self, id: &str) -> bool {
        self.status
            .lock()
            .unwrap()
            .servers
            .iter()
            .any(|s| s.id == id && s.running)
    }

    /// Stop all managed servers (used on shutdown / reconfigure).
    pub async fn stop(&self) {
        let mut kids = std::mem::take(&mut *self.children.lock().unwrap());
        for m in kids.iter_mut() {
            let _ = m.child.start_kill();
        }
        let mut s = self.status.lock().unwrap();
        for sv in s.servers.iter_mut() {
            sv.running = false;
            sv.healthy = false;
        }
    }

    /// Stop and forget a single managed server (used when a model is deleted).
    pub async fn stop_one(&self, id: &str) {
        let removed = {
            let mut kids = self.children.lock().unwrap();
            kids.iter()
                .position(|m| m.id == id)
                .map(|pos| kids.remove(pos))
        };
        if let Some(mut m) = removed {
            let _ = m.child.start_kill();
        }
        self.status.lock().unwrap().servers.retain(|sv| sv.id != id);
    }
}

impl Default for CmfRuntime {
    fn default() -> Self {
        Self {
            status: Mutex::new(CmfStatus::default()),
            children: Mutex::new(Vec::new()),
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
    v["crate"]["newest_version"].as_str().map(|s| s.to_string())
}

/// `<bin> --version` → the trailing version token (e.g. "cortiq 0.1.2" → "0.1.2").
pub(crate) fn installed_version(bin: &str) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace().last().map(|s| s.trim().to_string())
}

fn parse_ver(s: &str) -> (u32, u32, u32) {
    let mut it = s.split('.').map(|x| x.trim().parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

pub(crate) fn version_lt(a: &str, b: &str) -> bool {
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

/// Install/upgrade `cortiq-cli` from crates.io on demand (the admin "Install
/// cortiq" button). Runs in the background; progress + result land in the CMF
/// status log, and `installed_version` is refreshed on success.
pub async fn install_now(rt: Arc<CmfRuntime>, bin: String) {
    rt.log("installing cortiq-cli from crates.io (this compiles — a few minutes)…");
    match cargo_install(true).await {
        Ok(()) => {
            let v = installed_version(&bin);
            rt.status.lock().unwrap().installed_version = v.clone();
            rt.log(format!(
                "installed cortiq-cli {}",
                v.as_deref().unwrap_or("?")
            ));
        }
        Err(e) => rt.fail(format!("cargo install cortiq-cli failed: {e}")),
    }
}

/// Spawn `cortiq serve <model> --host <host> --port <port>` for one server; the
/// child is killed when its handle is dropped (i.e. when the gateway shuts down).
fn spawn_serve(server: &CmfServer, cfg: &CmfCfg) -> Result<Child, String> {
    let mut cmd = Command::new(&cfg.cortiq_bin);
    cmd.arg("serve")
        .arg(&server.model)
        .arg("--host")
        .arg(&cfg.local_host)
        .arg("--port")
        .arg(server.port.to_string());
    // Parallelise decode matvecs across cores (CMF_THREADS). 0/1 = leave the
    // runtime's own default; N>1 gave ~2.8x on a 15B CPU model.
    if server.threads > 1 {
        cmd.env("CMF_THREADS", server.threads.to_string());
    }
    if server.gpu {
        cmd.env("CMF_GPU", "1");
    }
    // O(1) Nyström attention: layers spec + sub-parameters.
    if let Some(o1) = &server.o1 {
        let s = o1.trim();
        if !s.is_empty() {
            // Pass even "off" so it explicitly overrides a file converter hint.
            cmd.arg("--o1").arg(s);
        }
    }
    if let Some(m) = server.o1_m {
        cmd.arg("--o1-m").arg(m.to_string());
    }
    if let Some(w) = server.o1_window {
        cmd.arg("--o1-window").arg(w.to_string());
    }
    if let Some(sink) = server.o1_sink {
        cmd.arg("--o1-sink").arg(sink.to_string());
    }
    // MTP (Multi-Token Prediction) speculative decoding: the engine reads
    // CMF_MTP env; "0" disables.  There is no CLI flag for this.
    if server.skip_mtp {
        cmd.env("CMF_MTP", "0");
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

/// Orchestrate the whole lifecycle: ensure installed → maybe update → spawn each
/// configured local model → wait healthy. Safe to call in a background task; it
/// never panics.
pub async fn manage(rt: Arc<CmfRuntime>, cfg: CmfCfg) {
    let servers = cfg.effective_servers();
    if servers.is_empty() {
        return;
    }
    rt.init_servers(&servers, &cfg.local_host);

    // 1. Ensure the `cortiq` binary is available (once for all servers).
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
            rt.fail(
                "cortiq binary not found (set [cmf].auto_install = true, or install cortiq-cli)",
            );
            return;
        }
    }
    rt.status.lock().unwrap().installed_version = ver.clone();

    // 2. Optionally update to the newest crates.io release.
    if cfg.auto_update {
        if let Some(latest) = latest_crates_version().await {
            rt.status.lock().unwrap().latest_version = Some(latest.clone());
            if ver
                .as_deref()
                .map(|c| version_lt(c, &latest))
                .unwrap_or(false)
            {
                rt.log(format!(
                    "updating cortiq-cli {} → {latest}…",
                    ver.as_deref().unwrap_or("?")
                ));
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

    // 3. Spawn each server (fast).
    for server in &servers {
        if !std::path::Path::new(&server.model).exists() {
            rt.fail_server(
                &server.id,
                format!("model file not found: {}", server.model),
            );
            continue;
        }
        rt.log(format!(
            "starting: cortiq serve {} --host {} --port {}",
            server.model, cfg.local_host, server.port
        ));
        match spawn_serve(server, &cfg) {
            Ok(child) => rt.add_child(server.id.clone(), child),
            Err(e) => rt.fail_server(&server.id, format!("failed to spawn: {e}")),
        }
    }

    // 4. Wait until each spawned server is serving (model load can be slow).
    for server in &servers {
        if !rt.is_running(&server.id) {
            continue; // failed to spawn / file missing
        }
        if wait_healthy(&cfg.local_host, server.port, 240).await {
            rt.set_healthy(&server.id, true);
            rt.log(format!(
                "local CMF server ready — '{}' at http://{}:{}/v1",
                server.id, cfg.local_host, server.port
            ));
        } else {
            rt.fail_server(&server.id, "did not become healthy in time");
        }
    }
}
