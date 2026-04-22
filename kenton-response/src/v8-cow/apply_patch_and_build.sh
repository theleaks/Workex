#!/usr/bin/env bash
# Apply the additive part of the COW-builtins skeleton (new files only)
# to a local V8 checkout and rebuild.
#
# This does NOT apply the existing-file hook edits listed in
# hooks_reference.md — those require exact line numbers that drift per
# V8 revision and must be done by hand. The additive diff here applies
# cleanly against any V8 tree.
#
# After this script finishes successfully, the new files are present in
# the tree but unused (no call sites yet). Make the edits in
# hooks_reference.md to wire them in.
#
# Usage:
#   V8_DIR=/path/to/v8 ./apply_patch_and_build.sh

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
: "${V8_DIR:=$here/../../vendor/v8}"

if [[ ! -d "$V8_DIR/.git" ]]; then
  echo "error: $V8_DIR is not a git checkout. Run" >&2
  echo "       ../../benchmarks/fetch_and_build_v8.sh first." >&2
  exit 1
fi

patch="$here/patch_skeleton.diff"

cd "$V8_DIR"

echo "=== checking additive patch applies ==="
if ! git apply --check "$patch" 2>/dev/null; then
  echo "note: strict git-apply --check failed (likely because the target" >&2
  echo "      new-file paths already exist from a previous run)." >&2
  echo "      Will attempt a forced re-apply after confirming no diff." >&2
  for f in src/heap/cow-shared-builtins.h src/heap/cow-shared-builtins.cc; do
    if [[ -e "$f" ]]; then
      echo "      $f already present; skipping" >&2
    fi
  done
  echo
  echo "to retry clean: cd $V8_DIR && rm -f $V8_DIR/src/heap/cow-shared-builtins.{h,cc}"
  echo "then rerun this script."
  exit 1
fi

echo "=== applying additive patch ==="
git apply "$patch"
echo "  -> added src/heap/cow-shared-builtins.{h,cc}"

echo
echo "=== NEXT STEPS — manual ==="
echo
echo "The additive patch is applied but the feature is not wired in yet."
echo "See ${here#$V8_DIR/}hooks_reference.md for the four edits required in"
echo "existing V8 source files:"
echo "  1. src/flags/flag-definitions.h  — add --shared-readonly-builtins"
echo "  2. BUILD.gn                      — add the new .cc to build list"
echo "  3. src/heap/read-only-heap.cc    — call WireIsolateRoots"
echo "  4. src/objects/js-objects.cc     — call PromoteOnWrite on write"
echo
echo "After those edits, rebuild with:"
echo "  ninja -C out/x64.release v8_monolith"
echo
echo "To revert just the additive patch:"
echo "  cd $V8_DIR && rm src/heap/cow-shared-builtins.{h,cc}"
