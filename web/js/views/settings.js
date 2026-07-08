// Settings — router/route/breaker/log/telemetry/stats + export/import config.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState, SITE_URL } from "../app.js";

function field(label, control, hint) {
  return h("label", { class: "field" }, h("span", {}, label), control, hint ? h("div", { class: "hint" }, hint) : null);
}
function check(label, checked) {
  const cb = h("input", { type: "checkbox", checked: checked ? true : null });
  return { node: h("label", { class: "check" }, cb, label), cb };
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
  const rVerify = check(t("settings.router.verifyTls"), s.router?.verify_tls);
  const rEnabled = check(t("settings.router.enabled"), s.router?.enabled !== false);
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
  const saveKeyBtn = h("button", { class: "btn sm" }, "✓ " + t("settings.router.saveKey"));
  saveKeyBtn.addEventListener("click", async () => {
    saveKeyBtn.disabled = true;
    try {
      if (await saveKey()) toast(t("settings.router.keySaved"), "good");
      else toast(t("settings.router.keyEmpty"), "warn");
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    saveKeyBtn.disabled = false;
  });
  // "Test router" — real /v1/route call: tells key problems apart from a down router
  const probeBtn = h("button", { class: "btn" }, t("settings.router.test"));
  const probeOut = h("div", { class: "hint", style: "margin-top:8px" });
  probeBtn.addEventListener("click", async () => {
    probeBtn.disabled = true;
    mount(probeOut, h("span", { class: "spinner" }));
    try {
      // If a key was just pasted but not saved, persist it first so the test uses it.
      if (rSecret.value.trim()) {
        try {
          await saveKey();
        } catch (_) {
          /* fall through — probe will report no_key/auth */
        }
      }
      const r = await api.probeRouter();
      const parts = [t(probeMsgKey(r.status), { ms: r.latency_ms ?? "—" })];
      if (r.message) parts.push(" — " + r.message);
      const line = h("span", {}, ...parts);
      const out = [line];
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
    } catch (e) {
      mount(probeOut, String(e.message || e));
    }
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
  const logBodies = check(t("settings.log.bodies"), s.log?.bodies);
  // telemetry / cortiq
  const metricsCb = check(t("settings.telemetry.metrics"), s.telemetry?.metrics);
  const echoCb = check(t("settings.cortiq.echo"), s.cortiq?.echo);
  // stats
  const statsEnabled = check(t("settings.stats.enabled"), s.stats?.enabled);
  const statsFile = h("input", { value: s.stats?.file || "" });
  const statsRet = h("input", { value: s.stats?.retention || "7d" });
  // cache
  const modelsData = await api.listModels();
  const embedModels = (modelsData.models || []).filter((m) => m.kind === "embedding").map((m) => m.id);
  const cacheEnabled = check(t("settings.cache.enabled"), s.cache?.enabled);
  const cacheThresh = h("input", { type: "number", step: "0.01", value: s.cache?.threshold ?? 0.92 });
  const cacheTtlIn = h("input", { value: s.cache?.ttl || "1h" });
  const cacheEmbedSel = h(
    "select",
    {},
    h("option", { value: "" }, "auto"),
    ...embedModels.map((id) => h("option", { value: id, selected: id === s.cache?.embed_model }, id))
  );
  // local models (CMF) — run a local .cmf server and/or force local-only routing
  const cmfLocalOnly = check(t("settings.cmf.localOnly"), s.cmf?.local_only);
  const cmfManage = check(t("settings.cmf.manage"), s.cmf?.manage_server);
  const cmfAutoInstall = check(t("settings.cmf.autoInstall"), s.cmf?.auto_install);
  const cmfAutoUpdate = check(t("settings.cmf.autoUpdate"), s.cmf?.auto_update);
  const cmfModel = h("input", { value: s.cmf?.local_model || "", placeholder: "models/model.cmf" });
  const cmfPort = h("input", { type: "number", value: s.cmf?.local_port ?? 8081 });
  const cmfModelId = h("input", { value: s.cmf?.model_id || "cmf-local" });
  const cmfThreads = h("input", { type: "number", min: 0, value: s.cmf?.threads ?? 8 });
  const cmfGpu = check(t("settings.cmf.gpu"), s.cmf?.gpu);

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
        local_model: cmfModel.value.trim(),
        local_port: parseInt(cmfPort.value) || 8081,
        model_id: cmfModelId.value.trim() || "cmf-local",
        threads: parseInt(cmfThreads.value) || 0,
        gpu: cmfGpu.cb.checked,
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
        field(t("settings.route.strategy"), strategySel),
        h("div", { class: "row" }, field(t("settings.route.maxChars"), maxChars), field(t("settings.route.cacheTtl"), cacheTtl)),
        field(t("settings.route.profile"), profileSel)
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
        cmfLocalOnly.node,
        cmfManage.node,
        h("div", { class: "row" }, cmfAutoInstall.node, cmfAutoUpdate.node),
        field(t("settings.cmf.model"), cmfModel),
        h("div", { class: "row" }, field(t("settings.cmf.port"), cmfPort), field(t("settings.cmf.modelId"), cmfModelId)),
        h("div", { class: "row" }, field(t("settings.cmf.threads"), cmfThreads), cmfGpu.node),
        h("div", { class: "hint" }, t("settings.cmf.note")),
        h("div", { class: "hint" }, t("settings.cmf.speedNote"))
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
