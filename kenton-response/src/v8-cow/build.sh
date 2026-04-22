#!/usr/bin/env bash
# Build the V8-COW artifacts:
#   - cow_builtins_prototype (standalone, no V8)
#   - memory_benchmark      (needs V8 monolith)
#   - heap_classifier       (needs V8 monolith)
#
# Usage:
#   ./build.sh                  # uses $V8_DIR env var if set
#   V8_DIR=/opt/v8 ./build.sh

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
: "${V8_DIR:=$here/../../vendor/v8}"

build_dir="$here/build"
mkdir -p "$build_dir"

if [[ -f "$V8_DIR/out/x64.release/obj/libv8_monolith.a" ]]; then
  echo "[build.sh] V8 found at $V8_DIR — building all targets"
  cmake -S "$here" -B "$build_dir" \
        -DV8_DIR="$V8_DIR" -DCMAKE_BUILD_TYPE=Release
else
  echo "[build.sh] V8 not found — building only the standalone prototype"
  echo "  (run ../../benchmarks/fetch_and_build_v8.sh to enable V8 targets)"
  cmake -S "$here" -B "$build_dir" -DCMAKE_BUILD_TYPE=Release
fi

cmake --build "$build_dir" -j"$(nproc 2>/dev/null || echo 4)"

echo
echo "built:"
for t in cow_builtins_prototype memory_benchmark heap_classifier; do
  [[ -x "$build_dir/$t" ]] && echo "  $build_dir/$t"
done
