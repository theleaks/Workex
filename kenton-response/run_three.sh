#!/usr/bin/env bash
# Run the kenton-bench Docker image 3 times with different seeds, into
# results/run_1, run_2, run_3. Then summarise across all three for
# variance estimates and median headline numbers.
#
# Usage:
#   ./run_three.sh
#   N=5 ./run_three.sh        # different number of runs

set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
N="${N:-3}"

# Detect Windows vs Linux for the docker -v path.
if command -v cygpath >/dev/null 2>&1; then
  RESULTS_BASE="$(cygpath -w "$here/results")"
else
  RESULTS_BASE="$here/results"
fi

mkdir -p "$here/results"

for i in $(seq 1 "$N"); do
  echo
  echo "=========================================="
  echo "  RUN $i / $N  (seed=$i)"
  echo "=========================================="
  rd="$here/results/run_$i"
  rm -rf "$rd"
  mkdir -p "$rd"

  # Path translation for Docker volume mount.
  if command -v cygpath >/dev/null 2>&1; then
    rd_mount="$(cygpath -w "$rd")"
  else
    rd_mount="$rd"
  fi

  docker run --rm \
    -v "$rd_mount:/work/results" \
    -e "KENTON_SEED=$i" \
    kenton-bench
done

echo
echo "=========================================="
echo "  AGGREGATING $N RUNS"
echo "=========================================="

if command -v python3 >/dev/null 2>&1; then
  PY=python3
elif command -v py >/dev/null 2>&1; then
  PY="py -3"
elif [[ -x "/c/Users/premi/AppData/Local/Programs/Python/Python312/python.exe" ]]; then
  PY="/c/Users/premi/AppData/Local/Programs/Python/Python312/python.exe"
else
  echo "no python found — run scripts/summarize.py manually" >&2
  exit 1
fi

$PY "$here/scripts/summarize.py" "$here/results"

echo
echo "done. SUMMARY at: $here/results/SUMMARY.md"
