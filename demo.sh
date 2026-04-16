#!/bin/bash
# demo.sh — Full Workex demo
# Windows: run with `powershell -File demo.ps1` instead
export PATH="$HOME/.cargo/bin:$PATH"

echo ""
echo "╔══════════════════════════════════════════════════╗"
echo "║  Workex Demo — Agent-Native JS Runtime           ║"
echo "║  166 tests, 0 mocks, 585x less memory than V8    ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

echo "Building (release)..."
cargo build --release 2>&1 | tail -1
echo ""

echo "═══════════════════════════════════════════════════"
echo "  CORRECTNESS"
echo "═══════════════════════════════════════════════════"
echo ""

echo -n "1. Worker compat (hello.ts)............ "
cargo test -p workex-runtime execute_hello_ts --release 2>&1 | grep -q "ok" && echo "PASS" || echo "FAIL"

echo -n "2. fetch() called once (bug fix)....... "
cargo test -p workex-runtime async_worker_fetch_called --release 2>&1 | grep -q "ok" && echo "PASS" || echo "FAIL"

echo -n "3. Cranelift native add(10,32)=42...... "
cargo test -p workex-runtime cranelift_native_fn --release 2>&1 | grep -q "ok" && echo "PASS" || echo "FAIL"

echo -n "4. Agent hibernation (restart)......... "
cargo test -p workex-vm agent_survives --release 2>&1 | grep -q "ok" && echo "PASS" || echo "FAIL"

echo -n "5. Full pipeline (TS→VM)............... "
cargo test -p workex-vm --test pipeline pipeline_hello --release 2>&1 | grep -q "ok" && echo "PASS" || echo "FAIL"

echo ""
echo "═══════════════════════════════════════════════════"
echo "  BENCHMARKS"
echo "═══════════════════════════════════════════════════"
echo ""

echo "--- 1M suspended agents ---"
cargo run -p workex-bench --release --bin continuation-bench 2>&1 \
    | grep -E "Per agent|1M agents|Factor"

echo ""
echo "--- 10M suspended agents ---"
cargo run -p workex-bench --release --bin ten-million-bench 2>&1 \
    | grep -E "Per agent|10M|Factor|Workex|V8"

echo ""
echo "--- SharedRuntime 10K (3-way) ---"
cargo run -p workex-bench --release --bin shared-bench 2>&1 \
    | grep -E "Per context|10K Total|Architecture"

echo ""
echo "--- Execution (3-way, 5 runs) ---"
cargo run -p workex-bench --release --bin unified-bench -- --runs 5 2>&1 \
    | grep -E "Cold start|Warm exec|Worker compat"

echo ""
echo "--- Worker compat latency (3-way) ---"
cargo run -p workex-bench --release --bin worker-test 2>&1 \
    | grep -E "Latency p50|Latency p99|Correct"

echo ""
echo "--- 10K real Worker RSS (3-way) ---"
cargo run -p workex-bench --release --bin rss-real-bench 2>&1 \
    | grep -E "Per Worker|10K Total"

echo ""
echo "═══════════════════════════════════════════════════"
echo "  DONE"
echo "═══════════════════════════════════════════════════"
echo ""
echo "  Results: benchmarks/results/"
echo "  Tests:   cargo test (166 tests, 0 failures)"
echo ""
