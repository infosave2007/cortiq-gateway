#!/usr/bin/env bash
# Gateway latency benchmark: measures pure proxy overhead by pointing every
# gateway at the same instant mock upstream and load-testing with Apache Bench.
#
#   CONC=20 REQ=10000 bash bench/run.sh
#
# Compares (when installed): mock-direct (baseline), cortiq-gateway, LiteLLM, Portkey.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
CONC="${CONC:-20}"
REQ="${REQ:-10000}"
BODY="$HERE/body.json"
MOCK_PORT=8100
pids=()
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT

ab_run() { # name url [extra ab -H args...]
  local name="$1" url="$2"; shift 2
  local out rps mean p50 p90 p99
  out=$(ab -k -c "$CONC" -n "$REQ" -p "$BODY" -T application/json -s 60 "$@" "$url" 2>/dev/null)
  rps=$(echo "$out" | awk '/Requests per second/{print $4}')
  mean=$(echo "$out" | awk '/Time per request/{print $4; exit}')
  p50=$(echo "$out" | awk '/ 50%/{print $2}')
  p90=$(echo "$out" | awk '/ 90%/{print $2}')
  p99=$(echo "$out" | awk '/ 99%/{print $2}')
  printf "%-16s | %9s | %8s | %6s | %6s | %6s\n" \
    "$name" "${rps:-NA}" "${mean:-NA}" "${p50:-NA}" "${p90:-NA}" "${p99:-NA}"
}

wait_up() { for _ in $(seq 1 150); do curl -s -o /dev/null "$1" && return 0; sleep 0.2; done; return 1; }

echo "concurrency=$CONC  requests=$REQ  (latency in ms; ab -k)"
printf "%-16s | %9s | %8s | %6s | %6s | %6s\n" "gateway" "rps" "mean" "p50" "p90" "p99"
printf -- "-----------------+-----------+----------+--------+--------+--------\n"

# --- mock baseline -----------------------------------------------------------
node "$HERE/mock.js" $MOCK_PORT >/tmp/bench_mock.log 2>&1 & pids+=($!)
wait_up "http://127.0.0.1:$MOCK_PORT/healthz" || { echo "mock failed"; exit 1; }
ab_run "mock-direct" "http://127.0.0.1:$MOCK_PORT/v1/chat/completions"

# --- cortiq-gateway ----------------------------------------------------------
BIN="$ROOT/target/release/cortiq-gateway"
[ -x "$BIN" ] || BIN="$ROOT/target/debug/cortiq-gateway"
"$BIN" --config "$HERE/cortiq.bench.toml" >/tmp/bench_cortiq.log 2>&1 & CG=$!; pids+=($CG)
if wait_up "http://127.0.0.1:9100/healthz"; then
  ab_run "cortiq-gateway" "http://127.0.0.1:9100/v1/chat/completions"
fi
kill $CG 2>/dev/null

# --- LiteLLM -----------------------------------------------------------------
if python3 -c "import litellm" 2>/dev/null; then
  cat > /tmp/litellm.yaml <<YAML
model_list:
  - model_name: bench
    litellm_params:
      model: openai/bench-model
      api_base: http://127.0.0.1:$MOCK_PORT/v1
      api_key: sk-bench
YAML
  # the proxy CLI is brittle across versions; run the ASGI app via uvicorn.
  # 4 workers = LiteLLM's recommended multi-process setup (fair vs a multi-core gateway).
  CONFIG_FILE_PATH=/tmp/litellm.yaml python3 -m uvicorn litellm.proxy.proxy_server:app \
    --host 127.0.0.1 --port 4000 --workers 4 >/tmp/bench_litellm.log 2>&1 & LL=$!; pids+=($LL)
  if wait_up "http://127.0.0.1:4000/health/liveliness"; then
    # warm up the workers before measuring (cold imports per worker)
    for _ in $(seq 1 10); do curl -s -o /dev/null -X POST -H 'content-type: application/json' -d @"$BODY" "http://127.0.0.1:4000/v1/chat/completions"; done
    sleep 2
    ab_run "litellm" "http://127.0.0.1:4000/v1/chat/completions"
  fi
  kill $LL 2>/dev/null
else
  echo "litellm           | (not installed — pip install 'litellm[proxy]')"
fi

# --- Portkey -----------------------------------------------------------------
# resolve relative to the active node (npm root -g may point at a different npm)
PK_MAIN="$(dirname "$(command -v node 2>/dev/null)")/../lib/node_modules/@portkey-ai/gateway/build/start-server.js"
if [ -f "$PK_MAIN" ]; then
  PORT=8787 node "$PK_MAIN" >/tmp/bench_portkey.log 2>&1 & PK=$!; pids+=($PK)
  if wait_up "http://127.0.0.1:8787/v1/health" || wait_up "http://127.0.0.1:8787/"; then
    sleep 1
    ab_run "portkey" "http://127.0.0.1:8787/v1/chat/completions" \
      -H "x-portkey-provider: openai" \
      -H "x-portkey-custom-host: http://127.0.0.1:$MOCK_PORT/v1" \
      -H "Authorization: Bearer sk-bench"
  fi
  kill $PK 2>/dev/null
else
  echo "portkey           | (not installed — npm i -g @portkey-ai/gateway)"
fi

# --- task-type routing accuracy (optional; needs a live router key) ----------
if [ -n "${CORTIQ_ROUTER_KEY:-}" ]; then
  echo
  echo "== task-type routing accuracy (allaigate router vs keyword heuristic) =="
  python3 "$HERE/accuracy.py"
fi

echo
echo "Note: latency numbers are gateway overhead over a zero-latency mock on one"
echo "machine; subtract mock-direct to isolate proxy cost. Re-run on your hardware."
