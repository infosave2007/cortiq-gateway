// Routing — tier editor (ordered model chips), default model, and policy.
import { h, mount, toast } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { appState } from "../app.js";

export async function renderRouting() {
  const meta = appState.meta || (await api.meta());
  const [routing, modelsData] = await Promise.all([api.getRouting(), api.listModels()]);
  const ids = (modelsData.models || []).map((m) => m.id);

  // local editable state — ensure standard tiers exist for a friendly editor
  const tiers = Object.assign({ low: [], medium: [], high: [] }, routing.tiers || {});
  const st = {
    tiers,
    default: routing.default || ids[0] || "",
    policy: routing.policy || { mode: "fixed_table", min_class: {} },
  };

  const root = h("div");

  function moveChip(tier, i, dir) {
    const arr = st.tiers[tier];
    const ni = i + dir;
    if (ni < 0 || ni >= arr.length) return;
    [arr[i], arr[ni]] = [arr[ni], arr[i]];
    render();
  }
  function removeChip(tier, i) {
    st.tiers[tier].splice(i, 1);
    render();
  }
  function addChip(tier, id) {
    if (id && !st.tiers[tier].includes(id)) st.tiers[tier].push(id);
    render();
  }

  function tierRow(tier) {
    const arr = st.tiers[tier];
    const avail = ids.filter((id) => !arr.includes(id));
    const addSel = h(
      "select",
      {
        style: "width:auto",
        onChange: (e) => {
          addChip(tier, e.target.value);
        },
      },
      h("option", { value: "" }, t("routing.addModel")),
      ...avail.map((id) => h("option", { value: id }, id))
    );
    return h(
      "div",
      { class: "tierrow" },
      h("div", { class: "tname" }, h("span", { class: "badge tier-" + tier }, tier)),
      h(
        "div",
        { class: "chips" },
        ...arr.map((id, i) =>
          h(
            "div",
            { class: "chip" },
            h("span", { class: "ord" }, i + 1),
            id,
            h("span", { class: "arrow", title: "←", onClick: () => moveChip(tier, i, -1) }, "◄"),
            h("span", { class: "arrow", title: "→", onClick: () => moveChip(tier, i, 1) }, "►"),
            h("span", { class: "x", onClick: () => removeChip(tier, i) }, "×")
          )
        ),
        avail.length ? addSel : null
      )
    );
  }

  function policyCard() {
    const modeSel = h(
      "select",
      { onChange: (e) => { st.policy.mode = e.target.value; render(); } },
      h("option", { value: "fixed_table", selected: st.policy.mode === "fixed_table" }, t("routing.policy.fixed")),
      h("option", { value: "cost_aware", selected: st.policy.mode === "cost_aware" }, t("routing.policy.costAware"))
    );
    const children = [
      h("label", { class: "field" }, h("span", {}, t("routing.policy.mode")), modeSel),
    ];
    if (st.policy.mode === "cost_aware") {
      const maxIn = h("input", {
        type: "number",
        step: "0.01",
        value: st.policy.max_cost_usd_per_request ?? "",
        placeholder: "0.50",
        onChange: (e) => { st.policy.max_cost_usd_per_request = e.target.value === "" ? null : parseFloat(e.target.value); },
      });
      children.push(h("label", { class: "field" }, h("span", {}, t("routing.policy.maxCost")), maxIn));
      st.policy.min_class = st.policy.min_class || {};
      const rows = Object.keys(st.tiers).map((tier) => {
        const sel = h(
          "select",
          { onChange: (e) => { st.policy.min_class[tier] = e.target.value; } },
          h("option", { value: "" }, "—"),
          ...(meta.cost_tiers || []).map((c) => h("option", { value: c, selected: st.policy.min_class[tier] === c }, c))
        );
        return h("label", { class: "field" }, h("span", {}, t("routing.policy.minClass") + " · " + tier), sel);
      });
      children.push(h("div", { class: "row" }, ...rows));
    }
    return h("div", { class: "card" }, h("div", { class: "card-head" }, h("h3", {}, t("routing.policy"))), ...children);
  }

  function render() {
    const defSel = h(
      "select",
      { onChange: (e) => { st.default = e.target.value; } },
      ...ids.map((id) => h("option", { value: id, selected: id === st.default }, id))
    );
    const saveBtn = h("button", { class: "btn primary" }, t("common.save"));
    saveBtn.addEventListener("click", async () => {
      saveBtn.disabled = true;
      try {
        await api.putRouting({ tiers: st.tiers, default: st.default, policy: st.policy });
        toast(t("toast.saved"), "good");
      } catch (e) {
        toast(String(e.message || e), "bad");
      }
      saveBtn.disabled = false;
    });

    mount(
      root,
      h(
        "div",
        { class: "page-head" },
        h(
          "div",
          { class: "flex" },
          h("div", { class: "grow" }, h("h2", {}, t("routing.title")), h("p", {}, t("routing.subtitle"))),
          saveBtn
        )
      ),
      ids.length === 0 ? h("div", { class: "callout" }, t("routing.empty")) : null,
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("routing.tiers")), h("span", { class: "card-head sub" }, t("routing.tierHint"))),
        ...Object.keys(st.tiers).map(tierRow)
      ),
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("routing.default"))),
        h("label", { class: "field" }, h("span", {}, t("routing.defaultHint")), defSel)
      ),
      policyCard()
    );
  }

  render();
  return root;
}
