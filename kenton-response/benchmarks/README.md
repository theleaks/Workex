# P1 + P2 Benchmarks

Measures, on V8 directly:

- **P1** — CPU cost of serializing and deserializing JavaScript live state
  across an I/O boundary, using `v8::ValueSerializer`. Answers Kenton's
  "serializing and deserializing everything on every I/O" question.
- **P2** — CPU cost of setting up an execution environment before a
  continuation can resume, under three isolation strategies. Answers
  Kenton's "set up a new execution environment... pretty expensive"
  question.

No Rust, no Workex. The benchmarks link against a locally-built V8
monolithic static library.

## Prereqs

- Linux x86_64 (tested on Ubuntu 22.04 / 24.04). macOS likely works but
  `install-build-deps.sh` is Linux-specific.
- ~20 GB free disk
- CMake 3.16+, Ninja, Python 3, git
- A few hours of patience for the V8 build on first run

## One-time: fetch and build V8

```bash
cd benchmarks
./fetch_and_build_v8.sh
```

What it does:

1. Clones `depot_tools` into `../vendor/depot_tools`
2. Uses `fetch v8` → `../vendor/v8`
3. Checks out `branch-heads/12.8` (override with `V8_REV=…`)
4. Runs `install-build-deps.sh` (will prompt for sudo; skip with
   `SKIP_DEPS=1` if on a non-Debian distro and you handle deps yourself)
5. `gn gen` with `v8_monolithic=true v8_use_external_startup_data=false`
6. `ninja -C out/x64.release v8_monolith`

Result: `../vendor/v8/out/x64.release/obj/libv8_monolith.a`.

## Build the benchmarks

```bash
./build.sh
```

Emits `build/p1_serialization_cost` and `build/p2_resume_cost`.

## Run

```bash
./run.sh             # both
./run.sh p1          # just P1
./run.sh p1 M        # just P1, workload M
./run.sh p2 A        # just P2, config A
```

Output lands in `results/`:

- `results/p1.json` — array of per-workload records
- `results/p2.json` — array of per-config records
- `results/env.txt` — host + CPU + memory snapshot

## What you'll see

For P1, per size class (XS..XL):

```json
{
  "workload": "M",
  "target_bytes": 5120,
  "serialized_bytes": ...,
  "serialize_ns":   { "mean": ..., "median": ..., "p99": ... },
  "deserialize_ns": { ... },
  "roundtrip_ns":   { ... }
}
```

For P2, per config (A/B/C):

```json
{
  "config": "A",
  "isolation_safe": true,
  "isolation_check_passed": true,
  "setup_ns":             { ... },
  "deserialize_ns":       { ... },
  "first_instruction_ns": { ... },
  "total_resume_ns":      { ... }
}
```

Config C is marked `isolation_safe: false` by design — we keep it in the
output because it's the optimistic upper bound people often cite, and we
want the gap between it and A/B to be visible.

## What we deliberately don't do

- Claim Workex is faster. These are V8 numbers measuring V8's own
  serialize/deserialize primitives. Any continuation-style scheme on
  workerd would pay at least this much.
- Cherry-pick. All five size classes are reported. All three configs
  are reported.
- Hide the fresh-isolate cost. Config A is the honest number for strict
  multi-tenant isolation — it's in the output, even though it's big.

## What's NOT fair to claim from these numbers

- `ValueSerializer` is V8's own fast path. A workerd-specific
  serializer could be faster or slower; we don't have that data.
- The M/L/XL workloads are our best guess at realistic agent state;
  there's a subjective gap here. Results scale cleanly with
  `serialized_bytes` though, so readers can re-project for their own
  workload sizes.
- No network I/O. Only the CPU work on top of it.
