#!/bin/bash
# demo.sh — Workex in 5 minutes
set -e

echo ""
echo "╔══════════════════════════════════════════════╗"
echo "║  Workex Demo — Agent-Native JS Runtime       ║"
echo "╚══════════════════════════════════════════════╝"
echo ""

echo "Building..."
cargo build --release 2>/dev/null
echo ""

echo "1. Bug fix: async fetch() called exactly once"
cargo test -p workex-runtime async_worker_fetch_called_once --release -q 2>/dev/null
echo "   PASS"

echo ""
echo "2. Worker compatibility: hello.ts runs on Workex"
cargo test -p workex-runtime execute_hello_ts --release -q 2>/dev/null
echo "   PASS"

echo ""
echo "3. Agent hibernation: survives server restart"
cargo test -p workex-vm agent_survives --release -q 2>/dev/null
echo "   PASS"

echo ""
echo "4. 1M suspended agents"
cargo run -p workex-bench --release --bin continuation-bench 2>/dev/null \
    | grep -E "Per agent|1M agents|Factor"

echo ""
echo "5. 10M suspended agents"
cargo run -p workex-bench --release --bin ten-million-bench 2>/dev/null \
    | grep -E "Per agent|10M|Factor|Workex|V8"

echo ""
echo "6. Execution performance (3-way)"
cargo run -p workex-bench --release --bin unified-bench -- --runs 3 2>/dev/null \
    | grep -E "Cold start|Warm exec|Worker compat"

echo ""
echo "All results saved to benchmarks/results/"
echo ""
echo "Total tests:"
cargo test --release -q 2>/dev/null | tail -1
