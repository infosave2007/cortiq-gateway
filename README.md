# Cortiq Gateway

**English** · [Русский](https://github.com/infosave2007/cortiq-gateway/blob/master/README.ru.md)

[![Crates.io](https://img.shields.io/crates/v/cortiq-gateway.svg)](https://crates.io/crates/cortiq-gateway)
[![CI](https://github.com/infosave2007/cortiq-gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/infosave2007/cortiq-gateway/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://github.com/infosave2007/cortiq-gateway/blob/master/LICENSE)
![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)

**A universal LLM gateway with intelligent routing.**
One OpenAI-compatible endpoint → automatic model selection from your pool
(cheap local + expensive hosted) based on the task type and complexity decided by
[allaigate / cortiq-router](https://api.allaigate.com).

> In your agent/SDK you only change `base_url` — and you get "smart" routing across
> models. No model-selection logic on the client side.

```
┌─────────────┐   OpenAI / Anthropic / MCP    ┌──────────────────┐
│   Agent /   │ ─────────────────────────────▶│  Cortiq Gateway  │
│  developer  │   (standard protocol)          │  (this project)  │
└─────────────┘ ◀─────────────────────────────└──────────────────┘
                       response + metadata        │          │
                                                  │ /v1/route│ call LLM
                                                  ▼          ▼
                                         ┌──────────────┐  ┌─────────────────────┐
                                         │ cortiq-router│  │  Model pool         │
                                         │ (task type,  │  │  • local llama.cpp  │
                                         │  complexity) │  │  • ollama / vLLM    │
                                         └──────────────┘  │  • OpenAI / Claude  │
                                                           └─────────────────────┘
```

---

## ✨ Highlights

- **Drop-in OpenAI API + streaming.** Point any OpenAI client at the gateway and send `model: "cortiq-auto"`; `stream: true` is fully supported (SSE).
- **Multi-protocol.** OpenAI Chat & Embeddings, **Anthropic Messages** (in/out, streaming), and an **MCP** server (`POST /mcp`) exposing routing as tools for agent orchestrators.
- **Intelligent routing.** `complexity.tier` → ordered model pool (`low → local`, `high → cloud`), with fallback and graceful degradation when the router is down.
- **Semantic cache.** Optional embedding-based cache returns a stored answer for prompts semantically near a previous one — skipping the model call (hit-rate & savings on the dashboard).
- **Built-in multilingual admin console.** Manage models, routing, protocols, keys and secrets from a web UI — **no hand-editing TOML, no restart** (changes are hot-reloaded). Available in **7 languages** (en, ru, de, fr, es, zh, tr) with light/dark themes.
- **Live analytics.** Per-request stats (tokens, cost, latency, success rate, failovers), time-series charts, breakdowns by model/tier/task, and a Prometheus `GET /metrics` endpoint.
- **Playground.** Send a prompt through the live pipeline and inspect the routing decision.
- **Single self-contained binary.** The SPA is embedded into the Rust binary — nothing extra to deploy.
- **Secrets stay in the gateway.** Agents hold only a virtual gateway key; provider keys live in the gateway and never leak to clients.

---

## 📦 Install

```bash
# from crates.io
cargo install cortiq-gateway

# or build from source
git clone https://github.com/infosave2007/cortiq-gateway
cd cortiq-gateway
cargo build --release   # ./target/release/cortiq-gateway
```

---

## 🖥️ Admin console

![Dashboard](https://raw.githubusercontent.com/infosave2007/cortiq-gateway/master/docs/screenshots/dashboard.png)

The gateway ships with an embedded web console at **`/admin`**:

| Models | Routing |
|---|---|
| ![Models](https://raw.githubusercontent.com/infosave2007/cortiq-gateway/master/docs/screenshots/models.png) | ![Routing](https://raw.githubusercontent.com/infosave2007/cortiq-gateway/master/docs/screenshots/routing.png) |
| add/edit/probe models, manage provider keys | visual tier editor (ordered, reorderable) |

| Playground |
|---|
| ![Playground](https://raw.githubusercontent.com/infosave2007/cortiq-gateway/master/docs/screenshots/playground.png) |
| test the live pipeline and see the routing decision |

```bash
cargo run --release -- --config config/gateway.toml --admin-token <YOUR_TOKEN>
# open  http://localhost:9000/admin?token=<YOUR_TOKEN>
```

If `--admin-token` / `[admin].token_env` are not set, a token is generated at startup
and printed to the log. All `/admin/api/*` endpoints require a Bearer admin token;
secret values are never returned by the API (only their presence: `store` / `env` / `missing`).

---

## 🚀 Quick start

```bash
# 1. start a router (the hosted allaigate router, or a local cortiq-router)
#    it listens e.g. on http://localhost:8080 (or https://api.allaigate.com)

# 2. describe your model pool
cp config/gateway.example.toml config/gateway.toml
$EDITOR config/gateway.toml

# 3. run the gateway
cortiq-gateway --config config/gateway.toml
# Gateway listens on 0.0.0.0:9000 and serves the admin console at /admin
```

Now any OpenAI client works through the gateway:

```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:9000/v1", api_key="sk-gw-...")

resp = client.chat.completions.create(
    model="cortiq-auto",                       # ← magic model = "choose for me"
    messages=[{"role": "user", "content": "Solve x^2 - 5x + 6 = 0"}],
)
print(resp.choices[0].message.content)
# response headers show what the gateway picked:
#   X-Cortiq-Task-Label: math
#   X-Cortiq-Complexity-Tier: low
#   X-Cortiq-Selected-Model: local-qwen
```

`model: "cortiq-auto"` enables routing. Any **real** model name from the config
(`"gpt-4o-mini"`, `"local-qwen"`) is a direct passthrough, without routing.

> **Using the hosted allaigate router?** Set `url = "https://138.226.222.209"`,
> `verify_tls = false`, `taxonomy_id = "data-assistant"`, and a `cortiq_…` key in
> `CORTIQ_ROUTER_KEY`. On hard prompts the router escalates to an oracle (~10 s), so
> set `timeout_ms = 12000+` — otherwise the gateway gracefully degrades to the default model.

---

## Request flow

1. Client → `POST /v1/chat/completions` (or Anthropic/MCP) with `model: "cortiq-auto"`.
2. The gateway extracts the **text to route on** (strategy is configurable, see [docs/ROUTING.md](https://github.com/infosave2007/cortiq-gateway/blob/master/docs/ROUTING.md)).
3. Gateway → `cortiq-router /v1/route` → gets `task_label` + `complexity.tier`.
4. Gateway selects a model from the pool via the routing table (with a fallback order).
5. Gateway → the selected model's provider (translating the protocol if needed), returns the response.
6. Gateway attaches routing metadata in headers/`usage` and records cost & statistics.

Details — [docs/ARCHITECTURE.md](https://github.com/infosave2007/cortiq-gateway/blob/master/docs/ARCHITECTURE.md).

---

## Supported protocols (inbound)

Each adapter is toggled in the config — take only what you need.

| Protocol | Endpoint | Status |
|---|---|---|
| OpenAI Chat Completions | `POST /v1/chat/completions` | ✅ implemented (+ streaming) |
| OpenAI Embeddings | `POST /v1/embeddings` | ✅ implemented |
| Anthropic Messages | `POST /v1/messages` | ✅ implemented (+ streaming) |
| MCP (Model Context Protocol) | `POST /mcp` (JSON-RPC) | ✅ implemented |
| OpenAI Completions (legacy) | `POST /v1/completions` | planned |
| OpenAI Models | `GET /v1/models` | planned |
| Native passthrough | `POST /route` | planned |

## Supported providers (outbound)

| Provider | Covers |
|---|---|
| `openai` (OpenAI-compatible) | OpenAI, OpenRouter, Together, Groq, **vLLM, llama.cpp server, LM Studio, Ollama (`/v1`)** |
| `anthropic` | Claude Messages API (+ streaming) |
| `ollama` | native Ollama API |
| `http` | arbitrary HTTP endpoint (custom adapter) |

Most local servers already expose an OpenAI-compatible API — so the `openai`
adapter makes the gateway **universal** with almost no effort.

---

## Configuration

```toml
listen = "0.0.0.0:9000"

[router]
url = "http://localhost:8080"
api_key_env = "CORTIQ_ROUTER_KEY"

[[models]]
id        = "local-qwen"
provider  = "openai"
base_url  = "http://localhost:8000/v1"
model     = "qwen2.5-7b-instruct"
cost_tier = "cheap"

[[models]]
id          = "claude-opus"
provider    = "anthropic"
base_url    = "https://api.anthropic.com"
model       = "claude-opus-4"
api_key_env = "ANTHROPIC_API_KEY"
cost_tier   = "expensive"

[routing]
low     = ["local-qwen"]
high    = ["claude-opus", "local-qwen"]
default = "local-qwen"
```

Full example — [config/gateway.example.toml](https://github.com/infosave2007/cortiq-gateway/blob/master/config/gateway.example.toml).
Routing & cost-aware selection — [docs/ROUTING.md](https://github.com/infosave2007/cortiq-gateway/blob/master/docs/ROUTING.md).
Everything in the config is also editable from the admin console at runtime.

---

## Cross-standards (like grown-up APIs)

- **Auth:** `Authorization: Bearer sk-gw-...` (virtual gateway keys).
- **Errors:** OpenAI-compatible `{"error": {...}}` envelope — parsed natively by SDKs.
- **Routing metadata:** `X-Cortiq-*` headers on every response.
- **Cost accounting:** token usage + USD cost and the selected model.
- **Observability:** `GET /metrics` (Prometheus), `GET /healthz`, `GET /readyz`.

Details — [docs/PROTOCOLS.md](https://github.com/infosave2007/cortiq-gateway/blob/master/docs/PROTOCOLS.md).

---

## Build & test

```bash
cargo build --release      # single self-contained binary (SPA embedded)
cargo test                 # config round-trip, routing validation
```

---

## Status

Implemented: OpenAI Chat Completions **with SSE streaming**, **Anthropic** provider +
inbound `/v1/messages` (streaming), **embeddings**, a **semantic cache**, **MCP** server,
routing with fallback & graceful degradation, cost/token accounting, the **embedded
multilingual admin console with hot config reload**, statistics and Prometheus
`GET /metrics`. Planned: OpenAI Completions/Models, native passthrough, per-account
routing, feedback loop. See the roadmap in
[docs/ARCHITECTURE.md](https://github.com/infosave2007/cortiq-gateway/blob/master/docs/ARCHITECTURE.md).
Contributions welcome — see [CONTRIBUTING.md](https://github.com/infosave2007/cortiq-gateway/blob/master/CONTRIBUTING.md).

## License

Apache-2.0 — see [LICENSE](https://github.com/infosave2007/cortiq-gateway/blob/master/LICENSE).
