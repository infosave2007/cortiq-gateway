*Read this in [Русский](ROUTING.ru.md).*

# Routing and Cost

How Cortiq Gateway translates a router decision (`task_label` + `complexity.tier`) into
selection of a specific model from your pool — and how to make this **cost-aware**:
cheap tasks stay local, expensive ones go to the cloud.

---

## 1. Base model: complexity tier → model pool

cortiq-router returns a complexity tier (`low` / `medium` / `high`, configurable in the
router). The `[routing]` table maps each tier to an **ordered list** of models — which
serves simultaneously as a priority order and a fallback chain.

```toml
[routing]
low    = ["local-qwen"]                  # simple → local only (free)
medium = ["gpt-4o-mini", "local-qwen"]   # medium → cheap hosted, fall back to local
high   = ["claude-opus", "gpt-4o-mini"]  # complex → expensive model, fall back to mid
default = "gpt-4o-mini"                   # if the router is unavailable
```

Selection algorithm:

```
1. tier := router.complexity.tier        (or the default tier if router is unavailable)
2. candidates := routing[tier]           (ordered list of model_id)
3. for each model_id in order:
     if the model is healthy AND supports the required capabilities (tools/vision):
         select it → done
4. if all are unavailable: try the next tier down/up per policy, or return error 503
```

---

## 2. Universal pool: cheap local + expensive hosted

The primary developer scenario: mix a **free local** model with a **paid cloud** API,
and let the gateway decide which one to use.

```toml
# Local model on vLLM / llama.cpp / Ollama (OpenAI-compatible endpoint)
[[models]]
id        = "local-qwen"
provider  = "openai"
base_url  = "http://localhost:8000/v1"
model     = "qwen2.5-7b-instruct"
cost_tier = "cheap"
price_in  = 0.0                          # local → 0
price_out = 0.0
caps      = ["tools"]                     # capabilities (checked during selection)

# Cheap cloud
[[models]]
id          = "gpt-4o-mini"
provider    = "openai"
base_url    = "https://api.openai.com/v1"
model       = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"
cost_tier   = "mid"
price_in    = 0.15                        # $/1M input tokens
price_out   = 0.60

# Expensive cloud for complex tasks
[[models]]
id          = "claude-opus"
provider    = "anthropic"
base_url    = "https://api.anthropic.com"
model       = "claude-opus-4"
api_key_env = "ANTHROPIC_API_KEY"
cost_tier   = "expensive"
price_in    = 15.0
price_out   = 75.0
caps        = ["tools", "vision"]
```

Because both `local-qwen` and `gpt-4o-mini` use `provider = "openai"`, the gateway does
not care where the model is physically located — **local servers already speak the OpenAI
API**. This is what makes the proxy truly universal.

---

## 3. Cost-aware mode (optional)

Instead of a fixed table, you can enable selection by **budget/price** (v0.2):

```toml
[routing.policy]
mode = "cost_aware"        # fixed_table | cost_aware
# in cost_aware mode the gateway picks the cheapest model for each tier
# that meets the minimum quality class for that tier:
min_class = { low = "cheap", medium = "mid", high = "expensive" }
# upper price bound per request (guard against expensive calls)
max_cost_usd_per_request = 0.50
```

Logic: among models that are at least `min_class[tier]` and within the price cap, the
cheapest available model is selected. This yields the "minimally sufficient" model for
the task complexity.

---

## 4. Pinning and passthrough

- **Passthrough.** `model = "local-qwen"` (a real id) — no router call; direct invocation.
  Useful when the client already knows what it wants.
- **Profile pinning.** `model = "cortiq-auto:cost-saver"` — routing with a specific router
  policy profile (escalates less often → cheaper).
- **Router version pinning** (v0.4). To prevent routing behavior from drifting due to
  router self-learning, you can pin `model_version` — the agent gets reproducible routing.

---

## 5. Text extraction for routing (multi-turn)

An agent request is not a single string but a system prompt + history + tools. What to
send to the router for classification is configurable:

```toml
[route]
text_strategy = "last_user"     # last_user | concat_all | last_user_plus_system
max_chars     = 4000            # truncate long context (cost/DoS protection)
cache_ttl     = "60s"          # cache routing decisions by text hash
profile       = "balanced"     # default policy profile
```

| Strategy | What is classified | When to use |
|---|---|---|
| `last_user` | last user message | default; cheap and accurate for most agents |
| `last_user_plus_system` | system prompt + last user message | when the role is set by the system prompt |
| `concat_all` | full conversation (truncated) | multi-turn with meaning spread across history |

---

## 6. Failover and circuit breaker

- **Failover.** On 5xx / timeout / `overloaded` from a provider — try the next model in
  the list.
- **Circuit breaker.** After `breaker.threshold` consecutive errors, the model is marked
  "open" for `breaker.cooldown`; requests skip it. Half-open: one probe request after the
  cooldown expires.
- **Router down.** `routing.default` is used; the request does not fail (soft degradation).

```toml
[breaker]
threshold = 5
cooldown  = "30s"
```

---

## 7. Feedback loop (optional, v0.4)

When enabled, the gateway can send a `/v1/feedback` signal to the router after a successful
completion (e.g. based on the result of agent-side response validation) — the router will
fine-tune the task mask. Over time this makes routing fit your specific pool more precisely.

```toml
[feedback]
enabled = false
# the ground-truth source is set by the integrator (e.g. the X-Cortiq-Correct-Label header)
```
