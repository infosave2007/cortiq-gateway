// Models — pool table with add/edit modal, probe, and per-model secret entry.
import { h, mount, modal, toast, confirmDialog, money } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState } from "../app.js";

function field(label, control, hint) {
  return h("label", { class: "field" }, h("span", {}, label), control, hint ? h("div", { class: "hint" }, hint) : null);
}
function opt(v, label, sel) {
  return h("option", { value: v, selected: sel ? true : null }, label || v);
}

function keyBadge(src) {
  const map = { store: "store", env: "env", missing: "bad", none: "" };
  return h("span", { class: "badge " + (map[src] ?? "") }, src);
}

// Verified per-provider defaults (base URL, key env, whether a key is needed).
const PROVIDER_DEFAULTS = {
  openai:     { base: "https://api.openai.com/v1",    keyEnv: "OPENAI_API_KEY",     needsKey: true,  tier: "mid",       caps: ["tools", "vision"] },
  anthropic:  { base: "https://api.anthropic.com",    keyEnv: "ANTHROPIC_API_KEY",  needsKey: true,  tier: "expensive", caps: ["tools", "vision"] },
  openrouter: { base: "https://openrouter.ai/api/v1", keyEnv: "OPENROUTER_API_KEY", needsKey: true,  tier: "mid",       caps: ["tools", "vision"] },
  lmstudio:   { base: "http://localhost:1234/v1",     keyEnv: "",                   needsKey: false, tier: "local",     caps: ["tools", "vision"] },
  ollama:     { base: "http://localhost:11434/v1",    keyEnv: "",                   needsKey: false, tier: "local",     caps: ["tools"] },
  http:       { base: "",                              keyEnv: "",                   needsKey: false, tier: "cheap",     caps: ["tools"] },
};

// Derive a clean model id from a model name (last path segment, slugified).
// e.g. "qwen/qwen3.7-max" → "qwen3-7-max".
function slugId(name) {
  const base = (name || "").split("/").pop() || "";
  return base.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

function modelForm(meta, existing) {
  const m = existing || { provider: "openai", kind: "chat", cost_tier: "mid", caps: ["tools"], price_in: 0, price_out: 0 };
  const idIn = h("input", { value: m.id || "", placeholder: "my-model", disabled: existing ? true : null });
  const providerSel = h("select", {}, ...(meta.providers || ["openai"]).map((p) => opt(p, p, p === m.provider)));
  const baseUrlIn = h("input", { value: m.base_url || "", placeholder: "https://api.openai.com/v1" });

  // API key — typed directly, stored securely as a secret under a provider env name.
  const keyIn = h("input", { type: "password", placeholder: t("models.form.apiKeyPlaceholder"), autocomplete: "new-password" });
  const testBtn = h("button", { class: "btn small", type: "button" }, t("models.form.testKey"));
  const keyStatus = h("span", { class: "small", style: "margin-left:8px" });

  // Model — below the key (some providers list models from the key); searchable datalist.
  const dlId = "model-opts-" + (m.id || "new");
  const dl = h("datalist", { id: dlId });
  if (m.model) dl.appendChild(h("option", { value: m.model }));
  const modelIn = h("input", { value: m.model || "", placeholder: t("models.form.modelPlaceholder"), list: dlId, autocomplete: "off" });

  const kindSel = h("select", {}, ...(meta.kinds || ["chat"]).map((k) => opt(k, k, k === m.kind)));
  const tierSel = h("select", {}, ...(meta.cost_tiers || ["cheap"]).map((c) => opt(c, c, c === m.cost_tier)));
  const priceInIn = h("input", { type: "number", step: "0.01", value: m.price_in ?? 0 });
  const priceOutIn = h("input", { type: "number", step: "0.01", value: m.price_out ?? 0 });
  const capInputs = (meta.caps || ["tools", "vision"]).map((c) => {
    const cb = h("input", { type: "checkbox", checked: (m.caps || []).includes(c) ? true : null });
    return { c, cb };
  });

  const envFor = () =>
    (existing && existing.api_key_env) ||
    (PROVIDER_DEFAULTS[providerSel.value] || {}).keyEnv ||
    providerSel.value.toUpperCase().replace(/[^A-Z0-9]/g, "_") + "_API_KEY";

  // Fill in a provider's verified defaults when it is selected.
  function applyDefaults() {
    const d = PROVIDER_DEFAULTS[providerSel.value];
    if (!d) return;
    baseUrlIn.value = d.base;
    tierSel.value = d.tier;
    capInputs.forEach(({ c, cb }) => { cb.checked = d.caps.includes(c); });
    keyIn.placeholder = d.needsKey ? t("models.form.apiKeyPlaceholder") : t("models.form.apiKeyNotNeeded");
    mount(keyStatus, "");
  }
  providerSel.addEventListener("change", applyDefaults);
  if (!existing) applyDefaults();

  // Auto-fill the id from the model name (until the user edits id by hand).
  if (!existing) {
    modelIn.addEventListener("input", () => {
      if (!idIn.value || idIn._auto) {
        idIn.value = slugId(modelIn.value);
        idIn._auto = true;
      }
    });
    idIn.addEventListener("input", () => { idIn._auto = false; });
  }

  async function testKey() {
    if (!baseUrlIn.value.trim()) { mount(keyStatus, h("span", { style: "color:var(--bad)" }, t("models.form.baseUrl") + "?")); return; }
    testBtn.disabled = true;
    mount(keyStatus, t("models.form.testing"));
    try {
      const r = await api.providerModels({
        provider: providerSel.value,
        base_url: baseUrlIn.value.trim(),
        api_key: keyIn.value.trim() || null,
        api_key_env: envFor(),
      });
      if (r.ok) {
        dl.replaceChildren(...(r.models || []).map((mm) => h("option", { value: mm })));
        mount(keyStatus, h("span", { style: "color:var(--good)" }, "✓ " + t("models.form.foundModels", { n: r.count })));
        modelIn.focus();
      } else {
        mount(keyStatus, h("span", { style: "color:var(--bad)" }, "✗ " + (r.error || t("common.error"))));
      }
    } catch (e) {
      mount(keyStatus, h("span", { style: "color:var(--bad)" }, "✗ " + String(e.message || e)));
    }
    testBtn.disabled = false;
  }
  testBtn.addEventListener("click", testKey);

  const keyHint =
    existing && existing.key_source === "store" ? t("models.form.keyStored")
    : existing && existing.key_source === "env" ? t("models.form.keyFromEnv")
    : t("models.form.apiKeyHint");

  // ⚙ Generation & Thinking Parameters
  const tempIn = h("input", { type: "number", step: "0.05", min: "0", max: "2.0", placeholder: "0.7", value: m.temperature ?? "" });
  const topPIn = h("input", { type: "number", step: "0.05", min: "0", max: "1.0", placeholder: "0.9", value: m.top_p ?? "" });
  const maxTokIn = h("input", { type: "number", min: "1", placeholder: "2048", value: m.max_tokens ?? "" });
  const thinkIn = h("input", { type: "number", min: "0", placeholder: "1024", value: m.think_budget ?? "" });
  const o1Sel = h("select", {},
    opt("", t("import.adv.auto")),
    opt("all", "all — O(1) Attention"),
    opt("deep", "deep — O(1) Deep"),
    opt("off", "off — Standard"));
  if (m.o1) o1Sel.value = m.o1;
  const skipMtpCb = h("input", { type: "checkbox", checked: m.skip_mtp ? true : null });
  const sysPromptIn = h("textarea", { rows: 2, placeholder: t("models.form.systemPromptPlaceholder"), value: m.system_prompt || "" });

  const advParams = h(
    "details",
    { class: "adv-params", style: "margin-top:12px;padding:10px;border:1px solid var(--border);border-radius:6px;" },
    h("summary", { style: "font-weight:600;cursor:pointer;" }, "⚙ " + t("models.form.paramsTitle")),
    h("div", { class: "row", style: "margin-top:8px" },
      field(t("models.form.temperature"), tempIn, t("models.form.temperatureHint")),
      field(t("models.form.topP"), topPIn, t("models.form.topPHint"))),
    h("div", { class: "row" },
      field(t("models.form.maxTokens"), maxTokIn, t("models.form.maxTokensHint")),
      field(t("models.form.thinkBudget"), thinkIn, t("models.form.thinkBudgetHint"))),
    h("div", { class: "row" },
      field("O(1) Attention", o1Sel, t("import.adv.o1Hint")),
      field("MTP", h("label", { class: "check" }, skipMtpCb, t("import.adv.skipMtp")))),
    field(t("models.form.systemPrompt"), sysPromptIn, t("models.form.systemPromptHint"))
  );

  const node = h(
    "div",
    {},
    h("div", { class: "row" }, field(t("models.form.id"), idIn, existing ? null : t("models.form.idHint")), field(t("models.form.provider"), providerSel)),
    field(t("models.form.baseUrl"), baseUrlIn),
    field(t("models.form.apiKey"), h("div", { class: "flex wrap", style: "gap:8px;align-items:center" }, keyIn, testBtn, keyStatus), keyHint),
    field(t("models.form.model"), modelIn, t("models.form.modelHint")),
    dl,
    h("div", { class: "row" }, field(t("models.form.kind"), kindSel), field(t("models.form.costTier"), tierSel)),
    h("div", { class: "row" }, field(t("models.form.priceIn"), priceInIn), field(t("models.form.priceOut"), priceOutIn)),
    field(t("models.form.caps"), h("div", { class: "flex wrap" }, ...capInputs.map(({ c, cb }) => h("label", { class: "check" }, cb, c)))),
    advParams
  );

  const getValue = () => {
    const typedKey = keyIn.value.trim();
    const hasKey = !!typedKey || !!(existing && existing.api_key_env);
    const keyEnv = envFor();
    return {
      model: {
        id: existing ? m.id : idIn.value.trim(),
        provider: providerSel.value,
        base_url: baseUrlIn.value.trim(),
        model: modelIn.value.trim(),
        kind: kindSel.value,
        cost_tier: tierSel.value,
        price_in: parseFloat(priceInIn.value) || 0,
        price_out: parseFloat(priceOutIn.value) || 0,
        api_key_env: hasKey ? keyEnv : null,
        caps: capInputs.filter(({ cb }) => cb.checked).map(({ c }) => c),
        temperature: tempIn.value !== "" ? parseFloat(tempIn.value) : null,
        top_p: topPIn.value !== "" ? parseFloat(topPIn.value) : null,
        max_tokens: maxTokIn.value !== "" ? parseInt(maxTokIn.value) : null,
        think_budget: thinkIn.value !== "" ? parseInt(thinkIn.value) : null,
        o1: o1Sel.value || null,
        skip_mtp: skipMtpCb.checked,
        system_prompt: sysPromptIn.value.trim() || null,
      },
      secret: typedKey,
      keyEnv,
    };
  };

  return { node, getValue };
}

async function openModal(meta, existing, reload) {
  const { node, getValue } = modelForm(meta, existing);
  modal(existing ? t("models.form.editTitle") : t("models.form.addTitle"), node, async () => {
    const { model, secret, keyEnv } = getValue();
    // Forgot the id? Derive it from the model name so saving never errors on a
    // blank id.
    if (!model.id) model.id = slugId(model.model);
    if (!model.id) {
      toast(t("models.form.idOrModel"), "bad");
      return false;
    }
    if (!model.api_key_env) delete model.api_key_env; // keep serde field optional
    try {
      if (existing) await api.updateModel(existing.id, model);
      else await api.createModel(model);
      if (secret && keyEnv) await api.setSecret(keyEnv, secret);
      toast(t("toast.saved"), "good");
      reload();
    } catch (e) {
      toast(String(e.message || e), "bad");
      return false;
    }
  });
}

export async function renderModels() {
  const meta = appState.meta || (await api.meta());
  const root = h("div");

  async function reload() {
    const data = await api.listModels();
    const models = data.models || [];
    mount(
      root,
      h(
        "div",
        { class: "page-head" },
        h(
          "div",
          { class: "flex" },
          h("div", { class: "grow" }, h("h2", {}, t("models.title")), h("p", {}, t("models.subtitle"))),
          h("button", { class: "btn primary", onClick: () => openModal(meta, null, reload) }, "+ " + t("models.add"))
        )
      ),
      models.length === 0
        ? h("div", { class: "card" }, h("div", { class: "empty" }, t("models.empty")))
        : h(
            "div",
            { class: "card" },
            h(
              "div",
              { class: "table-wrap" },
              h(
                "table",
                {},
                h(
                  "thead",
                  {},
                  h(
                    "tr",
                    {},
                    h("th", {}, t("models.col.id")),
                    h("th", {}, t("models.col.provider")),
                    h("th", {}, t("models.col.model")),
                    h("th", {}, t("models.col.tier")),
                    h("th", {}, t("models.col.key")),
                    h("th", {}, t("models.col.caps")),
                    h("th", {}, t("models.col.price")),
                    h("th", { class: "right-align" }, t("common.actions"))
                  )
                ),
                h(
                  "tbody",
                  {},
                  ...models.map((m) => row(m, meta, reload))
                )
              )
            )
          )
    );
  }

  await reload();
  return root;
}

function row(m, meta, reload) {
  const probeBtn = h("button", { class: "btn sm ghost" }, t("models.probe"));
  probeBtn.addEventListener("click", async () => {
    probeBtn.disabled = true;
    probeBtn.textContent = "…";
    try {
      const r = await api.probeModel(m.id);
      if (r.ok) toast(t("models.probe.ok", { ms: r.latency_ms }), "good");
      else toast(t("models.probe.fail", { err: r.error || "" }), "bad");
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
    probeBtn.disabled = false;
    probeBtn.textContent = t("models.probe");
  });
  return h(
    "tr",
    {},
    h("td", { class: "mono" }, m.id,
      m.managed ? h("span", { class: "badge", style: "margin-left:6px", title: t("models.managedHint") }, t("models.managed")) : null),
    h("td", {}, m.provider),
    h("td", { class: "mono" }, m.model),
    h("td", {}, h("span", { class: "badge " + (m.cost_tier || "") }, m.cost_tier || "—")),
    h("td", {}, keyBadge(m.key_source || (m.api_key_env ? "missing" : "none"))),
    h("td", {}, (m.caps || []).length ? h("div", { class: "flex wrap" }, ...m.caps.map((c) => h("span", { class: "badge" }, c))) : "—"),
    h("td", { class: "mono" }, `${m.price_in || 0}/${m.price_out || 0}`),
    h(
      "td",
      { class: "right-align" },
      h(
        "div",
        { class: "flex", style: "justify-content:flex-end" },
        probeBtn,
        m.managed ? null : h("button", { class: "btn sm ghost", onClick: () => openModal(meta, m, reload) }, t("common.edit")),
        h(
          "button",
          {
            class: "btn sm danger",
            onClick: async () => {
              if (!(await confirmDialog(t("models.deleteConfirm", { id: m.id })))) return;
              try {
                await api.deleteModel(m.id);
                toast(t("toast.deleted"), "good");
                await reload();
              } catch (e) {
                toast(String(e.message || e), "bad");
              }
            },
          },
          t("common.delete")
        )
      )
    )
  );
}
