// API keys — list, create (with copy-once), revoke.
import { h, mount, modal, toast, confirmDialog } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";

export async function renderKeys() {
  const modelsData = await api.listModels();
  const ids = (modelsData.models || []).map((m) => m.id);
  const root = h("div");

  function openCreate(reload) {
    const accountIn = h("input", { placeholder: "team-a" });
    const rateIn = h("input", { type: "number", value: 600 });
    const customIn = h("input", { placeholder: "sk-gw-…" });
    const allowInputs = ids.map((id) => ({ id, cb: h("input", { type: "checkbox" }) }));
    const body = h(
      "div",
      {},
      h("label", { class: "field" }, h("span", {}, t("keys.form.account")), accountIn),
      h("label", { class: "field" }, h("span", {}, t("keys.form.rate")), rateIn),
      h(
        "label",
        { class: "field" },
        h("span", {}, t("keys.form.allow")),
        h("div", { class: "flex wrap" }, ...allowInputs.map(({ id, cb }) => h("label", { class: "check" }, cb, id))),
        h("div", { class: "hint" }, t("keys.form.allowHint"))
      ),
      h("label", { class: "field" }, h("span", {}, t("keys.form.custom")), customIn, h("div", { class: "hint" }, t("keys.form.customHint")))
    );
    modal(t("keys.form.title"), body, async () => {
      try {
        const r = await api.createKey({
          key: customIn.value.trim(),
          account: accountIn.value.trim() || "default",
          rate_per_min: parseInt(rateIn.value) || 0,
          allow_models: allowInputs.filter(({ cb }) => cb.checked).map(({ id }) => id),
        });
        reload();
        showCreated(r.key);
      } catch (e) {
        toast(String(e.message || e), "bad");
        return false;
      }
    }, t("common.add"));
  }

  function showCreated(key) {
    const input = h("input", { value: key, readonly: true, class: "mono" });
    const copyBtn = h("button", { class: "btn", onClick: () => { input.select(); navigator.clipboard?.writeText(key); toast(t("common.copied"), "good"); } }, t("common.copy"));
    modal(t("common.ok"), h("div", {}, h("p", {}, t("keys.created")), h("div", { class: "flex" }, input, copyBtn)), null);
  }

  async function reload() {
    const data = await api.listKeys();
    const keys = data.keys || [];
    mount(
      root,
      h(
        "div",
        { class: "page-head" },
        h(
          "div",
          { class: "flex" },
          h("div", { class: "grow" }, h("h2", {}, t("keys.title")), h("p", {}, t("keys.subtitle"))),
          h("button", { class: "btn primary", onClick: () => openCreate(reload) }, "+ " + t("keys.add"))
        )
      ),
      data.open_mode ? h("div", { class: "callout info" }, t("keys.openMode")) : null,
      keys.length === 0
        ? h("div", { class: "card" }, h("div", { class: "empty" }, t("keys.empty")))
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
                  h("tr", {}, h("th", {}, t("keys.col.key")), h("th", {}, t("keys.col.account")), h("th", {}, t("keys.col.rate")), h("th", {}, t("keys.col.models")), h("th", { class: "right-align" }, t("common.actions")))
                ),
                h(
                  "tbody",
                  {},
                  ...keys.map((k) =>
                    h(
                      "tr",
                      {},
                      h("td", { class: "mono" }, k.key_masked),
                      h("td", {}, k.account),
                      h("td", { class: "mono" }, k.rate_per_min || "—"),
                      h("td", {}, (k.allow_models && k.allow_models.length) ? k.allow_models.join(", ") : h("span", { class: "muted" }, t("keys.allowAll"))),
                      h(
                        "td",
                        { class: "right-align" },
                        h(
                          "button",
                          {
                            class: "btn sm danger",
                            onClick: async () => {
                              if (!(await confirmDialog(t("keys.revokeConfirm")))) return;
                              try {
                                await api.deleteKey(k.key);
                                toast(t("toast.deleted"), "good");
                                reload();
                              } catch (e) {
                                toast(String(e.message || e), "bad");
                              }
                            },
                          },
                          t("keys.revoke")
                        )
                      )
                    )
                  )
                )
              )
            )
          )
    );
  }

  await reload();
  return root;
}
