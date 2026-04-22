# What's been tested, and how

Before sending this work anywhere, every piece that could be tested in
a non-Linux non-V8 sandbox has been tested. Recording the receipts so
the user (and Kenton) can verify.

## What was tested here (Windows / MSVC sandbox, no V8 built)

### 1. Shell scripts — `bash -n` syntax check

All seven scripts pass `bash -n`:

```
build_all.sh                                OK
run_all.sh                                  OK
benchmarks/build.sh                         OK
benchmarks/run.sh                           OK
benchmarks/fetch_and_build_v8.sh            OK
src/v8-cow/build.sh                         OK
src/v8-cow/apply_patch_and_build.sh         OK
```

### 2. Standalone COW prototype — compiled and run

`src/v8-cow/cow_builtins_prototype.cc`:

```
shared: installed 30 built-ins, sealed.
summary:
  isolates:                 1000
  shared built-ins (once):  1920 B
  naive per-isolate cost:   1920 B
  naive total:              1920000 B
  COW total:                8320 B
  savings:                  99.57%
  reads: 100000  writes: 100
```

Result is arithmetically correct: 30 shared objects × 64 B = 1920 B;
100 promotions (1 per 10 isolates) × 64 B = 6400 B; 1920 + 6400 = 8320.

### 3. Stats + JSON utilities — unit-tested

8 tests in `benchmarks/_self_test.cc`, all pass:

- `test_stats_empty` — returns 0-initialized Stats
- `test_stats_single` — mean/median/p99 = sample
- `test_stats_many` — 1..100 → mean 50.5, min 1, max 100, median 51,
  p99 100 (all matching the formula)
- `test_stats_unordered` — handles out-of-order input via internal sort
- `test_nownanos_monotonic` — `NowNanos()` is non-decreasing
- `test_json_empty` — produces `{}\n`
- `test_json_flat` — key/value pairs correctly comma-separated
- `test_json_stats` — stats struct serializes with all six fields

### 4. C++ sources — syntax-checked against real V8 headers

Downloaded the V8 `include/` directory from
`chromium.googlesource.com/v8/v8.git/+archive/refs/heads/12.8-lkgr/include.tar.gz`
(285 KB) and compiled every V8-dependent `.cc` in syntax-only mode
(`cl /Zs`) against it:

```
p1_serialization_cost.cc      EXIT=0   (clean)
p2_resume_cost.cc             EXIT=0   (clean)
common.cc                     EXIT=0   (clean)
memory_benchmark.cc           EXIT=0   (2 MSVC fopen/getenv deprecation warnings, not bugs)
heap_classifier.cc            EXIT=0   (1 MSVC fopen deprecation warning, not bug)
cow_builtins_prototype.cc     EXIT=0   (no V8 dep)
```

This validates that every V8 API I call (`ValueSerializer::WriteValue`,
`ValueDeserializer::ReadValue`, `Isolate::New`, `Context::New`,
`Script::Compile`, `EscapableHandleScope`, `HeapProfiler`,
`HeapSnapshot`, `OutputStream`, etc.) has the expected signature in
the targeted V8 version.

The reproduction: `benchmarks/_syntax_check.bat` and
`src/v8-cow/_syntax_check.bat`. They're not in the ship path; they're
here so anyone can re-run them. Both require Visual Studio 2022 and
the headers cached in `vendor/v8-headers-test/` (download recipe
above).

### 5. `patch_skeleton.diff` — applies cleanly

Created an empty git repo, ran `git apply --check patch_skeleton.diff`
→ OK. Then `git apply` → added `src/heap/cow-shared-builtins.{h,cc}`
with the expected content. Hunk line counts verified to match the
`@@ -0,0 +1,N @@` declarations.

## What was NOT tested here (can't be, without Linux + V8)

- **P1 runtime behavior:** needs V8. Code compiles cleanly against
  real V8 headers (see #4), so it's well-formed. Runtime correctness
  (e.g., does `WriteValue` actually round-trip for all five workload
  sizes, does it handle all the types in the L/XL workloads) will
  only be known after `./build_all.sh && ./run_all.sh` on Linux.
- **P2 runtime behavior:** same story. Additionally, the isolation
  checks (VerifyIsolationA/B/C) only prove correctness at runtime.
- **Memory benchmark / heap classifier:** same.
- **The full V8 patch:** `UNIMPLEMENTED()` by design. Verified only
  that the additive skeleton patch applies, not that it compiles into
  V8 (that's a V8 build, too heavy for the sandbox).

## Repro receipts

```
# standalone prototype
cd src/v8-cow && cmd /c _compile_test.bat && ./cow_prototype.exe

# stats+json self-test
cd benchmarks && cl /nologo /std:c++17 /EHsc /Fe:_self_test.exe _self_test.cc && ./_self_test.exe

# syntax-check against V8 headers
cd benchmarks && ./_syntax_check.bat
cd src/v8-cow && ./_syntax_check.bat

# patch apply test
git -C /tmp/test-repo apply --check path/to/patch_skeleton.diff
```
