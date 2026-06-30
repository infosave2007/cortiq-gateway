# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

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
