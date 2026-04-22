#!/usr/bin/env bash
# Top-level build orchestrator for the kenton-response artifacts.
#
# Steps, all re-runnable:
#   1. Fetch + build V8 (skips if libv8_monolith.a already exists)
#   2. Build P1/P2 benchmarks
#   3. Build V8-COW tools (standalone prototype + memory/heap benchmarks)
#
# This script is safe to run top-to-bottom on a fresh Linux x86_64 box.
# First run takes 30-90 min and ~20 GB due to the V8 build.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"

echo "=============================="
echo "1/3  fetch + build V8"
echo "=============================="
if [[ -f "$here/vendor/v8/out/x64.release/obj/libv8_monolith.a" ]]; then
  echo "    already built, skipping"
else
  "$here/benchmarks/fetch_and_build_v8.sh"
fi

echo
echo "=============================="
echo "2/3  P1+P2 benchmarks"
echo "=============================="
"$here/benchmarks/build.sh"

echo
echo "=============================="
echo "3/3  V8-COW tools"
echo "=============================="
"$here/src/v8-cow/build.sh"

echo
echo "all built. next steps:"
echo "  benchmarks:      $here/benchmarks/run.sh"
echo "  v8-cow tools:    $here/src/v8-cow/build/cow_builtins_prototype"
echo "                   $here/src/v8-cow/build/memory_benchmark"
echo "                   $here/src/v8-cow/build/heap_classifier"
echo "  apply patch:     $here/src/v8-cow/apply_patch_and_build.sh"
