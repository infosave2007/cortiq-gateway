// Settings — router/route/breaker/log/telemetry/stats + export/import config.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState } from "../app.js";

function field(label, control, hint) {
  return h("label", { class: "field" }, h("span", {}, label), control, hint ? h("div", { class: "hint" }, hint) : null);
}
function check(label, checked) {
  const cb = h("input", { type: "checkbox", checked: checked ? true : null });
  return { node: h("label", { class: "check" }, cb, label), cb };
}

export async function renderSettings() {
  const meta = appState.meta || (await api.meta());
  const s = await api.getSettings();
  const root = h("div");

  const listenIn = h("input", { value: s.listen || "" });
  // router
  const rUrl = h("input", { value: s.router?.url || "" });
  const rKeyEnv = h("input", { value: s.router?.api_key_env || "", placeholder: "CORTIQ_ROUTER_KEY" });
  const rTimeout = h("input", { type: "number", value: s.router?.timeout_ms ?? 800 });
  const rTax = h("input", { value: s.router?.taxonomy_id || "" });
  const rVerify = check(t("settings.router.verifyTls"), s.router?.verify_tls);
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

  const saveBtn = h("button", { class: "btn primary" }, t("common.save"));
  saveBtn.addEventListener("click", async () => {
    saveBtn.disabled = true;
    const patch = {
      listen: listenIn.value.trim(),
      router: {
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
    };
    // serde: drop null optionals
    if (!patch.router.api_key_env) delete patch.router.api_key_env;
    if (!patch.router.taxonomy_id) delete patch.router.taxonomy_id;
    if (!patch.telemetry.otlp_endpoint_env) delete patch.telemetry.otlp_endpoint_env;
    try {
      const r = await api.putSettings(patch);
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
        h("div", { class: "row" }, field(t("settings.router.timeout"), rTimeout), field(t("settings.router.taxonomy"), rTax)),
        rVerify.node
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
