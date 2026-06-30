// App bootstrap: login gate, sidebar, topbar, hash router, language + theme.
import { h, mount, toast } from "./ui.js";
import { t, getLang, setLang, onLangChange, LANGS } from "./i18n.js";
import { api, getToken, setToken, setUnauthorizedHandler } from "./api.js";
import { renderLogin } from "./views/login.js";
import { renderDashboard } from "./views/dashboard.js";
import { renderModels } from "./views/models.js";
import { renderRouting } from "./views/routing.js";
import { renderProtocols } from "./views/protocols.js";
import { renderKeys } from "./views/keys.js";
import { renderPlayground } from "./views/playground.js";
import { renderSettings } from "./views/settings.js";

const ROUTES = [
  { id: "dashboard", icon: "▤", view: renderDashboard },
  { id: "models", icon: "▦", view: renderModels },
  { id: "routing", icon: "⇄", view: renderRouting },
  { id: "protocols", icon: "⇆", view: renderProtocols },
  { id: "keys", icon: "🔑", view: renderKeys },
  { id: "playground", icon: "✦", view: renderPlayground },
  { id: "settings", icon: "⚙", view: renderSettings },
];

export const appState = { meta: null, health: null, version: "" };

function currentRoute() {
  const id = location.hash.replace(/^#\/?/, "") || "dashboard";
  return ROUTES.find((r) => r.id === id) || ROUTES[0];
}

const FAVICON =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="%233b82f6"/><stop offset=".5" stop-color="%238b5cf6"/><stop offset="1" stop-color="%2306b6d4"/></linearGradient></defs><rect width="32" height="32" rx="8" fill="url(%23g)"/><path d="M9 16h14M17 10l6 6-6 6" stroke="white" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>'
  );

function sidebar(activeId) {
  return h(
    "aside",
    { class: "sidebar", id: "sidebar" },
    h(
      "div",
      { class: "brand" },
      h("img", { src: FAVICON, alt: "" }),
      h("div", {}, h("b", {}, "allaigate"), h("div", { class: "sub" }, t("brand.sub")))
    ),
    ...ROUTES.map((r) =>
      h(
        "a",
        { class: "nav-item" + (r.id === activeId ? " active" : ""), href: "#/" + r.id },
        h("span", { class: "ico" }, r.icon),
        t("nav." + r.id)
      )
    ),
    h("div", { class: "nav-spacer" }),
    h("div", { class: "nav-foot" }, t("nav.foot", { v: appState.version || "—" }))
  );
}

function statusPills() {
  const h_ = appState.health;
  const gwOk = !!appState.meta;
  const routerOk = h_?.router?.reachable;
  return h(
    "div",
    { class: "flex" },
    h(
      "span",
      { class: "pill" },
      h("span", { class: "dot " + (gwOk ? "ok" : "bad") }),
      t("status.gateway") + ": " + (gwOk ? t("status.online") : t("status.offline"))
    ),
    h(
      "span",
      { class: "pill", title: h_?.router?.url || "" },
      h("span", { class: "dot " + (h_ ? (routerOk ? "ok" : "bad") : "warn") }),
      t("status.router") + ": " + (h_ ? (routerOk ? t("status.online") : t("status.offline")) : t("status.checking"))
    )
  );
}

function langSelect() {
  const sel = h(
    "select",
    {
      class: "lang-select",
      title: "Language",
      onChange: (e) => setLang(e.target.value),
    },
    ...LANGS.map((l) => h("option", { value: l.code, selected: l.code === getLang() }, l.label))
  );
  return sel;
}

function themeToggle() {
  const cur = document.documentElement.getAttribute("data-theme") || "dark";
  return h(
    "button",
    {
      class: "icon-btn",
      title: t("theme.toggle"),
      onClick: () => {
        const next = (document.documentElement.getAttribute("data-theme") || "dark") === "dark" ? "light" : "dark";
        document.documentElement.setAttribute("data-theme", next);
        localStorage.setItem("allaigate_theme", next);
        renderShell();
      },
    },
    cur === "dark" ? "☀" : "☾"
  );
}

function topbar(activeId) {
  return h(
    "header",
    { class: "topbar" },
    h("div", { class: "title" }, t("nav." + activeId)),
    h(
      "div",
      { class: "right" },
      statusPills(),
      themeToggle(),
      langSelect(),
      h(
        "button",
        {
          class: "icon-btn",
          title: t("logout"),
          onClick: () => {
            setToken("");
            boot();
          },
        },
        "⎋"
      )
    )
  );
}

async function renderView() {
  const route = currentRoute();
  const view = document.getElementById("view");
  if (!view) return;
  mount(view, h("div", { class: "empty" }, h("span", { class: "spinner" }), " ", t("common.loading")));
  try {
    const node = await route.view();
    mount(view, node);
  } catch (e) {
    if (String(e.message) === "unauthorized") return;
    mount(view, h("div", { class: "callout" }, String(e.message || e)));
  }
}

function renderShell() {
  const route = currentRoute();
  mount(
    document.getElementById("app"),
    h(
      "div",
      { class: "app" },
      sidebar(route.id),
      h("main", { class: "main" }, topbar(route.id), h("div", { class: "content", id: "view" }))
    )
  );
  renderView();
}

function showLogin() {
  mount(document.getElementById("app"), renderLogin(boot));
}

async function refreshHealth() {
  try {
    appState.health = await api.health();
  } catch {
    appState.health = null;
  }
  // re-render topbar only if shell present
  const route = currentRoute();
  const main = document.querySelector(".main");
  if (main) {
    const old = main.querySelector(".topbar");
    if (old) old.replaceWith(topbar(route.id));
  }
}

export async function boot() {
  // convenience: /admin?token=… logs in once, then is stripped from the URL
  const urlTok = new URLSearchParams(location.search).get("token");
  if (urlTok) {
    setToken(urlTok);
    history.replaceState({}, "", location.pathname + location.hash);
  }
  if (!getToken()) {
    showLogin();
    return;
  }
  try {
    appState.meta = await api.meta();
    appState.version = appState.meta.version || "";
  } catch (e) {
    if (String(e.message) === "unauthorized") {
      showLogin();
      return;
    }
    // gateway reachable but error — still show shell
  }
  if (!location.hash) location.hash = "#/dashboard";
  renderShell();
  refreshHealth();
}

setUnauthorizedHandler(() => {
  toast(t("login.failed"), "bad");
  setToken("");
  showLogin();
});
onLangChange(() => {
  if (document.querySelector(".app")) renderShell();
});
window.addEventListener("hashchange", () => {
  if (document.querySelector(".app")) renderShell();
});
setInterval(() => {
  if (document.querySelector(".app")) refreshHealth();
}, 20000);

boot();
