*Read this in [Русский](PROTOCOLS.ru.md).*

# Cortiq Gateway Protocols

Contracts for inbound protocols and cross-standard mechanisms. Each protocol is enabled
by a flag in `[protocols]`. Internally, all requests are normalized to the canonical model
(see [ARCHITECTURE.md](ARCHITECTURE.md#3-canonical-model-internal-neutral-format)).

Routing rule common to all protocols:

- `model = "cortiq-auto"` → enable intelligent routing via cortiq-router.
- `model = "cortiq-auto:quality-first"` → routing with the specified policy profile
  (`cost-saver` | `balanced` | `quality-first`).
- `model = "<real id from config>"` → passthrough to that model without routing.

---

## Table of Contents

1. [OpenAI Chat Completions](#1-openai-chat-completions)
2. [OpenAI Completions (legacy)](#2-openai-completions-legacy)
3. [OpenAI Embeddings](#3-openai-embeddings)
4. [OpenAI Models](#4-openai-models)
5. [Anthropic Messages](#5-anthropic-messages)
6. [MCP (Model Context Protocol)](#6-mcp-model-context-protocol)
7. [Native passthrough](#7-native-passthrough)
8. [Cross-standard mechanisms](#8-cross-standard-mechanisms)

---

## 1. OpenAI Chat Completions

`POST /v1/chat/completions` — the primary endpoint; the de facto standard for agents.

### Request

```jsonc
{
  "model": "cortiq-auto",                 // routing; or a real id for passthrough
  "messages": [
    { "role": "system", "content": "You are a helpful assistant." },
    { "role": "user", "content": "Solve x^2 - 5x + 6 = 0" }
  ],
  "temperature": 0.7,
  "max_tokens": 1024,
  "stream": false,
  "tools": [ /* function-calling, proxied to the provider as-is */ ],
  "tool_choice": "auto",
  "stop": ["\n\n"],
  "top_p": 1.0,
  "response_format": { "type": "json_object" }
}
```

Standard OpenAI fields are supported; unknown fields are proxied to the provider unchanged
(forward-compatible).

### Response (non-streaming)

```jsonc
{
  "id": "chatcmpl-9f...",
  "object": "chat.completion",
  "created": 1718800000,
  "model": "local-qwen",                  // the ACTUALLY selected model
  "choices": [
    {
      "index": 0,
      "message": { "role": "assistant", "content": "x = 2 or x = 3" },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 24,
    "completion_tokens": 11,
    "total_tokens": 35
  },
  "cortiq": {                             // extension (enabled with cortiq.echo=true)
    "task_label": "math",
    "complexity": { "score": 0.22, "tier": "low" },
    "selected_model": "local-qwen",
    "route_source": "router",             // router | cache | fallback | pinned
    "router_request_id": "req_...",
    "cost_usd": 0.0
  }
}
```

> The `cortiq` field is an optional extension. OpenAI SDKs ignore it. If the client does
> not want extra fields, disable `cortiq.echo` — the metadata will remain in the response
> headers.

### Metadata in response headers (always present)

```
X-Cortiq-Request-Id: req_...
X-Cortiq-Task-Label: math
X-Cortiq-Complexity-Score: 0.22
X-Cortiq-Complexity-Tier: low
X-Cortiq-Selected-Model: local-qwen
X-Cortiq-Route-Source: router
X-Cortiq-Cost-Usd: 0.000000
```

### Response (streaming, `stream: true`)

SSE in OpenAI format: a series of `chat.completion.chunk` events, terminated by
`data: [DONE]`. The gateway proxies provider chunks one-to-one (without buffering),
adding routing metadata to HTTP headers before the stream begins.

```
data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{"role":"assistant"},"index":0}]}

data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{"content":"x = 2"},"index":0}]}

data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{},"finish_reason":"stop","index":0}]}

data: [DONE]
```

---

## 2. OpenAI Completions (legacy)

`POST /v1/completions` — for legacy integrations. Internally, `prompt` is wrapped into a
single user message and follows the same pipeline. Response `object` is `"text_completion"`.

```jsonc
// request
{ "model": "cortiq-auto", "prompt": "Translate to French: hello", "max_tokens": 64 }
// response
{ "object": "text_completion", "model": "local-qwen",
  "choices": [ { "text": "bonjour", "index": 0, "finish_reason": "stop" } ],
  "usage": { "prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8 } }
```

---

## 3. OpenAI Embeddings

`POST /v1/embeddings`. Two modes (configurable):

- `provider` — proxy to an embedding model from the pool;
- `router` — return an embedding from the **cortiq-router encoder** (the same vector the
  router uses for classification — useful for consistent RAG).

```jsonc
// request
{ "model": "cortiq-embed", "input": ["first text", "second text"] }
// response
{
  "object": "list",
  "data": [
    { "object": "embedding", "index": 0, "embedding": [0.01, -0.02, /* ... */] },
    { "object": "embedding", "index": 1, "embedding": [0.03,  0.00, /* ... */] }
  ],
  "model": "cortiq-embed",
  "usage": { "prompt_tokens": 4, "total_tokens": 4 }
}
```

---

## 4. OpenAI Models

`GET /v1/models` — auto-discovery for clients. Returns the gateway's virtual models
(`cortiq-auto`, `cortiq-embed`) and all real models in the pool.

```jsonc
{
  "object": "list",
  "data": [
    { "id": "cortiq-auto", "object": "model", "owned_by": "cortiq-gateway" },
    { "id": "local-qwen",  "object": "model", "owned_by": "cortiq-gateway" },
    { "id": "gpt-4o-mini", "object": "model", "owned_by": "cortiq-gateway" },
    { "id": "claude-opus", "object": "model", "owned_by": "cortiq-gateway" }
  ]
}
```

---

## 5. Anthropic Messages

`POST /v1/messages` — for Claude-native agents and the Anthropic SDK.

```jsonc
// request
{
  "model": "cortiq-auto",
  "max_tokens": 1024,
  "system": "You are a helpful assistant.",
  "messages": [ { "role": "user", "content": "Solve x^2 - 5x + 6 = 0" } ],
  "stream": false
}
// response
{
  "id": "msg_...",
  "type": "message",
  "role": "assistant",
  "model": "local-qwen",
  "content": [ { "type": "text", "text": "x = 2 or x = 3" } ],
  "stop_reason": "end_turn",
  "usage": { "input_tokens": 24, "output_tokens": 11 }
}
```

Streaming uses native Anthropic events (`message_start`, `content_block_delta`,
`message_delta`, `message_stop`). If the selected model is a non-Anthropic provider,
the gateway translates through the canonical model in both directions.

---

## 6. MCP (Model Context Protocol)

The gateway runs an **MCP server** (JSON-RPC 2.0 over HTTP+SSE or stdio) and exposes
routing as tools for MCP-native orchestrators.

Published tools:

| Tool | Purpose |
|---|---|
| `route_and_complete` | classify + execute on the selected model, return the response |
| `classify_task` | classification only (task type + complexity), no execution |

`tools/list` returns JSON schemas. Example `tools/call` invocation:

```jsonc
// request (JSON-RPC)
{
  "jsonrpc": "2.0", "id": 1, "method": "tools/call",
  "params": {
    "name": "route_and_complete",
    "arguments": { "prompt": "Solve x^2 - 5x + 6 = 0", "profile": "balanced" }
  }
}
// response
{
  "jsonrpc": "2.0", "id": 1,
  "result": {
    "content": [ { "type": "text", "text": "x = 2 or x = 3" } ],
    "isError": false,
    "_meta": { "task_label": "math", "tier": "low", "selected_model": "local-qwen" }
  }
}
```

---

## 7. Native passthrough

`POST /route` — a thin wrapper over `cortiq-router /v1/route` for clients that need only
the **routing decision** (without execution). The request body and response match the router
contract (see `cortiq-router/docs/ФОРМАТ_ЗАПРОСА_И_ОТВЕТА.md`). Useful for debugging and
for integrators who want to manage the LLM call themselves.

---

## 8. Cross-standard mechanisms

Common to all HTTP protocols.

### 8.1 Authentication

```
Authorization: Bearer sk-gw-...
```
Virtual gateway key. Mapped to an account (`[[api_keys]]`). If no keys are configured,
the gateway runs in open mode (for local development). Provider keys are never exposed.

### 8.2 Idempotency

```
Idempotency-Key: <unique-operation-id>
```
A repeat request with the same key within the `idempotency.ttl` window returns the
**stored** response (safe agent retries, without double-invoking the LLM or double billing).

### 8.3 Rate limiting

On `429`, the following headers are returned:

```
Retry-After: 2
X-RateLimit-Limit: 600
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1718800060
```

### 8.4 Tracing

W3C `traceparent` is accepted and forwarded; OpenTelemetry spans cover
`inbound→route→provider`. The response includes `X-Request-Id`. This enables an
end-to-end trace from agent → gateway → provider.

### 8.5 Error format (OpenAI-compatible)

```jsonc
{
  "error": {
    "message": "no healthy model available for tier 'high'",
    "type": "upstream_unavailable",      // invalid_request_error | authentication_error | rate_limit_error | upstream_unavailable | internal_error
    "code": "no_healthy_model",
    "param": null
  }
}
```

OpenAI/Anthropic SDKs parse this envelope natively. Status code mapping:

| Type | HTTP | retriable |
|---|---|---|
| `invalid_request_error` | 400 | no |
| `authentication_error` | 401 | no |
| `rate_limit_error` | 429 | yes |
| `upstream_unavailable` | 502/503 | yes |
| `internal_error` | 500 | yes |

### 8.6 Cost tracking

`usage` is returned in the protocol format (OpenAI/Anthropic). Additionally, cost is
included in the `X-Cortiq-Cost-Usd` header and in `cortiq.cost_usd` (if `cortiq.echo`
is enabled). The price is calculated from the model's `price_in`/`price_out` in the config
multiplied by the token counts.
