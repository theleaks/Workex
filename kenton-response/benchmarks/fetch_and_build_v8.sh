#!/usr/bin/env bash
# Fetch V8 and build the monolithic static lib required by the P1/P2
# benchmarks. Linux + x86_64 only — macOS works but gn args below are
# Linux-tuned.
#
# This takes 30-90 minutes and ~20 GB of disk. Run once per machine.
#
# References:
#   https://v8.dev/docs/source-code
#   https://v8.dev/docs/embed
#
# V8_REV can be pinned to a specific release branch or tag. We default to
# a stable branch-head. If the workerd version you care about uses a
# specific V8 revision, override V8_REV to match.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
vendor="$here/../vendor"
mkdir -p "$vendor"

V8_REV="${V8_REV:-branch-heads/12.8}"
V8_DIR="${V8_DIR:-$vendor/v8}"
DEPOT_TOOLS_DIR="${DEPOT_TOOLS_DIR:-$vendor/depot_tools}"

echo "=== fetching depot_tools ==="
if [[ ! -d "$DEPOT_TOOLS_DIR" ]]; then
  git clone --depth=1 \
    https://chromium.googlesource.com/chromium/tools/depot_tools.git \
    "$DEPOT_TOOLS_DIR"
fi
export PATH="$DEPOT_TOOLS_DIR:$PATH"

echo "=== fetching V8 ($V8_REV) ==="
mkdir -p "$(dirname "$V8_DIR")"
cd "$(dirname "$V8_DIR")"
if [[ ! -d "$V8_DIR/.git" ]]; then
  # fetch uses gclient under the hood; picks up all runtime deps.
  fetch --nohooks v8
fi

cd "$V8_DIR"
git fetch origin "$V8_REV"
git checkout FETCH_HEAD
gclient sync -D

echo "=== installing V8 build deps (requires sudo; skip with SKIP_DEPS=1) ==="
if [[ "${SKIP_DEPS:-0}" != "1" ]]; then
  ./build/install-build-deps.sh --no-chromeos-fonts --no-arm --no-nacl \
    --no-backwards-compatible || {
      echo "install-build-deps.sh failed; retry with SKIP_DEPS=1 if your" >&2
      echo "distro is non-Debian/Ubuntu." >&2
      exit 1
    }
fi

echo "=== generating build config ==="
gn gen out/x64.release --args='
is_debug=false
target_cpu="x64"
v8_monolithic=true
v8_use_external_startup_data=false
use_custom_libcxx=false
is_component_build=false
v8_enable_sandbox=true
treat_warnings_as_errors=false
'

echo "=== building v8_monolith (this is the long step) ==="
ninja -C out/x64.release v8_monolith

echo
echo "done — monolith at:"
echo "  $V8_DIR/out/x64.release/obj/libv8_monolith.a"
echo
echo "next: cd $here && ./build.sh"
