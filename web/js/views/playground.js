// Playground — send a prompt through the live pipeline, inspect routing decision.
import { h, mount, money, ms } from "../ui.js";
import { t } from "../i18n.js";
import { api, testStream } from "../api.js";
import { appState } from "../app.js";

export async function renderPlayground() {
  const meta = appState.meta || (await api.meta());
  const modelsData = await api.listModels();
  const ids = (modelsData.models || []).map((m) => m.id);

  let mode = "auto";
  const root = h("div");

  const sysIn = h("textarea", { placeholder: t("pg.systemPlaceholder"), rows: 2 });
  const promptIn = h("textarea", { placeholder: t("pg.promptPlaceholder"), rows: 5 });
  const tempIn = h("input", { type: "number", step: "0.1", value: 0.7 });
  const maxTokIn = h("input", { type: "number", value: 512 });
  const streamCb = h("input", { type: "checkbox", checked: true });
  const profileSel = h("select", {}, ...(meta.profiles || []).map((p) => h("option", { value: p }, p)));
  const modelSel = h("select", {}, ...ids.map((id) => h("option", { value: id }, id)));

  const modeSeg = h(
    "div",
    { class: "seg" },
    h("button", { class: "active", onClick: (e) => setMode("auto", e) }, t("pg.mode.auto")),
    h("button", { onClick: (e) => setMode("pinned", e) }, t("pg.mode.pinned"))
  );
  function buildModeExtra() {
    return h(
      "label",
      { class: "field" },
      h("span", {}, mode === "auto" ? t("pg.profile") : t("pg.model")),
      mode === "auto" ? profileSel : modelSel
    );
  }
  let modeExtra = buildModeExtra();
  function setMode(m, e) {
    mode = m;
    modeSeg.querySelectorAll("button").forEach((b) => b.classList.remove("active"));
    e.target.classList.add("active");
    const next = buildModeExtra();
    modeExtra.replaceWith(next);
    modeExtra = next;
  }

  const out = h("div", { class: "answer" }, h("div", { class: "empty" }, t("pg.empty")));
  const decision = h("div", { class: "card routecard" }, h("div", { class: "card-head" }, h("h3", {}, t("pg.decision"))), h("div", { class: "empty" }, "—"));

  const sendBtn = h("button", { class: "btn primary" }, t("pg.send"));
  sendBtn.addEventListener("click", async () => {
    const prompt = promptIn.value.trim();
    if (!prompt) return;
    const messages = [];
    if (sysIn.value.trim()) messages.push({ role: "system", content: sysIn.value.trim() });
    messages.push({ role: "user", content: prompt });
    let model = "cortiq-auto";
    if (mode === "auto") {
      if (profileSel.value) model = "cortiq-auto:" + profileSel.value;
    } else {
      model = modelSel.value;
    }
    sendBtn.disabled = true;
    sendBtn.textContent = t("pg.sending");
    mount(out, h("div", { class: "flex" }, h("span", { class: "spinner" }), " ", t("pg.sending")));
    const body = {
      model,
      messages,
      temperature: parseFloat(tempIn.value),
      max_tokens: parseInt(maxTokIn.value) || null,
    };
    try {
      if (streamCb.checked) {
        const started = performance.now();
        let acc = "";
        out.textContent = "";
        const info = await testStream(body, (d) => {
          acc += d;
          out.textContent = acc;
        });
        out.textContent = acc || "—";
        renderDecision({
          cortiq: {
            task_label: info.task_label,
            complexity: { score: info.score, tier: info.tier },
            selected_model: info.selected_model,
            route_source: info.route_source,
            cost_usd: info.cost_usd,
            failover: false,
          },
          usage: {},
          latency_ms: Math.round(performance.now() - started),
        });
      } else {
        const r = await api.test(body);
        if (r.ok) {
          mount(out, r.answer || h("span", { class: "muted" }, "—"));
          renderDecision(r);
        } else {
          mount(out, h("div", { class: "callout" }, h("b", {}, t("pg.error") + ": "), r.error || ""));
          mount(decision, h("div", { class: "card-head" }, h("h3", {}, t("pg.decision"))), h("div", { class: "empty" }, "—"));
        }
      }
    } catch (e) {
      mount(out, h("div", { class: "callout" }, String(e.message || e)));
    }
    sendBtn.disabled = false;
    sendBtn.textContent = t("pg.send");
  });

  function kv(k, v) {
    return h("div", { class: "kv" }, h("span", { class: "k" }, k), h("span", { class: "v" }, v));
  }
  function renderDecision(r) {
    const c = r.cortiq || {};
    const score = c.complexity?.score ?? 0;
    mount(
      decision,
      h("div", { class: "card-head" }, h("h3", {}, t("pg.decision"))),
      kv(t("pg.task"), c.task_label || "—"),
      kv(t("pg.tier"), c.complexity?.tier ? h("span", { class: "badge tier-" + c.complexity.tier }, c.complexity.tier) : "—"),
      h(
        "div",
        { class: "kv", style: "display:block" },
        h("div", { class: "flex" }, h("span", { class: "k grow" }, t("pg.score")), h("span", { class: "v" }, score.toFixed(2))),
        h("div", { class: "scorebar" }, h("i", { style: `width:${Math.min(100, score * 100)}%` }))
      ),
      kv(t("pg.selected"), h("span", { class: "mono" }, c.selected_model || "—")),
      kv(t("pg.source"), c.route_source || "—"),
      kv(t("pg.tokens"), `${r.usage?.prompt_tokens ?? 0}/${r.usage?.completion_tokens ?? 0}`),
      kv(t("pg.cost"), money(c.cost_usd)),
      kv(t("pg.latency"), ms(r.latency_ms)),
      kv(t("pg.failover"), c.failover ? "yes" : "no")
    );
  }

  mount(
    root,
    h("div", { class: "page-head" }, h("h2", {}, t("pg.title")), h("p", {}, t("pg.subtitle"))),
    h(
      "div",
      { class: "pg" },
      h(
        "div",
        {},
        h(
          "div",
          { class: "card" },
          h("label", { class: "field" }, h("span", {}, t("pg.system")), sysIn),
          h("label", { class: "field" }, h("span", {}, t("pg.prompt")), promptIn),
          h("label", { class: "field" }, h("span", {}, t("pg.mode")), modeSeg),
          modeExtra,
          h("div", { class: "row" }, h("label", { class: "field" }, h("span", {}, t("pg.temp")), tempIn), h("label", { class: "field" }, h("span", {}, t("pg.maxTokens")), maxTokIn)),
          h(
            "div",
            { class: "flex", style: "justify-content:space-between" },
            h("label", { class: "check", style: "margin:0" }, streamCb, t("pg.stream")),
            sendBtn
          )
        ),
        h("div", { class: "card" }, h("div", { class: "card-head" }, h("h3", {}, t("pg.answer"))), out)
      ),
      decision
    )
  );
  return root;
}
