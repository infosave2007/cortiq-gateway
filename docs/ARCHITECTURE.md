*Read this in [Русский](ARCHITECTURE.ru.md).*

# Cortiq Gateway Architecture

This document describes the internal design of the gateway, request flow, abstractions, and
roadmap. Related documents: [PROTOCOLS.md](PROTOCOLS.md) (protocol contracts),
[ROUTING.md](ROUTING.md) (model selection and cost model).

---

## 1. Purpose and Scope

**Cortiq Gateway** is an L7 proxy that:

1. accepts requests in one of the **standard protocols** (OpenAI, Anthropic, MCP);
2. asks **cortiq-router** what type of task it is and how complex;
3. selects a specific model from the **provider pool** (local + cloud);
4. calls the provider, **translating the protocol** when necessary;
5. returns the response to the client (streaming and non-streaming), adding routing metadata;
6. tracks cost and (optionally) closes the feedback loop back to the router.

What the gateway does **not** do: it does not store or train models (that is the router's
job), and it does not implement the LLMs themselves (that is the providers' job). The gateway
is thin, stateless (except for idempotency caches and rate-limit state), and horizontally
scalable.

---

## 2. Component Diagram

```
                         ┌───────────────────────────────────────────────┐
                         │                Cortiq Gateway                  │
                         │                                                │
  client (any protocol)  │   ┌──────────────┐   ┌───────────────────┐    │
 ───────────────────────▶│──▶│  Inbound     │──▶│  Core pipeline    │    │
                         │   │  Protocol    │   │                   │    │
                         │   │  Adapters    │   │  1. extract text  │    │
                         │   │ (openai/     │   │  2. route()       │────┼──▶ cortiq-router
                         │   │  anthropic/  │   │  3. select model  │    │     /v1/route
                         │   │  mcp)        │   │  4. translate     │    │
                         │   └──────────────┘   │  5. call provider │────┼──▶ Provider pool
                         │                      │  6. stream back   │    │   (openai/anthropic/
                         │   ┌──────────────┐   │  7. usage/cost    │◀───┼──  ollama/http)
                         │   │ Cross-cutting│   │  8. feedback (opt)│────┼──▶ cortiq-router
                         │   │ auth · rate  │   └───────────────────┘    │     /v1/feedback
                         │   │ idem · trace │                            │
                         │   └──────────────┘                            │
                         └───────────────────────────────────────────────┘
```

Code modules (`src/`):

| Module | Responsibility |
|---|---|
| `main.rs` | config loading, route registration for enabled protocols, server startup |
| `config.rs` | TOML parsing: model pool, routing table, protocols, keys |
| `router_client.rs` | client for `cortiq-router` (`/v1/route`, `/v1/feedback`) |
| `registry.rs` | model pool registry + `id → provider+endpoint` resolution |
| `routing.rs` | `(label, tier) → ordered model list`; fallback/default |
| `pipeline.rs` | core: extract → route → select → translate → call → stream |
| `providers/mod.rs` | `Provider` trait (unified LLM call interface) |
| `providers/openai.rs` | OpenAI-compatible adapter (covers vLLM/llama.cpp/ollama/openrouter) |
| `providers/anthropic.rs` | Anthropic Messages adapter |
| `protocols/mod.rs` | inbound adapter trait + shared types for the canonical message model |
| `protocols/openai_chat.rs` | `/v1/chat/completions` ↔ canonical model |
| `protocols/anthropic_messages.rs` | `/v1/messages` ↔ canonical model |
| `protocols/mcp.rs` | MCP server (JSON-RPC), routing exposed as a tool |
| `auth.rs` | virtual keys, rate limiting, quotas |
| `telemetry.rs` | Prometheus metrics, OpenTelemetry spans, request IDs |
| `error.rs` | unified error type + rendering into OpenAI error envelope |

---

## 3. Canonical Model (internal neutral format)

To avoid N incoming protocols × M providers turning into N×M translation paths,
everything is normalized internally to a **single canonical request/response model**.

```
ChatRequest {
  routing: RoutingDirective,        // auto | pinned(model_id) | profile
  messages: Vec<Message>,           // role + content (+ tool_calls)
  tools: Vec<Tool>,
  params: GenParams,                // temperature, max_tokens, top_p, stop, ...
  stream: bool,
  meta: RequestMeta,                // idempotency key, traceparent, account
}

ChatResponse {
  id, model_used, choices, usage, finish_reason, cortiq: RouteInfo
}
```

- Inbound adapter: `protocol-specific → ChatRequest` and `ChatResponse → protocol-specific`.
- Outbound provider: `ChatRequest → provider wire` and `provider wire → ChatResponse`.

Adding a new protocol = one inbound adapter. Adding a new provider = one outbound adapter.
No combinatorial explosion.

---

## 4. Extension Traits

```rust
// inbound protocol
#[async_trait]
trait InboundProtocol {
    fn routes(&self) -> Router;                       // axum routes for this protocol
    fn name(&self) -> &'static str;                   // "openai_chat", ...
}

// outbound provider
#[async_trait]
trait Provider: Send + Sync {
    fn id(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream>;  // SSE stream
    async fn embed(&self, req: EmbedRequest) -> Result<EmbedResponse>;
    fn capabilities(&self) -> Caps;                   // tools? streaming? vision?
}
```

The registry (`registry.rs`) holds a `HashMap<model_id, Arc<dyn Provider>>` built
from `[[models]]`. The routing table (`routing.rs`) operates on `model_id`.

---

## 5. `route → select → call` Flow in Detail

1. **Extract.** Text for classification is taken from `messages`. Strategy from config:
   `last_user` (default) | `concat_all` | `last_user_plus_system`. Long text is
   truncated to `route.max_chars` (protection against cost/DoS).
2. **Route.** `router_client.route(text, profile)` → `{task_label, complexity}`.
   Cached by text hash for `route.cache_ttl` (minor savings on repeated requests).
3. **Select.** `routing.select(label, tier)` → ordered `Vec<model_id>`.
   Considers: model health, model `capabilities` (e.g. whether tools are needed),
   and the cost policy (see [ROUTING.md](ROUTING.md)).
4. **Translate + Call.** Canonical `ChatRequest` → provider. Streaming is proxied
   chunk-by-chunk (SSE → SSE) without buffering.
5. **Failover.** If a provider returns 5xx / timeout / overloaded — the next model
   in the list is tried. The circuit breaker marks an unhealthy model to avoid
   hammering it.
6. **Degrade.** If cortiq-router is unavailable, `routing.default` is used (soft
   degradation — the request does not fail).

---

## 6. Reliability

| Mechanism | Behavior |
|---|---|
| Timeouts | separate timeouts for router call, provider connect, and provider stream |
| Failover | ordered model list within the tier |
| Circuit breaker | temporarily excludes a model after repeated errors |
| Router down | `routing.default` |
| All tier providers down | OpenAI error `503 upstream_unavailable`, `retriable=true` |
| Idempotency | repeat with the same `Idempotency-Key` returns the stored response |

---

## 7. Security and Multi-tenancy

- **Key isolation.** Agents hold only a virtual gateway key; provider keys
  (`*_API_KEY`) live in the gateway environment and are never exposed externally.
- **Accounts.** Each `[[api_keys]]` entry maps to an account with its own rate limit,
  quota, and optionally its own allowed-model subset and routing table.
- **PII.** Redaction is performed in the router before escalation to the oracle; the
  gateway can additionally enable logging without request bodies (`log.bodies = false`
  by default).
- **Audit.** Every request records: `request_id`, account, selected model, tokens, cost,
  and decision path — without the prompt text by default.

---

## 8. Observability

`GET /metrics` (Prometheus), `GET /healthz`, `GET /readyz`. Metrics:

```
gw_requests_total{protocol,account}
gw_route_calls_total                 # calls to cortiq-router
gw_model_selected_total{model_id,tier}
gw_provider_calls_total{provider,model_id,outcome}
gw_failovers_total
gw_tokens_total{model_id,direction}
gw_cost_usd_total{account,model_id}
gw_latency_seconds{stage}            # route | provider | total (histogram)
```

OpenTelemetry spans: `inbound → route → provider`, with `traceparent` forwarding.

---

## 9. Roadmap

| Phase | Contents |
|---|---|
| **v0.1** | OpenAI Chat/Completions/Embeddings/Models, `openai` provider, routing + fallback, auth, metrics, streaming |
| **v0.2** | `anthropic` provider + inbound Anthropic Messages, cost-aware selection, idempotency, circuit breaker |
| **v0.3** | MCP server, native `ollama` provider, OpenTelemetry, routing cache |
| **v0.4** | Per-account routing tables, feedback loop to router, load tests, Helm chart |
| **v1.0** | Contract stabilization, OpenAPI spec, official examples for crewAI/LangChain/Hermes |

Current phase: **Phase 0** (skeleton + specification in this repository).
