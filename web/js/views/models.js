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

function modelForm(meta, existing) {
  const m = existing || { provider: "openai", kind: "chat", cost_tier: "cheap", caps: [], price_in: 0, price_out: 0 };
  const idIn = h("input", { value: m.id || "", placeholder: "local-qwen", disabled: existing ? true : null });
  const providerSel = h("select", {}, ...(meta.providers || ["openai"]).map((p) => opt(p, p, p === m.provider)));
  const baseUrlIn = h("input", { value: m.base_url || "", placeholder: "http://localhost:8000/v1" });
  const modelIn = h("input", { value: m.model || "", placeholder: "qwen2.5-7b-instruct" });
  const kindSel = h("select", {}, ...(meta.kinds || ["chat"]).map((k) => opt(k, k, k === m.kind)));
  const tierSel = h("select", {}, ...(meta.cost_tiers || ["cheap"]).map((c) => opt(c, c, c === m.cost_tier)));
  const priceInIn = h("input", { type: "number", step: "0.01", value: m.price_in ?? 0 });
  const priceOutIn = h("input", { type: "number", step: "0.01", value: m.price_out ?? 0 });
  const keyEnvIn = h("input", { value: m.api_key_env || "", placeholder: "OPENAI_API_KEY" });
  const secretIn = h("input", { type: "password", placeholder: t("models.form.secretPlaceholder") });
  const capInputs = (meta.caps || ["tools", "vision"]).map((c) => {
    const cb = h("input", { type: "checkbox", checked: (m.caps || []).includes(c) ? true : null });
    return { c, cb };
  });

  const node = h(
    "div",
    {},
    h("div", { class: "row" }, field(t("models.form.id"), idIn, existing ? null : t("models.form.idHint")), field(t("models.form.provider"), providerSel)),
    field(t("models.form.baseUrl"), baseUrlIn),
    field(t("models.form.model"), modelIn),
    h("div", { class: "row" }, field(t("models.form.kind"), kindSel), field(t("models.form.costTier"), tierSel)),
    h("div", { class: "row" }, field(t("models.form.priceIn"), priceInIn), field(t("models.form.priceOut"), priceOutIn)),
    field(t("models.form.caps"), h("div", { class: "flex wrap" }, ...capInputs.map(({ c, cb }) => h("label", { class: "check" }, cb, c)))),
    field(t("models.form.apiKeyEnv"), keyEnvIn, t("models.form.apiKeyEnvHint")),
    field(t("models.form.secret"), secretIn)
  );

  const getValue = () => ({
    model: {
      id: existing ? m.id : idIn.value.trim(),
      provider: providerSel.value,
      base_url: baseUrlIn.value.trim(),
      model: modelIn.value.trim(),
      kind: kindSel.value,
      cost_tier: tierSel.value,
      price_in: parseFloat(priceInIn.value) || 0,
      price_out: parseFloat(priceOutIn.value) || 0,
      api_key_env: keyEnvIn.value.trim() || null,
      caps: capInputs.filter(({ cb }) => cb.checked).map(({ c }) => c),
    },
    secret: secretIn.value,
    keyEnv: keyEnvIn.value.trim(),
  });

  return { node, getValue };
}

async function openModal(meta, existing, reload) {
  const { node, getValue } = modelForm(meta, existing);
  modal(existing ? t("models.form.editTitle") : t("models.form.addTitle"), node, async () => {
    const { model, secret, keyEnv } = getValue();
    if (!model.id) {
      toast(t("models.form.id") + "?", "bad");
      return false;
    }
    // strip null api_key_env so serde keeps it optional
    if (!model.api_key_env) delete model.api_key_env;
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
    h("td", { class: "mono" }, m.id),
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
        h("button", { class: "btn sm ghost", onClick: () => openModal(meta, m, reload) }, t("common.edit")),
        h(
          "button",
          {
            class: "btn sm danger",
            onClick: async () => {
              if (!(await confirmDialog(t("models.deleteConfirm", { id: m.id })))) return;
              try {
                await api.deleteModel(m.id);
                toast(t("toast.deleted"), "good");
                reload();
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
