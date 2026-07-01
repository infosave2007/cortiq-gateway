# Gateway latency benchmark

How much latency does the **gateway itself** add? For an LLM proxy the model call
dominates wall-clock time, but the gateway's own overhead is what differs between
products — and it compounds across the many calls an agent makes. This benchmark
isolates that overhead and compares Cortiq Gateway with **LiteLLM** and **Portkey**.

> Reproduce on your own hardware: `bash bench/run.sh` (see [Methodology](#methodology)).

## TL;DR

Every gateway proxies the **same instant mock backend** in passthrough mode, under
identical load (`ab -k -r -c 20 -n 5000`). Latency is in milliseconds.

| Gateway | Throughput (req/s) | p50 | p90 | p99 | mean |
|---|--:|--:|--:|--:|--:|
| **Cortiq Gateway** (Rust) | **~57,300** | 0 ms | 1 ms | **1 ms** | 0.35 ms |
| Portkey Gateway (Node) | ~5,800 | 3 ms | 5 ms | 9 ms | 3.5 ms |
| LiteLLM (Python, 4 workers) | ~1,200 | 9 ms | 37 ms | 59 ms | 16.6 ms |
| _mock-direct (no gateway)_ | _~29,000_ | _1 ms_ | _1 ms_ | _3 ms_ | _0.7 ms_ |

On this machine Cortiq adds **sub-millisecond p99** overhead and sustains **~10×**
the throughput of Portkey and **~47×** of LiteLLM. The gap is architectural: a Rust
/ Tokio gateway uses every core in one process, with no interpreter or GC in the hot
path.

> Absolute numbers depend on hardware; the **ratios** between gateways are the point.
> Run `bench/run.sh` to get numbers for your environment.

## Accuracy: task-type routing

Speed is only half the story. Cortiq's other edge is **routing by task type** — sending
each prompt to the right model — via the allaigate semantic classifier. LiteLLM and
Portkey have **no semantic task router**: you either pin a model, write brittle rules,
or pay for an extra LLM classification call.

On a 37-prompt, 7-task-type set of **natural-language prompts** (no explicit trigger
words like "translate" / "summarize" — [`bench/tasks.jsonl`](bench/tasks.jsonl)):

| Classifier | Correct | Accuracy |
|---|--:|--:|
| **allaigate semantic router** | 37 / 37 | **100%** |
| keyword heuristic (DIY without a classifier) | 12 / 37 | 32% |

A keyword matcher looks good on prompts that literally say "translate this" — but real
users don't. On natural phrasing (`"'Guten Tag' — what does that mean in English?"`,
`"boil this down to a few key points"`, `"which companies are named in this report?"`)
surface matching collapses while the semantic router still routes correctly.

- **Methodology.** [`bench/accuracy.py`](bench/accuracy.py) sends each prompt to the live
  allaigate router (`taxonomy = data-assistant`) and compares the predicted `task_label`
  to the ground truth; a keyword heuristic is scored on the same set as a lower bound for
  "route without a classifier."
- **Caveats.** Small, hand-labeled set; phrasing is deliberately keyword-light to test
  semantic understanding vs surface matching. Expand `tasks.jsonl` and re-run. allaigate's
  own published figure is **94% at separating close task types** (a harder eval than this one).
- **Reproduce.** `CORTIQ_ROUTER_KEY=cortiq_… python3 bench/accuracy.py`

## Why this matters

Agentic workloads make **many** gateway calls per task (plan → sub-tasks → tool calls
→ synthesis). Overhead that looks small per call (tens of ms) multiplies across a run
and caps the throughput a single instance can serve. Lower, more predictable tail
latency (p99) is the developer-facing differentiator when the model time is equal.

## Methodology

- **Isolate gateway overhead.** A tiny, dependency-free Node mock ([`bench/mock.js`](bench/mock.js))
  returns a fixed OpenAI-compatible completion **instantly**. So measured latency is
  the gateway's own cost, not model time. Every gateway proxies this same mock.
- **Apples-to-apples.** Each gateway runs in **passthrough / pinned** mode (no routing
  or model-selection logic) so we compare raw proxy cost. Cortiq's intelligent routing
  (`cortiq-auto`) adds one classifier round-trip — that's a feature, measured separately,
  not folded into this overhead number.
- **Load.** Apache Bench, keep-alive, tolerant of length variance:
  `ab -k -r -c 20 -n 5000 -p bench/body.json -T application/json`.
- **Fair to each runtime.** Cortiq = one process (Tokio, multi-core). Portkey = one Node
  process (event loop). LiteLLM = **4 uvicorn workers** (its recommended multi-process
  setup; a single Python worker is GIL-bound to one core and scores ~860 req/s).
- **Same box, localhost**, warm processes, results are representative of one run.

### Caveats (read me)

- This measures **gateway overhead only**. Real requests are dominated by model latency,
  which is identical regardless of gateway — but overhead and tail latency are not.
- Localhost + mock backend removes network/model variance on purpose. Your absolute
  numbers will differ; reproduce with `bench/run.sh`.
- `ab` reports LiteLLM "Failed requests" as a non-zero count — these are **response
  content-length variations, not HTTP errors** (all responses are `200`, verified).
- Tested versions: Cortiq Gateway (this repo, `--release`), LiteLLM proxy via
  `uvicorn litellm.proxy.proxy_server:app --workers 4`, Portkey `@portkey-ai/gateway`,
  Apache Bench. Hardware: Apple Silicon, macOS.

## Reproduce

```bash
# prerequisites: node, ab (Apache Bench), a release build of the gateway
cargo build --release

# optional competitors (the harness auto-detects them):
pip install 'litellm[proxy]' uvicorn
npm install -g @portkey-ai/gateway

# run (tune load via env)
CONC=20 REQ=5000 bash bench/run.sh
```

The harness ([`bench/run.sh`](bench/run.sh)) starts the mock, then benchmarks the mock
directly and each available gateway, printing a throughput / p50 / p90 / p99 table.
