// Thin client over the gateway's /admin/api surface. Adds the Bearer admin token,
// parses JSON, and throws Error(message) on non-2xx so views can try/catch.
// A 401 sets an "unauthorized" flag the app uses to show the login screen.

const BASE = "/admin/api";
const TOKEN_KEY = "allaigate_token";

let onUnauthorized = null;
export function setUnauthorizedHandler(fn) {
  onUnauthorized = fn;
}

export function getToken() {
  return localStorage.getItem(TOKEN_KEY) || "";
}
export function setToken(tok) {
  if (tok) localStorage.setItem(TOKEN_KEY, tok);
  else localStorage.removeItem(TOKEN_KEY);
}

async function j(method, path, body) {
  const opt = { method, headers: { Authorization: "Bearer " + getToken() } };
  if (body !== undefined) {
    opt.headers["content-type"] = "application/json";
    opt.body = JSON.stringify(body);
  }
  const r = await fetch(BASE + path, opt);
  if (r.status === 401) {
    if (onUnauthorized) onUnauthorized();
    throw new Error("unauthorized");
  }
  const text = await r.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!r.ok) {
    const msg =
      (data && data.error && (data.error.message || data.error)) ||
      (data && (data.detail || data.message)) ||
      r.statusText;
    throw new Error(typeof msg === "string" ? msg : JSON.stringify(msg));
  }
  return data;
}

export const api = {
  // meta / health
  meta: () => j("GET", "/meta"),
  health: () => j("GET", "/health"),

  // config
  getConfig: () => j("GET", "/config"),
  putConfig: (b) => j("PUT", "/config", b),

  // models
  listModels: () => j("GET", "/models"),
  createModel: (b) => j("POST", "/models", b),
  updateModel: (id, b) => j("PUT", "/models/" + encodeURIComponent(id), b),
  deleteModel: (id) => j("DELETE", "/models/" + encodeURIComponent(id)),
  probeModel: (id) => j("POST", "/models/" + encodeURIComponent(id) + "/probe"),

  // routing
  getRouting: () => j("GET", "/routing"),
  putRouting: (b) => j("PUT", "/routing", b),

  // protocols
  getProtocols: () => j("GET", "/protocols"),
  putProtocols: (b) => j("PUT", "/protocols", b),

  // settings
  getSettings: () => j("GET", "/settings"),
  putSettings: (b) => j("PUT", "/settings", b),

  // keys
  listKeys: () => j("GET", "/keys"),
  createKey: (b) => j("POST", "/keys", b),
  deleteKey: (k) => j("DELETE", "/keys/" + encodeURIComponent(k)),

  // secrets
  listSecrets: () => j("GET", "/secrets"),
  setSecret: (name, value) => j("PUT", "/secrets", { name, value }),
  clearSecret: (name) => j("DELETE", "/secrets?name=" + encodeURIComponent(name)),

  // stats
  stats: (range, groupby) => j("GET", `/stats?range=${range}&groupby=${groupby}`),
  requests: (limit = 50, offset = 0) => j("GET", `/requests?limit=${limit}&offset=${offset}`),

  // playground
  test: (b) => j("POST", "/test", b),
};
