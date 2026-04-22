#!/usr/bin/env bash
# Run P1 and P2 benchmarks, collect results into results/.
#
# Usage:
#   ./run.sh            # run both
#   ./run.sh p1         # just P1
#   ./run.sh p2 A       # P2 config A only

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
results="$here/results"
build="$here/build"
mkdir -p "$results"

which="${1:-all}"
shift || true
arg="${1:-}"

# Record environment for reproducibility.
{
  echo "date: $(date -u +%FT%TZ)"
  echo "host: $(uname -a)"
  if command -v lscpu >/dev/null 2>&1; then
    lscpu | grep -E 'Model name|CPU MHz|L3 cache' || true
  fi
  if [[ -r /proc/meminfo ]]; then
    grep MemTotal /proc/meminfo || true
  fi
} > "$results/env.txt"

if [[ "$which" == "all" || "$which" == "p1" ]]; then
  echo "=== P1: serialization cost ==="
  "$build/p1_serialization_cost" $arg > "$results/p1.json"
  echo "  -> $results/p1.json"
fi

if [[ "$which" == "all" || "$which" == "p2" ]]; then
  echo "=== P2: resume cost ==="
  "$build/p2_resume_cost" $arg > "$results/p2.json"
  echo "  -> $results/p2.json"
fi

echo
echo "done."
