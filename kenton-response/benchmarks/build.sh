#!/usr/bin/env bash
# Build the P1/P2 benchmarks against a local V8 checkout.
#
# Prereq: V8 checkout built via ./fetch_and_build_v8.sh (or equivalent).
# On first run, invoke fetch_and_build_v8.sh — takes 30-90 min and ~20 GB.
#
# Usage:
#   ./build.sh                     # uses $V8_DIR env var
#   V8_DIR=/opt/v8 ./build.sh      # explicit
#
# The benchmarks link against libv8_monolith.a statically. No shared-lib
# path fiddling at runtime.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
V8_DIR="${V8_DIR:-$here/../vendor/v8}"

if [[ ! -f "$V8_DIR/out/x64.release/obj/libv8_monolith.a" ]]; then
  cat >&2 <<EOF
error: V8 monolith lib not found.

  expected: $V8_DIR/out/x64.release/obj/libv8_monolith.a

Run ./fetch_and_build_v8.sh first, or point V8_DIR at an existing V8
checkout that has been built with v8_monolithic=true.
EOF
  exit 1
fi

build_dir="$here/build"
mkdir -p "$build_dir"
cmake -S "$here" -B "$build_dir" -DV8_DIR="$V8_DIR" -DCMAKE_BUILD_TYPE=Release
cmake --build "$build_dir" -j"$(nproc 2>/dev/null || echo 4)"

echo
echo "built:"
echo "  $build_dir/p1_serialization_cost"
echo "  $build_dir/p2_resume_cost"
