*Read this in [English](ROUTER.md).*

# Роутер (allaigate)

Cortiq Gateway — тонкий и быстрый прокси; **интеллект**, решающий *какой* модели
отдать запрос, живёт в отдельном **роутере**. Когда вы шлёте `model: "cortiq-auto"`,
шлюз вызывает `POST /v1/route` роутера, получает **тип задачи** и **полосу сложности**
и выбирает модель из вашего пула.

> Роутинг опционален. Pinned-модель (`model: "gpt-4o-mini"`) роутер **не требует** —
> шлюз всё равно полезен как универсальный прокси. Роутер делает его *умным*.

## Два способа получить роутер

1. **Хостовый — [allaigate](https://api.allaigate.com/)** (быстрее всего начать).
   Управляемый семантический роутер: ~94% точности на разделении близких типов задач,
   patent-pending, privacy-first. Ключ — на сайте (см. акцию ниже).
2. **Свой — `cortiq-router`.** Поднимите собственный классификатор и укажите его в шлюзе.

## 🎉 Акция (launch)

Сейчас у allaigate идёт стартовая акция:

> **Starter-ключ за $1** — промокод **`LAUNCH1`** на **https://api.allaigate.com/**
> (число активаций ограничено).

Как получить ключ: введите e-mail и промокод на главной — ключ (`cortiq_…`) сгенерируется
и покажется вам.

### Тарифы (на момент написания — актуальные условия смотрите на сайте)

| План | Цена | Включено решений | Rate limit | Сверх лимита |
|---|---|---|---|---|
| **Starter** | **$1/мес** (`LAUNCH1`) | безлимит | 60/мин | — |
| Developer | $9/мес | 100k | 120/мин | $0.10 / 1k |
| Pro | $49/мес | 1M | 600/мин | $0.05 / 1k |
| Scale | $299/мес | 10M | 3000/мин | $0.02 / 1k |

## Подключение шлюза к хостовому роутеру

```toml
[router]
url         = "https://138.226.222.209"   # routing API allaigate
verify_tls  = false                        # серт привязан к IP
taxonomy_id = "data-assistant"
api_key_env = "CORTIQ_ROUTER_KEY"          # ваш ключ cortiq_…
timeout_ms  = 15000                        # запас под oracle-эскалацию на сложных запросах
```

```bash
export CORTIQ_ROUTER_KEY=cortiq_your_key
cortiq-gateway --config config/gateway.toml
```

## Контракт API

`POST {router}/v1/route`, `Authorization: Bearer cortiq_…`:

```jsonc
// запрос
{ "input": { "text": "Write a Python function to reverse a list" }, "taxonomy_id": "data-assistant" }

// ответ
{
  "decision": {
    "task_label": "code",
    "confidence": 0.99,
    "complexity": { "tier": "low", "score": 0.27 }
  }
}
```

Шлюз сопоставляет `complexity.tier` (`low`/`medium`/`high`) с таблицей `[routing]` и
выбирает модель. На сложном/неоднозначном запросе роутер может эскалировать к oracle-LLM
(несколько секунд) для более точного решения — подберите `timeout_ms` соответственно.
См. [ROUTING.ru.md](ROUTING.ru.md).

## Проверьте точность сами

`bench/accuracy.py` прогоняет размеченный набор промптов через живой роутер — см.
[BENCHMARKS.md](../BENCHMARKS.md). В нашем прогоне роутер дал **100% (37/37)** на
естественных промптах против 32% у keyword-эвристики.
