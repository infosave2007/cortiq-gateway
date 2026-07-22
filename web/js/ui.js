// Tiny DOM helpers — no framework. `h` builds elements; `mount` swaps children.
import { t } from "./i18n.js";

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "html") el.innerHTML = v;
    else if (k === "dataset") Object.assign(el.dataset, v);
    else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.appendChild(typeof c === "object" ? c : document.createTextNode(String(c)));
  }
  return el;
}

export function clear(node) {
  while (node && node.firstChild) node.removeChild(node.firstChild);
}

export function opt(v, label, sel) {
  return h("option", { value: v, selected: sel ? true : null }, label || v);
}

export function mount(node, ...children) {
  clear(node);
  for (const c of children.flat())
    if (c != null && c !== false)
      node.appendChild(typeof c === "object" ? c : document.createTextNode(String(c)));
  return node;
}

export function toast(msg, kind = "info", ms = 3200) {
  const box = document.getElementById("toasts");
  if (!box) return;
  const el = h("div", { class: `toast ${kind}` }, msg);
  box.appendChild(el);
  setTimeout(() => {
    el.style.opacity = "0";
    el.style.transition = "opacity .2s";
    setTimeout(() => el.remove(), 220);
  }, ms);
}

// Modal dialog. `body` is a DOM node; `onSave` returns a Promise; returns nothing.
export function modal(title, body, onSave, saveLabel) {
  const backdrop = h("div", { class: "modal-backdrop" });
  const onKeydown = (e) => {
    if (e.key === "Escape") close();
  };
  const close = () => {
    backdrop.remove();
    document.body.classList.remove("modal-open");
    document.removeEventListener("keydown", onKeydown);
  };
  backdrop.addEventListener("click", (e) => {
    if (e.target === backdrop) close();
  });
  const saveBtn = h("button", { class: "btn primary" }, saveLabel || t("common.save"));
  saveBtn.addEventListener("click", async () => {
    saveBtn.disabled = true;
    try {
      const keep = await onSave();
      if (keep !== false) close();
    } finally {
      saveBtn.disabled = false;
    }
  });
  const card = h(
    "div",
    { class: "modal" },
    h("h3", {}, title),
    body,
    h(
      "div",
      { class: "modal-foot" },
      h("button", { class: "btn ghost", onClick: close }, t("common.cancel")),
      onSave ? saveBtn : null
    )
  );
  backdrop.appendChild(card);
  document.body.classList.add("modal-open");
  document.addEventListener("keydown", onKeydown);
  document.body.appendChild(backdrop);
  return { close };
}

export function confirmDialog(message) {
  return Promise.resolve(window.confirm(message));
}

// number formatting
export function money(n) {
  if (n == null) return "—";
  if (n === 0) return "$0";
  if (Math.abs(n) < 0.01) return "$" + n.toFixed(6);
  return "$" + n.toFixed(4);
}
export function ms(n) {
  return n == null ? "—" : `${Math.round(Number(n))} ms`;
}
export function num(n) {
  if (n == null) return "—";
  return Number(n).toLocaleString();
}
export function pct(n) {
  if (n == null) return "—";
  return (n * 100).toFixed(1) + "%";
}
export function timeAgo(unixSecs) {
  const d = new Date(unixSecs * 1000);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}
