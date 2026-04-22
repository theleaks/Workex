#!/usr/bin/env bash
# Run all benchmarks and collect results into results/.
#
# Prereq: ./build_all.sh has completed.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
results="$here/results"
mkdir -p "$results"

echo "=== P1: serialization cost (seed=${KENTON_SEED:-0}) ==="
"$here/benchmarks/build/p1_serialization_cost" > "$results/p1.json"
echo "  -> $results/p1.json"

echo
echo "=== P2: resume cost — sweep all (config × workload), seed=${KENTON_SEED:-0} ==="
"$here/benchmarks/build/p2_resume_cost" > "$results/p2.json"
echo "  -> $results/p2.json"

echo
echo "=== P3-baseline: memory benchmark (stock V8) ==="
"$here/src/v8-cow/build/memory_benchmark" "$results/p3_memory.json" \
  | tee "$results/p3_memory.txt"

echo
echo "=== P3-baseline: heap classifier ==="
"$here/src/v8-cow/build/heap_classifier" "$results/p3_heap_snapshot.json" \
  | tee "$results/p3_heap_classifier.txt"

echo
echo "=== P3-standalone: COW prototype self-check ==="
"$here/src/v8-cow/build/cow_builtins_prototype" \
  2>&1 | tee "$results/p3_prototype.txt"

# Environment snapshot for reproducibility.
{
  echo "date: $(date -u +%FT%TZ)"
  echo "host: $(uname -a)"
  if command -v lscpu >/dev/null 2>&1; then lscpu; fi
  if [[ -r /proc/meminfo ]]; then grep MemTotal /proc/meminfo; fi
  if [[ -d "$here/vendor/v8/.git" ]]; then
    echo "v8: $(git -C "$here/vendor/v8" rev-parse HEAD)"
    echo "v8-desc: $(git -C "$here/vendor/v8" describe --all 2>/dev/null || true)"
  fi
} > "$results/env.txt"

echo
echo "=== aggregating into SUMMARY.md ==="
python3 "$here/scripts/summarize.py" "$results" || echo "(summarize.py failed — JSON files still in $results/)"

echo
echo "results written to $results/"
ls -la "$results/"
