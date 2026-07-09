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
  probeRouter: () => j("POST", "/router/probe"),

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

  // shadow / self-warming loop
  shadow: () => j("GET", "/shadow"),

  // CMF runtime (the `cortiq` CLI that runs the .cmf format)
  cmfStatus: () => j("GET", "/cmf"),
  cmfInstall: () => j("POST", "/cmf/install"),
  cmfPortCheck: (port) => j("GET", "/cmf/port?port=" + encodeURIComponent(port)),
  cmfFiles: () => j("GET", "/cmf/files"),

  // CMF model factory (HuggingFace → local .cmf)
  hfSearch: (q, limit = 24) =>
    j("GET", `/hf/search?q=${encodeURIComponent(q || "")}&limit=${limit}`),
  startImport: (b) => j("POST", "/import", b),
  listImports: () => j("GET", "/import"),
  importStatus: (job) => j("GET", "/import/" + encodeURIComponent(job)),
  cancelImport: (job) =>
    j("POST", "/import/" + encodeURIComponent(job) + "/cancel"),
  deleteImport: (job) => j("DELETE", "/import/" + encodeURIComponent(job)),
  registerImport: (job) =>
    j("POST", "/import/" + encodeURIComponent(job) + "/register"),
};

// Streaming playground: POST /test/stream, calls onDelta(text) per content chunk,
// returns the routing decision read from X-Cortiq-* response headers.
export async function testStream(body, onDelta) {
  const r = await fetch(BASE + "/test/stream", {
    method: "POST",
    headers: { Authorization: "Bearer " + getToken(), "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (r.status === 401) {
    if (onUnauthorized) onUnauthorized();
    throw new Error("unauthorized");
  }
  if (!r.ok || !r.body) {
    throw new Error((await r.text()) || r.statusText);
  }
  const reader = r.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += dec.decode(value, { stream: true });
    let idx;
    while ((idx = buf.indexOf("\n\n")) >= 0) {
      const line = buf.slice(0, idx).split("\n").find((l) => l.startsWith("data:"));
      buf = buf.slice(idx + 2);
      if (!line) continue;
      const data = line.slice(5).trim();
      if (!data || data === "[DONE]") continue;
      try {
        const d = JSON.parse(data).choices?.[0]?.delta?.content;
        if (d) onDelta(d);
      } catch {
        /* ignore malformed chunk */
      }
    }
  }
  const h = r.headers;
  return {
    task_label: h.get("x-cortiq-task-label") || "",
    tier: h.get("x-cortiq-complexity-tier") || "",
    score: parseFloat(h.get("x-cortiq-complexity-score") || "0"),
    selected_model: h.get("x-cortiq-selected-model") || "",
    route_source: h.get("x-cortiq-route-source") || "",
    cost_usd: parseFloat(h.get("x-cortiq-cost-usd") || "0"),
  };
}
