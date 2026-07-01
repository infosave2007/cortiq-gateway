# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **Latency benchmark** vs LiteLLM and Portkey: a reproducible harness (`bench/`,
  instant mock backend + Apache Bench) and `BENCHMARKS.md`. Measured gateway overhead:
  Cortiq ~57k req/s / p99 1 ms vs Portkey ~5.8k / p99 9 ms vs LiteLLM ~1.2k / p99 59 ms.
- **Accuracy benchmark**: task-type routing on natural-language prompts (`bench/accuracy.py`,
  `bench/tasks.jsonl`) — allaigate semantic router 100% (37/37) vs a keyword heuristic 32%
  (competitors have no semantic task router).

## [0.2.1] - 2026-06-30

### Added
- **OpenAI Models** (`GET /v1/models`): lists the pool + the virtual `cortiq-auto`.
- **OpenAI Completions (legacy)** (`POST /v1/completions`): non-streaming and streaming
  (`text_completion`), routed through the same pipeline.
- **Native passthrough** (`POST /route`): returns the routing decision (task, complexity,
  candidate models) without calling a model.

All inbound protocol adapters are now implemented.

## [0.2.0] - 2026-06-30

### Added
- **Streaming (SSE)** end-to-end: `POST /v1/chat/completions` with `"stream": true`
  forwards provider SSE chunks verbatim, with `X-Cortiq-*` routing headers and token
  usage tapped from the final chunk for statistics. Failover applies before the first
  byte. Live streaming output in the admin Playground (`/admin/api/test/stream`).
- Cross-platform Windows binaries in releases (`x86_64-pc-windows-msvc`) and a
  cross-platform CI test matrix (ubuntu/windows/macos); secure token RNG via `getrandom`.
- **Anthropic provider** (outbound): `POST /v1/messages` with `system` extraction,
  `x-api-key` + `anthropic-version` headers, and streaming that translates Anthropic
  SSE events into the unified OpenAI wire format.
- **Anthropic inbound** adapter: `POST /v1/messages` (non-streaming JSON and streaming
  SSE), so Claude-native clients can target the gateway.
- **Embeddings**: `POST /v1/embeddings` inbound + provider `embed()`; embedding models
  are now resolvable from the pool.
- **Semantic cache**: embedding-based cache that returns a stored answer for prompts
  semantically near a previous one (cosine threshold, TTL, ring buffer), skipping the
  model call. Partitioned by routing signature; hit-rate and cost saved are tracked and
  shown on the dashboard; configurable from Settings (`[cache]`).
- **MCP server** (`POST /mcp`, JSON-RPC 2.0): exposes routing to MCP-native orchestrators
  via two tools — `cortiq_route` (classify → decision) and `cortiq_chat` (route → answer).

## [0.1.0] - 2026-06-30

### Added
- **Embedded admin web console** at `/admin` — a build-free single-page app compiled
  into the binary via `rust-embed`. Manage models, routing, protocols, virtual keys
  and secrets from the browser; changes are hot-reloaded (no restart).
- **Multilingual UI** in 7 languages (en, ru, de, fr, es, zh, tr) with browser
  auto-detection, plus light/dark themes.
- **Admin API** under `/admin/api/*` (Bearer admin token): config, models (CRUD +
  probe), routing, protocols, settings, keys, secrets, stats, requests, and a
  playground `test` endpoint.
- **Hot config reload** — config-derived state lives behind `arc-swap`; writes
  validate → atomically rewrite the TOML → swap, with no restart.
- **Request statistics** — in-memory aggregates, per-minute time buckets, and a ring
  buffer of recent requests, appended to a JSONL file and replayed on startup.
- **Prometheus `GET /metrics`** endpoint.
- **Secret store** (`config/secrets.toml`, mode `0600`) — provider keys can be set
  from the UI without writing them into `gateway.toml`; values are never returned by
  the API.
- English documentation as the primary language, with Russian translations
  (`README.ru.md`, `docs/*.ru.md`).

### Changed
- The Anthropic provider now returns a clear error instead of panicking when called
  (the adapter is still planned for v0.2).

## [0.0.1]
- Initial skeleton: OpenAI Chat Completions inbound adapter, OpenAI-compatible
  outbound provider, routing table with fallback and graceful degradation, and
  cost/token accounting.
