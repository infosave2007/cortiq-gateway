*Read this in [English](PROTOCOLS.md).*

# Протоколы Cortiq Gateway

Контракты входящих протоколов и кросс-стандартных механизмов. Каждый протокол
включается флагом в `[protocols]`. Внутри всё приводится к канонической модели
(см. [ARCHITECTURE.md](ARCHITECTURE.ru.md#3-каноническая-модель)).

Общее правило маршрутизации во всех протоколах:

- `model = "cortiq-auto"` → включить интеллектуальный роутинг через cortiq-router.
- `model = "cortiq-auto:quality-first"` → роутинг с указанным policy-профилем
  (`cost-saver` | `balanced` | `quality-first`).
- `model = "<реальный id из конфига>"` → passthrough к этой модели без роутинга.

---

## Оглавление

1. [OpenAI Chat Completions](#1-openai-chat-completions)
2. [OpenAI Completions (legacy)](#2-openai-completions-legacy)
3. [OpenAI Embeddings](#3-openai-embeddings)
4. [OpenAI Models](#4-openai-models)
5. [Anthropic Messages](#5-anthropic-messages)
6. [MCP (Model Context Protocol)](#6-mcp-model-context-protocol)
7. [Native passthrough](#7-native-passthrough)
8. [Кросс-стандарты](#8-кросс-стандарты)

---

## 1. OpenAI Chat Completions

`POST /v1/chat/completions` — основной endpoint, де-факто стандарт для агентов.

### Запрос

```jsonc
{
  "model": "cortiq-auto",                 // роутинг; или реальный id для passthrough
  "messages": [
    { "role": "system", "content": "You are a helpful assistant." },
    { "role": "user", "content": "Solve x^2 - 5x + 6 = 0" }
  ],
  "temperature": 0.7,
  "max_tokens": 1024,
  "stream": false,
  "tools": [ /* function-calling, проксируется к провайдеру как есть */ ],
  "tool_choice": "auto",
  "stop": ["\n\n"],
  "top_p": 1.0,
  "response_format": { "type": "json_object" }
}
```

Поддерживаются стандартные поля OpenAI; неизвестные поля проксируются к провайдеру
без изменений (forward-compatible).

### Ответ (без стриминга)

```jsonc
{
  "id": "chatcmpl-9f...",
  "object": "chat.completion",
  "created": 1718800000,
  "model": "local-qwen",                  // РЕАЛЬНО выбранная модель
  "choices": [
    {
      "index": 0,
      "message": { "role": "assistant", "content": "x = 2 или x = 3" },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 24,
    "completion_tokens": 11,
    "total_tokens": 35
  },
  "cortiq": {                             // расширение (включается cortiq.echo=true)
    "task_label": "math",
    "complexity": { "score": 0.22, "tier": "low" },
    "selected_model": "local-qwen",
    "route_source": "router",             // router | cache | fallback | pinned
    "router_request_id": "req_...",
    "cost_usd": 0.0
  }
}
```

> Поле `cortiq` — необязательное расширение. OpenAI-SDK его игнорируют. Если клиент
> не хочет «лишних» полей — выключите `cortiq.echo`, метаданные останутся в заголовках.

### Метаданные в заголовках (всегда)

```
X-Cortiq-Request-Id: req_...
X-Cortiq-Task-Label: math
X-Cortiq-Complexity-Score: 0.22
X-Cortiq-Complexity-Tier: low
X-Cortiq-Selected-Model: local-qwen
X-Cortiq-Route-Source: router
X-Cortiq-Cost-Usd: 0.000000
```

### Ответ (стриминг, `stream: true`)

SSE, формат OpenAI: серия чанков `chat.completion.chunk`, завершается `data: [DONE]`.
Шлюз проксирует чанки провайдера один-в-один (без буферизации), добавив метаданные
роутинга в HTTP-заголовки до начала потока.

```
data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{"role":"assistant"},"index":0}]}

data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{"content":"x = 2"},"index":0}]}

data: {"id":"chatcmpl-9f","object":"chat.completion.chunk","choices":[{"delta":{},"finish_reason":"stop","index":0}]}

data: [DONE]
```

---

## 2. OpenAI Completions (legacy)

`POST /v1/completions` — для старых интеграций. Внутри `prompt` оборачивается в
одно user-сообщение и идёт по тому же пути. Ответ — `object: "text_completion"`.

```jsonc
// запрос
{ "model": "cortiq-auto", "prompt": "Translate to French: hello", "max_tokens": 64 }
// ответ
{ "object": "text_completion", "model": "local-qwen",
  "choices": [ { "text": "bonjour", "index": 0, "finish_reason": "stop" } ],
  "usage": { "prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8 } }
```

---

## 3. OpenAI Embeddings

`POST /v1/embeddings`. Два режима (настраивается):

- `provider` — проксировать к embedding-модели из пула;
- `router` — вернуть эмбеддинг **энкодером cortiq-router** (тот же вектор, что роутер
  использует для классификации — удобно для согласованного RAG).

```jsonc
// запрос
{ "model": "cortiq-embed", "input": ["first text", "second text"] }
// ответ
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

`GET /v1/models` — автодискавери для клиентов. Возвращает виртуальные модели шлюза
(`cortiq-auto`, `cortiq-embed`) и все реальные модели пула.

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

`POST /v1/messages` — для Claude-нативных агентов и Anthropic SDK.

```jsonc
// запрос
{
  "model": "cortiq-auto",
  "max_tokens": 1024,
  "system": "You are a helpful assistant.",
  "messages": [ { "role": "user", "content": "Solve x^2 - 5x + 6 = 0" } ],
  "stream": false
}
// ответ
{
  "id": "msg_...",
  "type": "message",
  "role": "assistant",
  "model": "local-qwen",
  "content": [ { "type": "text", "text": "x = 2 или x = 3" } ],
  "stop_reason": "end_turn",
  "usage": { "input_tokens": 24, "output_tokens": 11 }
}
```

Стриминг — нативные Anthropic-события (`message_start`, `content_block_delta`,
`message_delta`, `message_stop`). Если выбранная модель — не-Anthropic-провайдер,
шлюз транслирует через каноническую модель в обе стороны.

---

## 6. MCP (Model Context Protocol)

Шлюз поднимает **MCP-сервер** (JSON-RPC 2.0 поверх HTTP+SSE или stdio) и публикует
маршрутизацию как инструменты для MCP-нативных оркестраторов.

Публикуемые инструменты:

| Инструмент | Назначение |
|---|---|
| `route_and_complete` | классифицировать + выполнить на выбранной модели, вернуть ответ |
| `classify_task` | только классификация (тип задачи + сложность), без выполнения |

`tools/list` отдаёт JSON-схемы. Пример вызова `tools/call`:

```jsonc
// запрос (JSON-RPC)
{
  "jsonrpc": "2.0", "id": 1, "method": "tools/call",
  "params": {
    "name": "route_and_complete",
    "arguments": { "prompt": "Solve x^2 - 5x + 6 = 0", "profile": "balanced" }
  }
}
// ответ
{
  "jsonrpc": "2.0", "id": 1,
  "result": {
    "content": [ { "type": "text", "text": "x = 2 или x = 3" } ],
    "isError": false,
    "_meta": { "task_label": "math", "tier": "low", "selected_model": "local-qwen" }
  }
}
```

---

## 7. Native passthrough

`POST /route` — тонкая обёртка над `cortiq-router /v1/route` для клиентов, которым
нужно только **решение** (без выполнения). Тело и ответ совпадают с контрактом роутера
(см. `cortiq-router/docs/ФОРМАТ_ЗАПРОСА_И_ОТВЕТА.md`). Полезно для отладки и для тех,
кто хочет сам управлять вызовом LLM.

---

## 8. Кросс-стандарты

Едины для всех HTTP-протоколов.

### 8.1 Аутентификация

```
Authorization: Bearer sk-gw-...
```
Виртуальный ключ шлюза. Маппится на аккаунт (`[[api_keys]]`). Если ключи не настроены —
открытый режим (для локальной разработки). Ключи провайдеров наружу не отдаются.

### 8.2 Идемпотентность

```
Idempotency-Key: <уникальный-id-операции>
```
Повтор с тем же ключом в окне `idempotency.ttl` отдаёт **сохранённый** ответ
(безопасные ретраи агентов, без двойного вызова LLM и двойного биллинга).

### 8.3 Rate-limit

На `429` возвращаются:

```
Retry-After: 2
X-RateLimit-Limit: 600
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1718800060
```

### 8.4 Трассировка

Приём и проброс W3C `traceparent`; OpenTelemetry-спаны `inbound→route→provider`.
В ответе — `X-Request-Id`. Это позволяет сшить сквозной трейс агент → шлюз → провайдер.

### 8.5 Формат ошибки (OpenAI-совместимый)

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

OpenAI/Anthropic SDK парсят такой конверт нативно. Сопоставление статусов:

| Тип | HTTP | retriable |
|---|---|---|
| `invalid_request_error` | 400 | нет |
| `authentication_error` | 401 | нет |
| `rate_limit_error` | 429 | да |
| `upstream_unavailable` | 502/503 | да |
| `internal_error` | 500 | да |

### 8.6 Учёт стоимости

`usage` отдаётся в формате протокола (OpenAI/Anthropic). Дополнительно — стоимость
в заголовке `X-Cortiq-Cost-Usd` и в `cortiq.cost_usd` (если включено `cortiq.echo`).
Цена считается из `price_in`/`price_out` модели в конфиге × токены.
