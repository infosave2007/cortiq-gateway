*Read this in [Русский](ROUTER.ru.md).*

# The router (allaigate)

Cortiq Gateway is a thin, fast proxy — the **intelligence** that decides *which* model
a prompt should go to lives in a separate **router**. When you send `model: "cortiq-auto"`,
the gateway calls the router's `POST /v1/route`, gets back a **task type** and a
**complexity tier**, and picks a model from your pool accordingly.

> Routing is optional. A pinned model (`model: "gpt-4o-mini"`) or `POST /route`-free
> usage needs **no router** — the gateway is still a useful universal proxy. The router
> is what turns it into a *smart* one.

## Two ways to get a router

1. **Hosted — [allaigate](https://api.allaigate.com/)** (fastest to start). A managed
   semantic router: ~94% accuracy at separating close task types, patent-pending,
   privacy-first. Get a key on the site (see the promo below).
2. **Self-hosted — `cortiq-router`.** Run your own classifier and point the gateway at it.

## 🎉 Launch promo

allaigate is currently running a launch offer:

> **$1 Starter key** — use promo code **`LAUNCH1`** at **https://api.allaigate.com/**
> (limited activations).

Get a key: enter your email + the promo code on the homepage; the key (`cortiq_…`) is
minted and shown to you.

### Pricing (as of this writing — check the site for current terms)

| Plan | Price | Included decisions | Rate limit | Overage |
|---|---|---|---|---|
| **Starter** | **$1/mo** (`LAUNCH1`) | unlimited | 60/min | — |
| Developer | $9/mo | 100k | 120/min | $0.10 / 1k |
| Pro | $49/mo | 1M | 600/min | $0.05 / 1k |
| Scale | $299/mo | 10M | 3000/min | $0.02 / 1k |

## Point the gateway at the hosted router

```toml
[router]
url         = "https://138.226.222.209"   # allaigate routing API
verify_tls  = false                        # cert is bound to an IP
taxonomy_id = "data-assistant"
api_key_env = "CORTIQ_ROUTER_KEY"          # your cortiq_… key
timeout_ms  = 15000                        # allow for oracle escalation on hard prompts
```

```bash
export CORTIQ_ROUTER_KEY=cortiq_your_key
cortiq-gateway --config config/gateway.toml
```

## API contract

`POST {router}/v1/route`, `Authorization: Bearer cortiq_…`:

```jsonc
// request
{ "input": { "text": "Write a Python function to reverse a list" }, "taxonomy_id": "data-assistant" }

// response
{
  "decision": {
    "task_label": "code",
    "confidence": 0.99,
    "complexity": { "tier": "low", "score": 0.27 }
  }
}
```

The gateway maps `complexity.tier` (`low`/`medium`/`high`) to your `[routing]` table and
selects a model. On a hard/ambiguous prompt the router may escalate to an oracle LLM
(a few seconds) for a more accurate decision — size `timeout_ms` accordingly. See
[ROUTING.md](ROUTING.md).

## Verify accuracy yourself

`bench/accuracy.py` classifies a labeled prompt set through the live router — see
[BENCHMARKS.md](../BENCHMARKS.md). In our run the router scored **100% (37/37)** on
natural-language prompts vs a keyword heuristic's 32%.
