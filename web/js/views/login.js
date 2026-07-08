// Login screen — admin token entry. Shown when there is no token or on 401.
import { h } from "../ui.js";
import { t } from "../i18n.js";
import { setToken } from "../api.js";
import { SITE_URL } from "../app.js";

export function renderLogin(onOk) {
  const input = h("input", {
    type: "password",
    placeholder: t("login.tokenPlaceholder"),
    autofocus: true,
  });
  const submit = () => {
    const val = input.value.trim();
    if (!val) return;
    setToken(val);
    onOk();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") submit();
  });
  return h(
    "div",
    { class: "login-wrap" },
    h(
      "div",
      { class: "login-card" },
      h(
        "div",
        { class: "brand" },
        h("b", { style: "font-size:20px" }, "allaigate"),
        h("span", { class: "sub" }, t("brand.sub"))
      ),
      h("h2", {}, t("login.title")),
      h("p", { class: "sub" }, t("login.subtitle")),
      h("label", { class: "field" }, h("span", {}, t("login.token")), input),
      h("button", { class: "btn primary", style: "width:100%", onClick: submit }, t("login.submit")),
      h("p", { class: "hint", style: "margin-top:16px" }, t("login.hint")),
      h(
        "p",
        { class: "hint", style: "margin-top:8px" },
        h("a", { href: SITE_URL, target: "_blank", rel: "noopener" }, t("nav.billing") + " · api.allaigate.com ↗")
      )
    )
  );
}
