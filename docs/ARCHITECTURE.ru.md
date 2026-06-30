*Read this in [English](ARCHITECTURE.md).*

# Архитектура Cortiq Gateway

Документ описывает внутреннее устройство шлюза, поток запроса, абстракции и
дорожную карту. Сопутствующие документы: [PROTOCOLS.md](PROTOCOLS.ru.md) (контракты
протоколов), [ROUTING.md](ROUTING.ru.md) (модель выбора моделей и стоимости).

---

## 1. Назначение и границы

**Cortiq Gateway** — это L7-прокси, который:

1. принимает запрос в одном из **стандартных протоколов** (OpenAI, Anthropic, MCP);
2. спрашивает **cortiq-router**, какой это тип задачи и насколько он сложный;
3. выбирает конкретную модель из **пула провайдеров** (локальные + облачные);
4. вызывает провайдера, **транслируя протокол** при необходимости;
5. возвращает ответ клиенту (стриминг и без), добавляя метаданные роутинга;
6. учитывает стоимость и (опц.) замыкает петлю обратной связи в роутер.

Что шлюз **не** делает: не хранит и не обучает модели (это роутер), не реализует
сами LLM (это провайдеры). Шлюз — тонкий, без состояния (кроме кэшей идемпотентности
и rate-limit), горизонтально масштабируется.

---

## 2. Компонентная схема

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

Модули кода (`src/`):

| Модуль | Ответственность |
|---|---|
| `main.rs` | загрузка конфига, сборка роутов по включённым протоколам, запуск сервера |
| `config.rs` | парсинг TOML: пул моделей, таблица роутинга, протоколы, ключи |
| `router_client.rs` | клиент к `cortiq-router` (`/v1/route`, `/v1/feedback`) |
| `registry.rs` | реестр моделей пула + резолв `id → провайдер+endpoint` |
| `routing.rs` | `(label, tier) → упорядоченный список моделей`; fallback/default |
| `pipeline.rs` | ядро: extract → route → select → translate → call → stream |
| `providers/mod.rs` | трейт `Provider` (унифицированный вызов LLM) |
| `providers/openai.rs` | OpenAI-совместимый адаптер (covers vLLM/llama.cpp/ollama/openrouter) |
| `providers/anthropic.rs` | адаптер Anthropic Messages |
| `protocols/mod.rs` | трейт входящего адаптера + общие типы канонической модели сообщения |
| `protocols/openai_chat.rs` | вход `/v1/chat/completions` ↔ каноническая модель |
| `protocols/anthropic_messages.rs` | вход `/v1/messages` ↔ каноническая модель |
| `protocols/mcp.rs` | MCP-сервер (JSON-RPC), маршрутизация как инструмент |
| `auth.rs` | виртуальные ключи, rate-limit, квоты |
| `telemetry.rs` | метрики Prometheus, OpenTelemetry-спаны, request-id |
| `error.rs` | единый тип ошибки + рендер в OpenAI-конверт |

---

## 3. Каноническая модель (внутренний нейтральный формат)

Чтобы N входящих протоколов × M провайдеров не превратились в N×M переводов,
внутри всё приводится к **одной канонической модели** запроса/ответа.

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

- Входящий адаптер: `protocol-specific → ChatRequest` и `ChatResponse → protocol-specific`.
- Исходящий провайдер: `ChatRequest → provider wire` и `provider wire → ChatResponse`.

Добавить новый протокол = один входящий адаптер. Добавить нового провайдера =
один исходящий адаптер. Никакого комбинаторного взрыва.

---

## 4. Трейты-расширения

```rust
// входящий протокол
#[async_trait]
trait InboundProtocol {
    fn routes(&self) -> Router;                       // axum-роуты этого протокола
    fn name(&self) -> &'static str;                   // "openai_chat", ...
}

// исходящий провайдер
#[async_trait]
trait Provider: Send + Sync {
    fn id(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream>;  // SSE-поток
    async fn embed(&self, req: EmbedRequest) -> Result<EmbedResponse>;
    fn capabilities(&self) -> Caps;                   // tools? streaming? vision?
}
```

Реестр (`registry.rs`) держит `HashMap<model_id, Arc<dyn Provider>>`, построенный
из `[[models]]`. Таблица роутинга (`routing.rs`) оперирует `model_id`.

---

## 5. Поток `route → select → call` детально

1. **Extract.** Из `messages` берётся текст для классификации. Стратегия из конфига:
   `last_user` (по умолчанию) | `concat_all` | `last_user_plus_system`. Длинный
   текст усечётся до `route.max_chars` (защита от стоимости/DoS).
2. **Route.** `router_client.route(text, profile)` → `{task_label, complexity}`.
   Кэшируется по хэшу текста на `route.cache_ttl` (мелкая экономия на повторах).
3. **Select.** `routing.select(label, tier)` → упорядоченный `Vec<model_id>`.
   Учитываются: доступность модели (health), её `capabilities` (нужны ли tools),
   и cost-политика (см. [ROUTING.md](ROUTING.ru.md)).
4. **Translate + Call.** Канонический `ChatRequest` → провайдер. Стриминг проксируется
   чанк-в-чанк (SSE → SSE) без буферизации.
5. **Failover.** Если провайдер вернул 5xx/таймаут/перегрузку — следующий из списка.
   Circuit breaker помечает «больную» модель, чтобы не долбить её.
6. **Degrade.** Если cortiq-router недоступен — берётся `routing.default` (мягкая
   деградация, запрос не падает).

---

## 6. Надёжность

| Механизм | Поведение |
|---|---|
| Таймауты | отдельные на router-вызов, connect и стрим провайдера |
| Failover | по упорядоченному списку моделей полосы |
| Circuit breaker | временно исключает модель после серии ошибок |
| Router down | `routing.default` |
| Все провайдеры полосы down | OpenAI-ошибка `503 upstream_unavailable`, `retriable=true` |
| Идемпотентность | повтор с тем же `Idempotency-Key` отдаёт сохранённый ответ |

---

## 7. Безопасность и мультиарендность

- **Изоляция ключей.** Агенты держат только виртуальный ключ шлюза; ключи провайдеров
  (`*_API_KEY`) живут в окружении шлюза и наружу не уходят.
- **Аккаунты.** Каждый `[[api_keys]]` → аккаунт с rate-limit, квотой, (опц.) своим
  подмножеством разрешённых моделей и своей таблицей роутинга.
- **PII.** Редакция выполняется в роутере перед эскалацией к оракулу; шлюз дополнительно
  может включить логирование без тел сообщений (`log.bodies = false` по умолчанию).
- **Аудит.** Каждый запрос: `request_id`, аккаунт, выбранная модель, токены, стоимость,
  путь решения — без текста промпта по умолчанию.

---

## 8. Наблюдаемость

`GET /metrics` (Prometheus), `GET /healthz`, `GET /readyz`. Метрики:

```
gw_requests_total{protocol,account}
gw_route_calls_total                 # обращения к cortiq-router
gw_model_selected_total{model_id,tier}
gw_provider_calls_total{provider,model_id,outcome}
gw_failovers_total
gw_tokens_total{model_id,direction}
gw_cost_usd_total{account,model_id}
gw_latency_seconds{stage}            # route | provider | total (histogram)
```

OpenTelemetry-спаны: `inbound → route → provider`, с пробросом `traceparent`.

---

## 9. Дорожная карта

| Фаза | Содержимое |
|---|---|
| **v0.1** | OpenAI Chat/Completions/Embeddings/Models, провайдер `openai`, роутинг + fallback, auth, метрики, стриминг |
| **v0.2** | Провайдер `anthropic` + входящий Anthropic Messages, cost-aware выбор, идемпотентность, circuit breaker |
| **v0.3** | MCP-сервер, провайдер `ollama` нативный, OpenTelemetry, кэш роутинга |
| **v0.4** | Per-account таблицы роутинга, feedback-loop в роутер, нагрузочные тесты, Helm-чарт |
| **v1.0** | Стабилизация контракта, OpenAPI-спека, официальные примеры под crewAI/LangChain/Hermes |

Текущая фаза — **Phase 0** (скелет + спецификация в этом репозитории).
