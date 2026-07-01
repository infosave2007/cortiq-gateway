# Cortiq Gateway

[English](README.md) · **Русский**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)
![Status](https://img.shields.io/badge/status-active-success.svg)

**Универсальный LLM-шлюз с интеллектуальной маршрутизацией.**
Один OpenAI-совместимый endpoint → автоматический выбор модели из вашего пула
(локальные дешёвые + хостовые дорогие) на основе типа задачи и сложности, которые
определяет [allaigate / cortiq-router](https://api.allaigate.com).

> Меняете в своём агенте/SDK только `base_url` — и получаете «умный» роутинг между
> моделями. Никакой логики выбора модели на стороне клиента.

```
┌─────────────┐   OpenAI / Anthropic / MCP    ┌──────────────────┐
│   Агент /   │ ─────────────────────────────▶│  Cortiq Gateway  │
│ разработчик │   (стандартный протокол)       │  (этот проект)   │
└─────────────┘ ◀─────────────────────────────└──────────────────┘
                        ответ + метаданные        │          │
                                                  │ /v1/route│ вызов LLM
                                                  ▼          ▼
                                         ┌──────────────┐  ┌─────────────────────┐
                                         │ cortiq-router│  │  Пул моделей        │
                                         │ (тип задачи, │  │  • local llama.cpp  │
                                         │  сложность)  │  │  • ollama / vLLM    │
                                         └──────────────┘  │  • OpenAI / Claude  │
                                                           └─────────────────────┘
```

---

## ✨ Ключевое

- **Быстрый.** Rust/Tokio: overhead шлюза с **субмиллисекундным p99** — ~10× к throughput Portkey и ~47× к LiteLLM в сопоставимом бенчмарке ([детали](BENCHMARKS.md)).
- **Drop-in OpenAI API + стриминг.** Направьте любой OpenAI-клиент на шлюз и шлите `model: "cortiq-auto"`; `stream: true` поддержан (SSE).
- **Интеллектуальный роутинг.** `complexity.tier` → упорядоченный пул моделей (`low → локально`, `high → облако`), с fallback и мягкой деградацией, если роутер недоступен.
- **Встроенная многоязычная панель управления.** Модели, роутинг, протоколы, ключи и секреты — из веб-интерфейса, **без ручного TOML и без рестарта** (горячая перезагрузка). **7 языков** (en, ru, de, fr, es, zh, tr), тёмная/светлая тема.
- **Живая аналитика.** Статистика по запросам (токены, стоимость, латентность, доля успеха, failover), графики, разбивка по моделям/полосам/задачам и Prometheus `GET /metrics`.
- **Playground.** Прогон промпта через живой пайплайн с разбором решения роутинга.
- **Один самодостаточный бинарь.** SPA встроена в Rust-бинарь — ничего лишнего разворачивать не нужно.
- **Секреты не покидают шлюз.** Агенты держат только виртуальный ключ шлюза; ключи провайдеров живут в шлюзе и наружу не уходят.

---

## 📦 Установка

```bash
# с crates.io
cargo install cortiq-gateway

# или сборка из исходников
git clone https://github.com/infosave2007/cortiq-gateway
cd cortiq-gateway
cargo build --release   # ./target/release/cortiq-gateway
```

---

## 🖥️ Панель управления

![Дашборд](docs/screenshots/dashboard.png)

В шлюз встроена веб-панель на **`/admin`**:

| | |
|---|---|
| ![Модели](docs/screenshots/models.png) | ![Роутинг](docs/screenshots/routing.png) |
| **Models** — добавление/правка/probe моделей, ключи провайдеров | **Routing** — визуальный редактор полос (упорядоченный) |
| ![Playground](docs/screenshots/playground.png) | ![Дашборд](docs/screenshots/dashboard.png) |
| **Playground** — тест живого пайплайна с решением роутинга | **Dashboard** — трафик, стоимость, латентность |

```bash
cargo run --release -- --config config/gateway.toml --admin-token <ВАШ_ТОКЕН>
# откройте  http://localhost:9000/admin?token=<ВАШ_ТОКЕН>
```

Если `--admin-token` / `[admin].token_env` не заданы — токен генерируется при старте и
печатается в лог. Все эндпоинты `/admin/api/*` требуют Bearer admin-токен; значения
секретов API наружу не отдаёт (только статус наличия: `store` / `env` / `missing`).

---

## 🚀 Быстрый старт

```bash
# 1. поднимите роутер (хостовый allaigate или локальный cortiq-router)
#    он слушает, например, http://localhost:8080 (или https://api.allaigate.com)

# 2. опишите свой пул моделей
cp config/gateway.example.toml config/gateway.toml
$EDITOR config/gateway.toml

# 3. запустите шлюз
cargo run --release -- --config config/gateway.toml
# Шлюз слушает 0.0.0.0:9000 и отдаёт панель на /admin
```

Теперь любой OpenAI-клиент работает через шлюз:

```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:9000/v1", api_key="sk-gw-...")

resp = client.chat.completions.create(
    model="cortiq-auto",                       # ← магическая модель = «выбери сам»
    messages=[{"role": "user", "content": "Solve x^2 - 5x + 6 = 0"}],
)
print(resp.choices[0].message.content)
# заголовки ответа покажут выбор шлюза:
#   X-Cortiq-Task-Label: math
#   X-Cortiq-Complexity-Tier: low
#   X-Cortiq-Selected-Model: local-qwen
```

`model: "cortiq-auto"` включает роутинг. Любое **реальное** имя модели из конфига
(`"gpt-4o-mini"`, `"local-qwen"`) — это passthrough напрямую, без роутинга.

> **Используете хостовый allaigate-роутер?** Укажите `url = "https://138.226.222.209"`,
> `verify_tls = false`, `taxonomy_id = "data-assistant"` и ключ `cortiq_…` в
> `CORTIQ_ROUTER_KEY`. На сложных запросах роутер эскалирует к oracle (~10 с), поэтому
> ставьте `timeout_ms = 12000+` — иначе шлюз мягко деградирует на default-модель.

---

## Поток запроса

1. Клиент → `POST /v1/chat/completions` (или Anthropic/MCP) с `model: "cortiq-auto"`.
2. Шлюз извлекает **текст для маршрутизации** (стратегия настраивается, см. [docs/ROUTING.ru.md](docs/ROUTING.ru.md)).
3. Шлюз → `cortiq-router /v1/route` → получает `task_label` + `complexity.tier`.
4. Шлюз выбирает модель из пула по таблице роутинга (с порядком fallback).
5. Шлюз → провайдер выбранной модели (транслируя протокол при необходимости), возвращает ответ.
6. Шлюз отдаёт метаданные роутинга в заголовках/`usage`, считает стоимость и статистику.

Подробно — [docs/ARCHITECTURE.ru.md](docs/ARCHITECTURE.ru.md).

---

## Поддерживаемые протоколы (входящие)

| Протокол | Endpoint | Статус |
|---|---|---|
| OpenAI Chat Completions | `POST /v1/chat/completions` | ✅ реализовано (+ стриминг) |
| OpenAI Embeddings | `POST /v1/embeddings` | ✅ реализовано |
| Anthropic Messages | `POST /v1/messages` | ✅ реализовано (+ стриминг) |
| MCP (Model Context Protocol) | `POST /mcp` (JSON-RPC) | ✅ реализовано |
| OpenAI Completions (legacy) | `POST /v1/completions` | ✅ реализовано (+ стриминг) |
| OpenAI Models | `GET /v1/models` | ✅ реализовано |
| Native passthrough | `POST /route` | ✅ реализовано |

## Поддерживаемые провайдеры (исходящие)

| Провайдер | Покрывает |
|---|---|
| `openai` (OpenAI-совместимый) | OpenAI, OpenRouter, Together, Groq, **vLLM, llama.cpp server, LM Studio, Ollama (`/v1`)** |
| `anthropic` | Claude Messages API (+ стриминг) |
| `ollama` | нативный Ollama API |
| `http` | произвольный HTTP-эндпоинт (свой адаптер) |

---

## Конфигурация

Полный пример — [config/gateway.example.toml](config/gateway.example.toml).
Модель роутинга и cost-aware выбор — [docs/ROUTING.ru.md](docs/ROUTING.ru.md).
Всё из конфига также правится из панели управления в рантайме.

---

## ⚡ Производительность и точность

**Латентность** — overhead шлюза при проксировании одного мгновенного бэкенда, `ab -k -r -c 20 -n 5000`:

| Шлюз | req/s | p50 | p99 |
|---|--:|--:|--:|
| **Cortiq Gateway** (Rust) | **~57 300** | 0 ms | **1 ms** |
| Portkey (Node) | ~5 800 | 3 ms | 9 ms |
| LiteLLM (Python, 4 воркера) | ~1 200 | 9 ms | 59 ms |

**Точность** — определение типа задачи на естественных промптах (7 типов). У LiteLLM/Portkey
семантического роутера нет; keyword-эвристика — это DIY-замена:

| Классификатор | Точность |
|---|--:|
| **семантический роутер allaigate** | **100%** (37/37) |
| keyword-эвристика (без классификатора) | 32% (12/37) |

Методология, оговорки и воспроизводимый harness — **[BENCHMARKS.md](BENCHMARKS.md)**
(`bash bench/run.sh`, `python3 bench/accuracy.py`). Числа зависят от железа; важны разрывы.

## Сборка и тесты

```bash
cargo build --release      # один самодостаточный бинарь (SPA встроена)
cargo test                 # round-trip конфига, валидация роутинга
```

## ✅ Проверено

| Область | Как проверено | Статус |
|---|---|---|
| Round-trip загрузки/записи конфига, валидация роутинга | `cargo test` (юнит) | ✅ |
| Сборка · формат · линт | CI: `cargo build` / `fmt --check` / `clippy -D warnings` на Linux · macOS · Windows | ✅ |
| OpenAI Chat + SSE-стриминг | интеграционно (mock-бэкенд) | ✅ |
| Anthropic Messages — in/out, стриминг + не-стриминг | интеграционно (mock) | ✅ |
| Embeddings (`POST /v1/embeddings`) | интеграционно (mock) | ✅ |
| Семантический кэш — hit / miss / экономия | интеграционно (mock) | ✅ |
| MCP — `initialize` / `tools/list` / `tools/call` | интеграционно (mock) | ✅ |
| Models · Completions · нативный `POST /route` | интеграционно (mock) | ✅ |
| Роутинг — auto / pinned / failover / деградация | интеграционно (mock **и** живой allaigate-роутер) | ✅ |
| Горячая перезагрузка конфига (без рестарта) | интеграционно | ✅ |
| Бенчмарк latency vs LiteLLM / Portkey | `bench/run.sh` (Apache Bench) — см. [BENCHMARKS.md](BENCHMARKS.md) | ✅ |
| Точность определения типа задачи | `bench/accuracy.py` — живой роутер 100% vs keyword-эвристика 32% | ✅ |

---

## Статус

Реализовано: OpenAI Chat Completions **со стримингом (SSE)**, провайдер и вход
**Anthropic** `/v1/messages` (стриминг), **embeddings**, **семантический кэш**,
**MCP**-сервер, роутинг с fallback и мягкой деградацией, учёт стоимости/токенов,
**встроенная многоязычная панель управления с горячей перезагрузкой конфига**,
статистика и Prometheus `GET /metrics`. В планах: per-account таблицы роутинга,
feedback-loop в роутер. Дорожная карта —
[docs/ARCHITECTURE.ru.md](docs/ARCHITECTURE.ru.md). Контрибьюции приветствуются —
см. [CONTRIBUTING.ru.md](CONTRIBUTING.ru.md).

## Лицензия

Apache-2.0 — см. [LICENSE](LICENSE).
