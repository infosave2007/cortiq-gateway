// Dashboard — KPIs, time-series charts, breakdown, recent requests, health.
import { h, mount, money, num, ms, pct, timeAgo } from "../ui.js";
import { t } from "../i18n.js";
import { api } from "../api.js";
import { lineChart, barList } from "../charts.js";

const RANGES = ["1h", "24h", "7d"];
const GROUPS = ["model", "tier", "label", "account"];

function kpi(big, lbl) {
  return h("div", { class: "kpi" }, h("div", { class: "big" }, big), h("div", { class: "lbl" }, lbl));
}

function seg(options, current, onPick, labelFn) {
  return h(
    "div",
    { class: "seg" },
    ...options.map((o) =>
      h("button", { class: o === current ? "active" : "", onClick: () => onPick(o) }, labelFn ? labelFn(o) : o)
    )
  );
}

export async function renderDashboard() {
  let range = "24h";
  let groupby = "model";
  const root = h("div");

  async function load() {
    const rangeSecs = range;
    let stats, recent, health;
    try {
      [stats, recent, health] = await Promise.all([
        api.stats(rangeSecs, groupby),
        api.requests(40, 0),
        api.health().catch(() => null),
      ]);
    } catch (e) {
      mount(root, h("div", { class: "callout" }, String(e.message || e)));
      return;
    }
    const tot = stats.totals || {};
    const series = stats.series || [];
    const breakdown = (stats.breakdown || []).map((b) => ({ key: b.key || t("common.none"), value: b.requests }));

    mount(
      root,
      h(
        "div",
        { class: "page-head" },
        h(
          "div",
          { class: "flex" },
          h("div", { class: "grow" }, h("h2", {}, t("dash.title")), h("p", {}, t("dash.subtitle"))),
          seg(RANGES, range, (r) => {
            range = r;
            load();
          }, (r) => t("dash.range." + r))
        )
      ),

      // KPIs
      h(
        "div",
        { class: "kpis" },
        kpi(num(tot.requests), t("dash.kpi.requests")),
        kpi(num(tot.total_tokens), t("dash.kpi.tokens")),
        kpi(money(tot.cost_usd), t("dash.kpi.cost")),
        kpi(ms(tot.avg_latency_ms), t("dash.kpi.latency")),
        kpi(pct(tot.success_rate), t("dash.kpi.success")),
        kpi(num(tot.failovers), t("dash.kpi.failovers"))
      ),

      h(
        "div",
        { class: "grid cols-2" },
        h(
          "div",
          { class: "card" },
          h("div", { class: "card-head" }, h("h3", {}, t("dash.series.title"))),
          series.length < 2 ? h("div", { class: "empty" }, t("dash.series.empty")) : lineChart(series, "requests")
        ),
        h(
          "div",
          { class: "card" },
          h("div", { class: "card-head" }, h("h3", {}, t("dash.series.cost"))),
          series.length < 2 ? h("div", { class: "empty" }, t("dash.series.empty")) : lineChart(series, "cost_usd")
        )
      ),

      // breakdown
      h(
        "div",
        { class: "card" },
        h(
          "div",
          { class: "card-head" },
          h("h3", {}, t("dash.breakdown.title")),
          h(
            "div",
            { class: "right" },
            seg(GROUPS, groupby, (g) => {
              groupby = g;
              load();
            }, (g) => t("dash.groupby." + g))
          )
        ),
        barList(breakdown, num)
      ),

      // recent
      h(
        "div",
        { class: "card" },
        h("div", { class: "card-head" }, h("h3", {}, t("dash.recent.title"))),
        recentTable(recent.requests || [])
      ),

      health ? healthCard(health) : null
    );
  }

  await load();
  return root;
}

function recentTable(rows) {
  if (!rows.length) return h("div", { class: "empty" }, t("dash.recent.empty"));
  return h(
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
          h("th", {}, t("dash.col.time")),
          h("th", {}, t("dash.col.model")),
          h("th", {}, t("dash.col.tier")),
          h("th", {}, t("dash.col.task")),
          h("th", {}, t("dash.col.tokens")),
          h("th", {}, t("dash.col.cost")),
          h("th", {}, t("dash.col.latency")),
          h("th", {}, t("dash.col.outcome"))
        )
      ),
      h(
        "tbody",
        {},
        ...rows.map((r) =>
          h(
            "tr",
            {},
            h("td", { class: "mono" }, timeAgo(r.ts)),
            h("td", {}, r.model_id || "—", r.failover ? h("span", { class: "badge warn", style: "margin-left:6px" }, "fo") : null),
            h("td", {}, r.tier ? h("span", { class: "badge tier-" + r.tier }, r.tier) : "—"),
            h("td", {}, r.task_label || "—"),
            h("td", { class: "mono" }, `${r.prompt_tokens}/${r.completion_tokens}`),
            h("td", { class: "mono" }, money(r.cost_usd)),
            h("td", { class: "mono" }, ms(r.latency_ms)),
            h("td", {}, h("span", { class: "badge " + (r.outcome === "ok" ? "ok" : "error") }, r.outcome))
          )
        )
      )
    )
  );
}

function healthCard(hh) {
  return h(
    "div",
    { class: "card" },
    h("div", { class: "card-head" }, h("h3", {}, t("dash.health.title"))),
    h(
      "div",
      { class: "flex wrap", style: "margin-bottom:12px" },
      h(
        "span",
        { class: "badge " + (hh.router?.reachable ? "ok" : "bad"), title: hh.router?.url || "" },
        t("status.router") + ": " + (hh.router?.reachable ? t("dash.health.reachable") : t("dash.health.unreachable"))
      )
    ),
    h(
      "div",
      { class: "table-wrap" },
      h(
        "table",
        {},
        h("thead", {}, h("tr", {}, h("th", {}, t("models.col.id")), h("th", {}, t("models.col.provider")), h("th", {}, t("models.col.model")), h("th", {}, t("models.col.key")))),
        h(
          "tbody",
          {},
          ...(hh.models || []).map((m) =>
            h(
              "tr",
              {},
              h("td", { class: "mono" }, m.id),
              h("td", {}, m.provider),
              h("td", { class: "mono" }, m.model),
              h("td", {}, h("span", { class: "badge " + keyBadge(m.key_source) }, m.key_source))
            )
          )
        )
      )
    )
  );
}

function keyBadge(src) {
  if (src === "store") return "store";
  if (src === "env") return "env";
  if (src === "missing") return "bad";
  return "";
}
