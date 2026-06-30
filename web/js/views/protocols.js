// Protocols — live toggles for inbound API surfaces.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState } from "../app.js";

const ORDER = [
  "openai_chat",
  "openai_completions",
  "openai_embeddings",
  "openai_models",
  "anthropic_messages",
  "mcp",
  "native_passthrough",
];

export async function renderProtocols() {
  const meta = appState.meta || (await api.meta());
  const impl = meta.protocols_impl || {};
  const state = await api.getProtocols();
  const root = h("div");

  async function save() {
    try {
      await api.putProtocols(state);
      toast(t("toast.saved"), "good");
    } catch (e) {
      toast(String(e.message || e), "bad");
    }
  }

  function row(key) {
    const implemented = !!impl[key];
    const input = h("input", {
      type: "checkbox",
      checked: state[key] ? true : null,
      disabled: implemented ? null : true,
      onChange: (e) => {
        state[key] = e.target.checked;
        save();
      },
    });
    return h(
      "div",
      { class: "togglerow" },
      h(
        "div",
        { class: "meta" },
        h(
          "div",
          { class: "name" },
          t("protocols." + key),
          h("span", { class: "badge " + (implemented ? "ok" : "") }, implemented ? t("protocols.implemented") : t("protocols.planned"))
        ),
        h("div", { class: "desc" }, t("protocols." + key + ".d"))
      ),
      h("label", { class: "switch" }, input, h("span", { class: "slider" }))
    );
  }

  mount(
    root,
    h("div", { class: "page-head" }, h("h2", {}, t("protocols.title")), h("p", {}, t("protocols.subtitle"))),
    h("div", { class: "card" }, ...ORDER.map(row))
  );
  return root;
}
