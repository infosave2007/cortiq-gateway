// i18n engine. English is the source of truth; other languages override per key
// and fall back to English for any missing key.
import en from "./i18n/en.js";
import ru from "./i18n/ru.js";
import de from "./i18n/de.js";
import fr from "./i18n/fr.js";
import es from "./i18n/es.js";
import zh from "./i18n/zh.js";
import tr from "./i18n/tr.js";

const DICTS = { en, ru, de, fr, es, zh, tr };
export const LANGS = [
  { code: "en", label: "English" },
  { code: "ru", label: "Русский" },
  { code: "de", label: "Deutsch" },
  { code: "fr", label: "Français" },
  { code: "es", label: "Español" },
  { code: "zh", label: "中文" },
  { code: "tr", label: "Türkçe" },
];

const listeners = new Set();

function detect() {
  const saved = localStorage.getItem("allaigate_lang");
  if (saved && DICTS[saved]) return saved;
  const url = new URLSearchParams(location.search).get("lang");
  if (url && DICTS[url]) return url;
  const nav = (navigator.language || "en").slice(0, 2).toLowerCase();
  return DICTS[nav] ? nav : "en";
}

let lang = detect();

export function getLang() {
  return lang;
}

export function setLang(l) {
  lang = DICTS[l] ? l : "en";
  localStorage.setItem("allaigate_lang", lang);
  document.documentElement.setAttribute("lang", lang);
  listeners.forEach((fn) => fn(lang));
}

export function onLangChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function t(key, vars) {
  const d = DICTS[lang] || en;
  let s = d[key] ?? en[key] ?? key;
  if (vars) for (const k of Object.keys(vars)) s = s.split("{" + k + "}").join(vars[k]);
  return s;
}
