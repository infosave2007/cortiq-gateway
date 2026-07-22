// Settings — router/route/breaker/log/telemetry/stats + export/import config.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState, SITE_URL } from "../app.js";

// A small "?" badge with a hover/focus tooltip explaining a setting.
function help(text) {
  return text ? h("span", { class: "help", title: text, tabindex: "0", role: "img", "aria-label": text }, "?") : null;
}
function field(label, control, hint, helpText) {
  return h(
    "label",
    { class: "field" },
    h("span", {}, label, helpText ? " " : null, help(helpText)),
    control,
    hint ? h("div", { class: "hint" }, hint) : null
  );
}
function check(label, checked, helpText) {
  const cb = h("input", { type: "checkbox", checked: checked ? true : null });
  return { node: h("label", { class: "check" }, cb, label, helpText ? " " : null, help(helpText)), cb };
}

// Flatten router usage (nested account/usage objects) into readable, wrapping
// key: value rows — a raw JSON.stringify dump overflows the card horizontally.
function usageRows(obj, prefix) {
  const rows = [];
  for (const [k, v] of Object.entries(obj || {})) {
    const key = prefix ? prefix + "." + k : k;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      rows.push(...usageRows(v, key));
    } else {
      rows.push(
        h("div", { class: "mono kv" },
          h("span", { class: "kv-k" }, key + ": "),
          h("span", { class: "kv-v" }, Array.isArray(v) ? JSON.stringify(v) : String(v)))
      );
    }
  }
  return rows;
}

// Probe status → i18n key suffix (see /admin/api/router/probe).
function probeMsgKey(status) {
  const map = {
    ok: "ok",
    no_key: "noKey",
    auth: "auth",
    payment: "payment",
    quota: "quota",
    timeout: "timeout",
    unreachable: "unreachable",
  };
  return "router.probe." + (map[status] || "error");
}

// Statuses where the fix is on the site (get / pay for a key).
const PROBE_SITE_STATUSES = ["no_key", "auth", "payment", "quota"];

export async function renderSettings() {
  const meta = appState.meta || (await api.meta());
  const [s, secretsData] = await Promise.all([api.getSettings(), api.listSecrets()]);
  const root = h("div");

  const listenIn = h("input", { value: s.listen || "" });
  // router
  const rUrl = h("input", { value: s.router?.url || "" });
  const rKeyEnv = h("input", { value: s.router?.api_key_env || "", placeholder: "CORTIQ_ROUTER_KEY" });
  const rTimeout = h("input", { type: "number", value: s.router?.timeout_ms ?? 800 });
  const rTax = h("input", { value: s.router?.taxonomy_id || "" });
  const rVerify = check(t("settings.router.verifyTls"), s.router?.verify_tls, t("settings.router.verifyTlsHelp"));
  const rEnabled = check(t("settings.router.enabled"), s.router?.enabled !== false, t("settings.router.enabledHelp"));
  // router key value (write-only; stored in the secret store, like model keys)
  const rKeySource =
    (secretsData.secrets || []).find((x) => x.name === (s.router?.api_key_env || "CORTIQ_ROUTER_KEY"))?.source ||
    "missing";
  const keyStored = rKeySource === "store" || rKeySource === "env";
  // When a key is already saved, show a masked "saved" hint instead of an empty
  // field (an empty field reads as "nothing is set").
  const rSecret = h("input", {
    type: "password",
    placeholder: keyStored ? t("settings.router.keyStoredMask") : t("models.form.secretPlaceholder"),
  });
  const rKeyBadge = h(
    "span",
    { class: "badge " + (rKeySource === "store" ? "store" : rKeySource === "env" ? "env" : "bad") },
    rKeySource === "store"
      ? t("models.form.secretStored")
      : rKeySource === "env"
        ? t("models.form.secretEnv")
        : t("models.form.secretMissing")
  );
  const rSecretHint = h(
    "div",
    {},
    h(
      "div",
      {},
      rKeyBadge,
      " · ",
      t("settings.router.getKey") + " ",
      h("a", { href: SITE_URL, target: "_blank", rel: "noopener" }, "api.allaigate.com")
    ),
    h("div", { class: "muted", style: "margin-top:4px" }, t("settings.router.secretHelp"))
  );
  // The key lives in the secret store under this env name (config default: CORTIQ_ROUTER_KEY).
  const keyName = () => rKeyEnv.value.trim() || "CORTIQ_ROUTER_KEY";
  // Persist just the pasted router key (independent of the big Save at page top).
  async function saveKey() {
    const v = rSecret.value.trim();
    if (!v) return false;
    if (!rKeyEnv.value.trim()) rKeyEnv.value = "CORTIQ_ROUTER_KEY";
    await api.setSecret(keyName(), v);
    rSecret.value = "";
    rSecret.placeholder = t("settings.router.keyStoredMask"); // show "saved", not empty
    rKeyBadge.textContent = t("models.form.secretStored");
    rKeyBadge.className = "badge store";
    return true;
  }
  // Shared connection test (real /v1/route call) — tells key problems apart from
  // a down router. Rendered into probeOut; also used right after saving the key.
  const probeOut = h("div", { class: "hint", style: "margin-top:8px" });
  async function runProbe() {
    mount(probeOut, h("span", { class: "spinner" }));
    try {
      const r = await api.probeRouter();
      const parts = [t(probeMsgKey(r.status), { ms: r.latency_ms ?? "—" })];
      if (r.message) parts.push(" — " + r.message);
      const out = [h("span", {}, ...parts)];
      if (PROBE_SITE_STATUSES.includes(r.status)) {
        out.push(" · ", h("a", { href: SITE_URL, target: "_blank", rel: "noopener" }, t("dash.health.getKey") + " ↗"));
      }
      if (r.usage && typeof r.usage === "object") {
        out.push(
          h("div", { class: "usage-box", style: "margin-top:4px" },
            h("b", {}, t("router.usage")), ...usageRows(r.usage))
        );
      }
      mount(probeOut, h("div", {}, h("span", { class: "badge " + (r.ok ? "ok" : "bad") }, r.ok ? "OK" : "×"), " ", ...out));
      toast(t(probeMsgKey(r.status), { ms: r.latency_ms ?? "—" }), r.ok ? "good" : "bad");
      return r.ok;
    } catch (e) {
      mount(probeOut, String(e.message || e));
      return false;
    }
  }
  const saveKeyBtn = h("button", { class: "btn sm" }, "✓ " + t("settings.router.saveKey"));
  saveKeyBtn.addEventListener("click", async () => {
    saveKeyBtn.disabled = true;
    try {
      if (await saveKey()) {
        toast(t("settings.router.keySaved"), "good");
        // Immediately test the connection so the user sees reachable/error at once,
        // instead of a stale "unavailable" until something re-checks.
        if (rEnabled.cb.checked) await runProbe();
      } else {
        toast(t("settings.router.keyEmpty"), "warn");
      }
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    saveKeyBtn.disabled = false;
  });
  const probeBtn = h("button", { class: "btn" }, t("settings.router.test"));
  probeBtn.addEventListener("click", async () => {
    probeBtn.disabled = true;
    // If a key was pasted but not saved, persist it first so the test uses it.
    if (rSecret.value.trim()) {
      try { await saveKey(); } catch (_) { /* probe will report no_key/auth */ }
    }
    await runProbe();
    probeBtn.disabled = false;
  });
  // route
  const strategySel = h("select", {}, ...(meta.text_strategies || []).map((x) => h("option", { value: x, selected: x === s.route?.text_strategy }, x)));
  const maxChars = h("input", { type: "number", value: s.route?.max_chars ?? 4000 });
  const cacheTtl = h("input", { value: s.route?.cache_ttl || "60s" });
  const profileSel = h("select", {}, ...(meta.profiles || []).map((x) => h("option", { value: x, selected: x === s.route?.profile }, x)));
  // breaker
  const brThresh = h("input", { type: "number", value: s.breaker?.threshold ?? 5 });
  const brCool = h("input", { value: s.breaker?.cooldown || "30s" });
  // log
  const logSel = h("select", {}, ...["error", "warn", "info", "debug", "trace"].map((x) => h("option", { value: x, selected: x === s.log?.level }, x)));
  const logBodies = check(t("settings.log.bodies"), s.log?.bodies, t("settings.log.bodiesHelp"));
  // telemetry / cortiq
  const metricsCb = check(t("settings.telemetry.metrics"), s.telemetry?.metrics, t("settings.telemetry.metricsHelp"));
  const echoCb = check(t("settings.cortiq.echo"), s.cortiq?.echo, t("settings.cortiq.echoHelp"));
  // stats
  const statsEnabled = check(t("settings.stats.enabled"), s.stats?.enabled, t("settings.stats.enabledHelp"));
  const statsFile = h("input", { value: s.stats?.file || "" });
  const statsRet = h("input", { value: s.stats?.retention || "7d" });
  // cache
  const modelsData = await api.listModels();
  const embedModels = (modelsData.models || []).filter((m) => m.kind === "embedding").map((m) => m.id);
  const cacheEnabled = check(t("settings.cache.enabled"), s.cache?.enabled, t("settings.cache.enabledHelp"));
  const cacheThresh = h("input", { type: "number", step: "0.01", value: s.cache?.threshold ?? 0.92 });
  const cacheTtlIn = h("input", { value: s.cache?.ttl || "1h" });
  const cacheEmbedSel = h(
    "select",
    {},
    h("option", { value: "" }, "auto"),
    ...embedModels.map((id) => h("option", { value: id, selected: id === s.cache?.embed_model }, id))
  );
  // local models (CMF) — run a local .cmf server and/or force local-only routing
  const cmfLocalOnly = check(t("settings.cmf.localOnly"), s.cmf?.local_only, t("settings.cmf.localOnlyHelp"));
  const cmfManage = check(t("settings.cmf.manage"), s.cmf?.manage_server, t("settings.cmf.manageHelp"));
  const cmfAutoInstall = check(t("settings.cmf.autoInstall"), s.cmf?.auto_install, t("settings.cmf.autoInstallHelp"));
  const cmfAutoUpdate = check(t("settings.cmf.autoUpdate"), s.cmf?.auto_update, t("settings.cmf.autoUpdateHelp"));
  // ── managed local models: one supervised `cortiq serve` per row ──
  const serverRows = [];
  const serversWrap = h("div", {});
  const legacyServers = s.cmf?.local_model
    ? [{ id: s.cmf.model_id || "cmf-local", model: s.cmf.local_model, port: s.cmf.local_port ?? 8081, threads: s.cmf.threads ?? 8, gpu: !!s.cmf.gpu }]
    : [];
  const initServers = (s.cmf?.servers && s.cmf.servers.length) ? s.cmf.servers : legacyServers;
  // available .cmf files under models_dir → offer as a dropdown, not a typed path
  const cmfFiles = ((await api.cmfFiles().catch(() => ({ files: [] }))).files) || [];
  const fileLabel = (f) => f.name + (f.size ? ` (${(f.size / 1073741824).toFixed(f.size >= 1073741824 ? 1 : 2)} GB)` : f.is_dir ? " (dir)" : "");
  function autoId(path) {
    const base = (path || "").split("/").pop().replace(/\.cmf$/i, "");
    return base ? "cmf-" + base.replace(/[^a-zA-Z0-9_-]/g, "-") : "";
  }
  function makeServerRow(sv) {
    sv = sv || { id: "", model: "", port: 8090, threads: 8, gpu: false };
    const idIn = h("input", { value: sv.id || "", placeholder: "cmf-id", style: "width:130px" });
    // pick from existing .cmf files rather than typing a path
    const modelSel = h("select", { style: "flex:2;min-width:150px" });
    const seen = new Set();
    modelSel.appendChild(h("option", { value: "" }, "— " + t("settings.cmf.pick") + " —"));
    cmfFiles.forEach((f) => { modelSel.appendChild(h("option", { value: f.path }, fileLabel(f))); seen.add(f.path); });
    if (sv.model && !seen.has(sv.model)) modelSel.appendChild(h("option", { value: sv.model }, sv.model));
    modelSel.value = sv.model || "";
    const portIn = h("input", { type: "number", value: sv.port ?? 8090, style: "width:82px" });
    const threadsIn = h("input", { type: "number", min: 0, value: sv.threads ?? 8, style: "width:60px", title: t("settings.cmf.threadsHelp") });
    const gpuIn = h("input", { type: "checkbox", checked: sv.gpu ? true : null, title: t("settings.cmf.gpuHelp") });
    const checkBtn = h("button", { class: "btn small", type: "button" }, t("settings.cmf.checkPort"));
    const removeBtn = h("button", { class: "btn small danger", type: "button", title: t("common.delete") }, "✕");
    const st = h("span", { class: "small" });
    modelSel.addEventListener("change", () => { if (!idIn.value || idIn._auto) { idIn.value = autoId(modelSel.value); idIn._auto = true; } });
    idIn.addEventListener("input", () => { idIn._auto = false; });
    checkBtn.addEventListener("click", async () => {
      const p = parseInt(portIn.value, 10); if (!p) return;
      checkBtn.disabled = true; mount(st, t("status.checking"));
      try {
        const r = await api.cmfPortCheck(p);
        if (r.available) mount(st, h("span", { style: "color:var(--good)" }, "✓ " + t("settings.cmf.portFree")));
        else {
          const kids = [h("span", { style: "color:var(--bad)" }, "✗ " + t("settings.cmf.portBusy"))];
          if (r.suggested) { const b = h("button", { class: "btn small", type: "button" }, t("settings.cmf.useSuggested", { port: r.suggested })); b.addEventListener("click", () => { portIn.value = r.suggested; mount(st, ""); }); kids.push(b); }
          mount(st, h("span", {}, ...kids));
        }
      } catch (e) { mount(st, String(e.message || e)); }
      checkBtn.disabled = false;
    });

    // ── O(1) Nyström Attention ──
    // NOT a "reasoning mode". This is a memory-efficient streaming attention
    // kernel (Nyström/landmark) that replaces standard KV-cache attention,
    // giving O(1) memory per decode step instead of O(n).
    const o1Sel = h("select", { style: "width:120px", title: "O(1) Nyström attention layer spec: which layers replace KV-cache with streaming Nyström attention" },
      h("option", { value: "" }, "O(1) авто"),
      h("option", { value: "all" }, "O(1) все слои"),
      h("option", { value: "deep" }, "O(1) глубокие"),
      h("option", { value: "off" }, "O(1) выкл."));
    if (sv.o1) o1Sel.value = sv.o1;
    const o1mIn = h("input", { type: "number", min: "4", placeholder: "32", value: sv.o1_m ?? "", style: "width:55px", title: "Landmark budget (≥4, default 32). More landmarks ≠ better: m=64 measured worse." });
    const o1wIn = h("input", { type: "number", min: "1", placeholder: "128", value: sv.o1_window ?? "", style: "width:60px", title: "Exact-window width — main quality lever (default 128)" });
    const o1sinkIn = h("input", { type: "number", min: "0", placeholder: "4", value: sv.o1_sink ?? "", style: "width:50px", title: "Permanent exact sink keys, StreamingLLM discipline (default 4)" });

    // ── Generation parameters ──
    const tempIn = h("input", { type: "number", step: "0.05", min: "0", max: "2.0", placeholder: "0.7", value: sv.temperature ?? "", style: "width:60px" });
    const topPIn = h("input", { type: "number", step: "0.05", min: "0", max: "1.0", placeholder: "0.9", value: sv.top_p ?? "", style: "width:60px" });
    const maxTokIn = h("input", { type: "number", min: "1", placeholder: "2048", value: sv.max_tokens ?? "", style: "width:75px" });
    // think_budget = 0 means reasoning disabled (enable_thinking=false);
    // null = model decides; N>0 = token budget for the <think> block.
    const noThinkCb = h("input", { type: "checkbox", checked: (sv.think_budget === 0) ? true : null, title: "Отключить режим размышлений (think_budget=0, enable_thinking=false для Qwen3/3.5)" });
    const thinkIn = h("input", { type: "number", min: "1", placeholder: "авто", value: (sv.think_budget && sv.think_budget > 0) ? sv.think_budget : "", style: "width:75px", title: "Бюджет токенов на размышления (think_budget). Пустое = модель решает сама." });
    if (sv.think_budget === 0) thinkIn.disabled = true;
    noThinkCb.addEventListener("change", () => {
      thinkIn.disabled = noThinkCb.checked;
      if (noThinkCb.checked) thinkIn.value = "";
    });
    const sysIn = h("input", { placeholder: "System prompt (optional)", value: sv.system_prompt || "", style: "flex:1;min-width:200px" });

    // ── MTP (Multi-Token Prediction): speculative decoding, NOT reasoning ──
    const skipMtpCb = h("input", { type: "checkbox", checked: sv.skip_mtp ? true : null, title: "Disable MTP speculative decoding (env CMF_MTP=0)" });

    // ── Settings drawer ──
    const drawerStyle = "display:none;width:100%;margin-top:6px;padding:10px 12px;background:var(--bg-subtle);border-radius:6px;border:1px solid var(--border)";
    const groupStyle = "display:flex;flex-wrap:wrap;gap:8px;align-items:center;padding:4px 0";
    const labelStyle = "font-size:11px;color:var(--text-muted);font-weight:600;text-transform:uppercase;letter-spacing:0.5px;width:100%;margin-bottom:2px";

    const drawer = h("div", { style: drawerStyle },
      // Group 1: O(1) Nyström Attention
      h("div", { style: labelStyle }, "O(1) Nyström Attention"),
      h("div", { style: groupStyle },
        h("span", { class: "small muted" }, "Слои:"), o1Sel,
        h("span", { class: "small muted" }, "m:"), o1mIn,
        h("span", { class: "small muted" }, "Window:"), o1wIn,
        h("span", { class: "small muted" }, "Sink:"), o1sinkIn
      ),
      h("hr", { style: "border:none;border-top:1px solid var(--border);margin:6px 0" }),
      // Group 2: Generation
      h("div", { style: labelStyle }, "Генерация"),
      h("div", { style: groupStyle },
        h("span", { class: "small muted" }, "Temp:"), tempIn,
        h("span", { class: "small muted" }, "Top-P:"), topPIn,
        h("span", { class: "small muted" }, "Max токенов:"), maxTokIn
      ),
      h("div", { style: groupStyle },
        h("span", { class: "small muted" }, "System prompt:"), sysIn
      ),
      h("hr", { style: "border:none;border-top:1px solid var(--border);margin:6px 0" }),
      // Group 3: Reasoning (thinking)
      h("div", { style: labelStyle }, "Режим размышлений"),
      h("div", { style: groupStyle },
        h("label", { class: "check small", title: "Для моделей Qwen3/3.5: enable_thinking=false — модель отвечает напрямую без блока <think>" }, noThinkCb, "Отключить размышления"),
        h("span", { class: "small muted" }, "Think budget:"), thinkIn
      ),
      h("hr", { style: "border:none;border-top:1px solid var(--border);margin:6px 0" }),
      // Group 4: Runtime
      h("div", { style: labelStyle }, "Runtime"),
      h("div", { style: groupStyle },
        h("label", { class: "check small", title: "MTP — Multi-Token Prediction (спекулятивное декодирование). Отключение замедляет генерацию, но стабилизирует output." }, skipMtpCb, "Отключить MTP")
      )
    );

    const gearBtn = h("button", { class: "btn small", type: "button", title: "⚙ Настройки модели" }, "⚙");
    gearBtn.addEventListener("click", () => {
      drawer.style.display = drawer.style.display === "none" ? "block" : "none";
    });

    const row = h("div", { class: "flex wrap", style: "gap:6px;align-items:center;margin:6px 0" },
      idIn, modelSel, portIn, checkBtn, threadsIn, h("label", { class: "flex", style: "gap:4px" }, gpuIn, "GPU"), gearBtn, removeBtn, st, drawer);
    const entry = { node: row, read: () => ({
      id: idIn.value.trim() || autoId(modelSel.value),
      model: modelSel.value.trim(),
      port: parseInt(portIn.value) || 8090,
      threads: parseInt(threadsIn.value) || 0,
      gpu: gpuIn.checked,
      temperature: tempIn.value !== "" ? parseFloat(tempIn.value) : null,
      top_p: topPIn.value !== "" ? parseFloat(topPIn.value) : null,
      max_tokens: maxTokIn.value !== "" ? parseInt(maxTokIn.value) : null,
      think_budget: noThinkCb.checked ? 0 : (thinkIn.value !== "" ? parseInt(thinkIn.value) : null),
      o1: o1Sel.value || null,
      o1_m: o1mIn.value !== "" ? parseInt(o1mIn.value) : null,
      o1_window: o1wIn.value !== "" ? parseInt(o1wIn.value) : null,
      o1_sink: o1sinkIn.value !== "" ? parseInt(o1sinkIn.value) : null,
      skip_mtp: skipMtpCb.checked,
      system_prompt: sysIn.value.trim() || null,
    }) };
    removeBtn.addEventListener("click", () => { const i = serverRows.indexOf(entry); if (i >= 0) serverRows.splice(i, 1); row.remove(); });
    serverRows.push(entry);
    serversWrap.appendChild(row);
    return entry;
  }
  initServers.forEach((sv) => makeServerRow(sv));
  const addServerBtn = h("button", { class: "btn small", type: "button" }, t("settings.cmf.addServer"));
  addServerBtn.addEventListener("click", () => makeServerRow());
  // "Install cortiq" — the CLI that RUNS the .cmf format. Must be present before
  // a local model server can start; this is step 1 for a new user.
  const cmfStatusEl = h("span", { class: "muted" }, "…");
  const cmfInstallBtn = h("button", { class: "btn" }, t("settings.cmf.install"));
  // live "serving <model> on :<port>" indicator for the managed local server
  const cmfServingEl = h("div", { class: "hint" });
  async function refreshCmfStatus() {
    try {
      const st = await api.cmfStatus();
      cmfInstallBtn.textContent = st.installed ? t("settings.cmf.reinstall") : t("settings.cmf.install");
      mount(
        cmfStatusEl,
        st.installed
          ? h("span", {}, h("span", { class: "badge ok" }, "✓"), " cortiq " + (st.version || ""))
          : h("span", {}, h("span", { class: "badge bad" }, "×"), " " + t("settings.cmf.notInstalled"))
      );
      // per-server serving indicators
      const servers = st.servers || [];
      if (!servers.length) {
        mount(cmfServingEl, "");
      } else {
        mount(cmfServingEl, h("div", {}, ...servers.map((sv) => {
          if (sv.last_error) return h("div", { style: "color:var(--bad)" }, "✗ " + sv.id + ": " + sv.last_error);
          if (sv.running) return h("div", { style: "color:var(--good)" },
            "▶ " + t("settings.cmf.serving", { model: sv.id, port: sv.port }) + (sv.healthy ? "" : " …"));
          return h("div", { class: "muted" }, "• " + sv.id + " :" + sv.port);
        })));
      }
    } catch (_) {
      mount(cmfStatusEl, "—");
    }
  }
  cmfInstallBtn.addEventListener("click", async () => {
    cmfInstallBtn.disabled = true;
    mount(cmfStatusEl, h("span", {}, h("span", { class: "spinner" }), " " + t("settings.cmf.installing")));
    try {
      await api.cmfInstall();
      // install compiles from crates.io — poll status for a few minutes
      for (let i = 0; i < 120; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        const st = await api.cmfStatus();
        if (st.installed) { toast(t("settings.cmf.installedOk"), "good"); break; }
      }
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    await refreshCmfStatus();
    cmfInstallBtn.disabled = false;
  });
  refreshCmfStatus();

  const saveBtn = h("button", { class: "btn primary" }, t("common.save"));
  saveBtn.addEventListener("click", async () => {
    saveBtn.disabled = true;
    // a pasted router key needs an env name to live under — default it
    if (rSecret.value.trim() && !rKeyEnv.value.trim()) rKeyEnv.value = "CORTIQ_ROUTER_KEY";
    const patch = {
      listen: listenIn.value.trim(),
      router: {
        enabled: rEnabled.cb.checked,
        url: rUrl.value.trim(),
        api_key_env: rKeyEnv.value.trim() || null,
        timeout_ms: parseInt(rTimeout.value) || 800,
        verify_tls: rVerify.cb.checked,
        taxonomy_id: rTax.value.trim() || null,
      },
      route: {
        text_strategy: strategySel.value,
        max_chars: parseInt(maxChars.value) || 4000,
        cache_ttl: cacheTtl.value.trim() || "60s",
        profile: profileSel.value,
      },
      breaker: { threshold: parseInt(brThresh.value) || 5, cooldown: brCool.value.trim() || "30s" },
      log: { level: logSel.value, bodies: logBodies.cb.checked },
      telemetry: { metrics: metricsCb.cb.checked, otlp_endpoint_env: s.telemetry?.otlp_endpoint_env || null },
      cortiq: { echo: echoCb.cb.checked },
      stats: {
        enabled: statsEnabled.cb.checked,
        file: statsFile.value.trim(),
        retention: statsRet.value.trim() || "7d",
        ring_size: s.stats?.ring_size ?? 500,
      },
      cache: {
        enabled: cacheEnabled.cb.checked,
        threshold: parseFloat(cacheThresh.value) || 0.92,
        ttl: cacheTtlIn.value.trim() || "1h",
        max_entries: s.cache?.max_entries ?? 1000,
        embed_model: cacheEmbedSel.value || null,
      },
      cmf: {
        ...(s.cmf || {}),
        local_only: cmfLocalOnly.cb.checked,
        manage_server: cmfManage.cb.checked,
        auto_install: cmfAutoInstall.cb.checked,
        auto_update: cmfAutoUpdate.cb.checked,
        // the managed-models list is the source of truth; clear the legacy single
        servers: serverRows.map((r) => r.read()).filter((sv) => sv.model && sv.id),
        local_model: "",
      },
    };
    // serde: drop null optionals
    if (!patch.router.api_key_env) delete patch.router.api_key_env;
    if (!patch.router.taxonomy_id) delete patch.router.taxonomy_id;
    if (!patch.telemetry.otlp_endpoint_env) delete patch.telemetry.otlp_endpoint_env;
    if (!patch.cache.embed_model) delete patch.cache.embed_model;
    try {
      const r = await api.putSettings(patch);
      const secretVal = rSecret.value.trim();
      if (secretVal) {
        await api.setSecret(rKeyEnv.value.trim(), secretVal);
        rSecret.value = "";
        rKeyBadge.textContent = t("models.form.secretStored");
        rKeyBadge.className = "badge store";
      }
      toast(r.needs_restart ? t("settings.needsRestart") : t("toast.saved"), r.needs_restart ? "warn" : "good");
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    saveBtn.disabled = false;
  });

  async function exportConfig() {
    const cfg = await api.getConfig();
    const blob = new Blob([JSON.stringify(cfg, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = h("a", { href: url, download: "gateway-config.json" });
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }
  const fileIn = h("input", { type: "file", accept: ".json,application/json", style: "display:none" });
  fileIn.addEventListener("change", async () => {
    const f = fileIn.files[0];
    if (!f) return;
    try {
      const cfg = JSON.parse(await f.text());
      await api.putConfig(cfg);
      toast(t("toast.imported"), "good");
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    fileIn.value = "";
  });

  mount(
    root,
    h(
      "div",
      { class: "page-head" },
      h(
        "div",
        { class: "flex" },
        h("div", { class: "grow" }, h("h2", {}, t("settings.title")), h("p", {}, t("settings.subtitle"))),
        saveBtn
      )
    ),
    h(
      "div",
      { class: "grid cols-2" },
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.router"))),
        field(t("settings.router.url"), rUrl),
        field(t("settings.router.keyEnv"), rKeyEnv),
        field(t("settings.router.secret"), h("div", { class: "key-row" }, rSecret, saveKeyBtn), rSecretHint),
        h("div", { class: "row" }, field(t("settings.router.timeout"), rTimeout), field(t("settings.router.taxonomy"), rTax)),
        rEnabled.node,
        rVerify.node,
        h("div", { class: "divider" }),
        h("div", { class: "flex" }, probeBtn),
        probeOut
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.route"))),
        field(t("settings.route.strategy"), strategySel, null, t("settings.route.strategyHelp")),
        h("div", { class: "row" },
          field(t("settings.route.maxChars"), maxChars, null, t("settings.route.maxCharsHelp")),
          field(t("settings.route.cacheTtl"), cacheTtl, null, t("settings.route.cacheTtlHelp"))),
        field(t("settings.route.profile"), profileSel, null, t("settings.route.profileHelp"))
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.breaker"))),
        h("div", { class: "row" }, field(t("settings.breaker.threshold"), brThresh), field(t("settings.breaker.cooldown"), brCool))
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.log"))),
        field(t("settings.log.level"), logSel),
        logBodies.node
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.telemetry"))),
        metricsCb.node,
        h("div", { class: "divider" }),
        h("div", { class: "card-head" }, h("h3", {}, t("settings.cortiq"))),
        echoCb.node
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.stats"))),
        statsEnabled.node,
        field(t("settings.stats.file"), statsFile),
        field(t("settings.stats.retention"), statsRet)
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.cache"))),
        cacheEnabled.node,
        h("div", { class: "row" }, field(t("settings.cache.threshold"), cacheThresh), field(t("settings.cache.ttl"), cacheTtlIn)),
        field(t("settings.cache.embed"), cacheEmbedSel),
        h("div", { class: "hint" }, t("settings.cache.note"))
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("settings.cmf"))),
        h("div", { class: "hint" }, t("settings.cmf.intro")),
        // Step 1 — install the cortiq runtime (the CLI that runs .cmf models).
        h(
          "label",
          { class: "field" },
          h("span", {}, "1. " + t("settings.cmf.runtime"), " ", help(t("settings.cmf.runtimeHelp"))),
          h("div", { class: "flex", style: "gap:8px;align-items:center;flex-wrap:wrap" }, cmfInstallBtn, cmfStatusEl)
        ),
        h("div", { class: "row" }, cmfAutoInstall.node, cmfAutoUpdate.node),
        h("div", { class: "divider" }),
        // Step 2 — managed local models: one supervised cortiq serve per row, each
        // on its own port. Add as many as you have .cmf files + free ports.
        h("div", { class: "field" },
          h("span", {}, "2. " + t("settings.cmf.models"), " ", help(t("settings.cmf.modelsHelp"))),
          h("div", { class: "hint" }, t("settings.cmf.rowHint")),
          h("div", { class: "hint" }, t("settings.cmf.importNote"),
            " ", h("a", { href: "#/import" }, t("settings.cmf.importLink") + " ↗"))),
        serversWrap,
        addServerBtn,
        cmfServingEl,
        h("div", { class: "hint" }, t("settings.cmf.speedNote")),
        h("div", { class: "divider" }),
        // Step 3 — how the gateway uses them.
        cmfManage.node,
        cmfLocalOnly.node
      )
    ),
    h(
      "div",
      { class: "card" },
      h("div", { class: "card-head" }, h("h3", {}, t("settings.listen"))),
      field(t("settings.listen"), listenIn),
      h("div", { class: "divider" }),
      h(
        "div",
        { class: "flex wrap" },
        h("button", { class: "btn", onClick: exportConfig }, t("settings.export")),
        h("button", { class: "btn", onClick: () => fileIn.click() }, t("settings.import")),
        fileIn
      )
    )
  );
  return root;
}
