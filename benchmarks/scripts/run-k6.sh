#!/usr/bin/env bash
# k6 Load Test Orchestrator
# Starts 3 servers, runs k6 against each, saves results, prints comparison.
#
# Usage: bash benchmarks/scripts/run-k6.sh
#
# Results saved to: benchmarks/results/k6/vN-{workex,node,workers}.json

set -e

K6="/c/Program Files/k6/k6.exe"
SCRIPTS_DIR="benchmarks/scripts"
RESULTS_DIR="benchmarks/results/k6"
mkdir -p "$RESULTS_DIR"

# Auto-detect next version
LAST=$(ls "$RESULTS_DIR" 2>/dev/null | grep -oP 'v\K[0-9]+' | sort -n | tail -1)
VERSION="v$((${LAST:-0} + 1))"

echo ""
echo "+======================================================+"
echo "|  k6 Load Test — 3-Way Comparison                     |"
echo "|  Version: $VERSION                                         |"
echo "+======================================================+"
echo ""

# ── Cleanup on exit ──
PIDS=()
cleanup() {
  echo ""
  echo "Stopping servers..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait "${PIDS[@]}" 2>/dev/null || true
  echo "Done."
}
trap cleanup EXIT

# ── Start servers ──
echo "[1/3] Starting Workex server on :3001..."
cargo run -p workex-cli --release --bin workex-server -- 3001 2>&1 &
PIDS+=($!)

echo "[2/3] Starting Node.js server on :3002..."
node "$SCRIPTS_DIR/node-server.mjs" 3002 2>&1 &
PIDS+=($!)

echo "[3/3] Starting wrangler dev on :3003..."
(cd "$SCRIPTS_DIR/worker" && npx wrangler dev --port 3003 --ip 127.0.0.1 2>&1) &
PIDS+=($!)

# ── Wait for readiness ──
echo ""
echo "Waiting for servers..."
for entry in "3001:Workex" "3002:Node.js" "3003:Workers"; do
  port="${entry%%:*}"
  name="${entry##*:}"
  for i in $(seq 1 30); do
    if curl -s "http://127.0.0.1:$port/health" > /dev/null 2>&1; then
      echo "  $name (:$port) ready"
      break
    fi
    sleep 1
    if [ "$i" -eq 30 ]; then echo "  $name (:$port) TIMEOUT — skipping"; fi
  done
done
echo ""

# ── Run k6 ──
run_k6() {
  local name=$1 port=$2 label=$3
  local outfile="$RESULTS_DIR/${VERSION}-${name}.json"

  echo "+-----------------------------------------+"
  echo "|  k6: $label"
  echo "+-----------------------------------------+"

  "$K6" run \
    -e TARGET="http://127.0.0.1:$port" \
    -e OUTPUT="$outfile" \
    "$SCRIPTS_DIR/k6-test.js" 2>&1 | grep -E "running|iteration|✓|✗|http_req|checks|vus"

  echo ""
}

run_k6 "workex"  3001 "Workex (port 3001)"
run_k6 "node"    3002 "Node.js V8 (port 3002)"
run_k6 "workers" 3003 "CF Workers (port 3003)"

# ── Print comparison table ──
echo "+================================================================================================+"
echo "|                        k6 RESULTS — $VERSION                                                    |"
echo "+================================================================================================+"
echo ""

node -e "
const fs = require('fs');
const dir = '$RESULTS_DIR';
const v = '$VERSION';

function load(name) {
  try { return JSON.parse(fs.readFileSync(dir + '/' + v + '-' + name + '.json', 'utf8')); }
  catch { return null; }
}

const w = load('workex'), n = load('node'), c = load('workers');
const hdr = (s, pad) => s.toString().padStart(pad);

console.log('  Endpoint/Pct  Workex(ms)   Node(ms) Workers(ms)    vs V8 vs Workers');
console.log('  ' + '-'.repeat(70));

for (const ep of ['health','json','compute','hello']) {
  for (const p of ['p50','p95','p99']) {
    const key = ep + '_' + p;
    const wv = w?.metrics?.[key], nv = n?.metrics?.[key], cv = c?.metrics?.[key];
    const fmt = v => typeof v === 'number' ? v.toFixed(2) : '-';
    const fac = (base, other) => (typeof base === 'number' && typeof other === 'number' && base > 0)
      ? (other/base).toFixed(1)+'x' : '-';
    console.log('  ' + (ep+'/'+p).padEnd(14) + hdr(fmt(wv),10) + hdr(fmt(nv),11) + hdr(fmt(cv),12) + hdr(fac(wv,nv),9) + hdr(fac(wv,cv),10));
  }
}

console.log('');
const fmt0 = v => typeof v === 'number' ? Math.round(v) : '-';
console.log('  RPS           ' + hdr(fmt0(w?.metrics?.rps),10) + hdr(fmt0(n?.metrics?.rps),11) + hdr(fmt0(c?.metrics?.rps),12));
console.log('  Requests      ' + hdr(fmt0(w?.metrics?.total_requests),10) + hdr(fmt0(n?.metrics?.total_requests),11) + hdr(fmt0(c?.metrics?.total_requests),12));
const errFmt = v => typeof v === 'number' ? (v*100).toFixed(2)+'%' : '-';
console.log('  Errors        ' + hdr(errFmt(w?.metrics?.error_rate),10) + hdr(errFmt(n?.metrics?.error_rate),11) + hdr(errFmt(c?.metrics?.error_rate),12));
"

echo ""
echo "Results saved to:"
ls -la "$RESULTS_DIR/${VERSION}"*.json 2>/dev/null
echo ""
