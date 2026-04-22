# kenton-response — answering workerd#6595

A self-contained response to Kenton Varda's three technical objections
to the workerd#6595 issue. Every measurement is on V8 directly (C++
linked against `libv8_monolith.a`); no Rust, no Workex runtime, no
"981x" claims.

## Status

- [x] P1 — Serialization cost per I/O (benchmark built + run; results in `results/p1.json`)
- [x] P2 — Resume cost with fresh execution env (benchmark built + run; results in `results/p2.json`)
- [x] P3 — COW built-ins: RFC + baselines + standalone prototype + skeleton patch (`src/v8-cow/`)
- [x] P3 memory baselines measured (`results/p3_memory.json`, heap classifier)
- [x] PR / RFC drafts with real numbers (`PR_DRAFTS/workerd-issue-6595-response-FINAL.md`)
- [ ] Actual V8 patch implementation — pending v8-dev@ go/no-go

## Headline numbers (V8 12.8.374.38, 16-core Intel Ultra 9 285H)

- **P1 serialize round-trip:** XS 1.09 µs, M (5 KB) 11.37 µs, XL (500 KB) 1.06 ms.
- **P2 resume setup:** fresh isolate 1.73 ms; pooled isolate 303 µs; pooled context 64 ns (not iso-safe).
- **P3 per-isolate heap** (hello, 1000 isolates): 10.4 KB used / ~588 KB RSS.
- **COW upper bound:** ~11% of per-isolate used heap is cleanly shareable.

See `results/SUMMARY.md` for the full tables and `PR_DRAFTS/workerd-issue-6595-response-FINAL.md` for the issue comment draft.

## Files

| Path | Status |
|---|---|
| `PLAN.md` | done — three-problem breakdown |
| `docs/P1_METHODOLOGY.md` | done — serialization-cost measurement spec |
| `docs/P2_METHODOLOGY.md` | done — resume-cost measurement spec |
| `docs/P3_DESIGN.md` | done — V8 COW builtins prototype design |
| `benchmarks/p1_serialization_cost.cc` | done — C++ + V8 |
| `benchmarks/p2_resume_cost.cc` | done — C++ + V8 |
| `benchmarks/common.{h,cc}` | done — shared workload/stats/V8 init |
| `benchmarks/CMakeLists.txt` + scripts | done |
| `src/v8-cow/RFC.md` | done — for v8-dev@ |
| `src/v8-cow/cow_builtins_prototype.cc` | done — no V8 dep |
| `src/v8-cow/memory_benchmark.cc` | done — stock-V8 baseline |
| `src/v8-cow/heap_classifier.cc` | done — heap-composition stages |
| `src/v8-cow/patch_skeleton.diff` | done — flag + class + hook locations |
| `PR_DRAFTS/` | done — workerd comment, v8-dev@ RFC, workerd PR |

## Quickstart

On a Linux x86_64 box with ~20 GB free:

```bash
./build_all.sh    # fetch V8 (30-90 min, one-time) + build benchmarks
./run_all.sh      # run everything, collect results/ JSON
```

Partial runs:

```bash
# just the standalone COW prototype (no V8 required)
cd src/v8-cow && ./build.sh && ./build/cow_builtins_prototype

# just P1
cd benchmarks && ./build.sh && ./run.sh p1

# just P2 config A
cd benchmarks && ./run.sh p2 A
```

## Why this folder, not inside Workex

Kenton rejected the alternative-JS-engine direction. Keeping the work
in Workex's source tree would look like we're still trying to sell
Workex. By working here, each artefact stands alone:

- P1 + P2 benchmarks measure V8's own `ValueSerializer` and
  isolate-creation costs. They're about V8, not about Workex.
- P3 is a V8 proposal + prototype, unrelated to any Rust runtime.
- The PRs go to V8 (for P3) or workerd (for a benchmark harness), not
  to our own repo.

## Execution order

1. P1 — `benchmarks/p1_serialization_cost.cc`. Uses
   `v8::ValueSerializer` / `ValueDeserializer` to measure the CPU floor
   for continuation-style suspend/resume on V8. Five workload sizes
   (XS/S/M/L/XL).
2. P2 — `benchmarks/p2_resume_cost.cc`. Measures fresh-env resume cost
   in three configurations (fresh isolate; pooled isolate + fresh
   context; pooled context). Each has an isolation leak test. Config C
   is explicitly labeled `isolation_safe: false`.
3. P3 — `src/v8-cow/`:
   - `memory_benchmark.cc` and `heap_classifier.cc` establish the
     baseline stock V8 already pays per isolate.
   - `cow_builtins_prototype.cc` validates the shared-slab + promote
     data structure in standalone C++ (no V8 dep).
   - `patch_skeleton.diff` shows where the real V8 patch would hook in.
   - `RFC.md` is the design for v8-dev@.
4. Drafts — `PR_DRAFTS/`:
   - `workerd-issue-6595-response.md` — comment on the issue
   - `v8-dev-rfc.md` — email to v8-dev@
   - `workerd-benchmark-harness-pr.md` — optional benchmark PR

## Next action

Read `PLAN.md`, then `docs/P1_METHODOLOGY.md` and
`docs/P2_METHODOLOGY.md` for what is measured. Skim `src/v8-cow/RFC.md`
for the V8 proposal.

To actually produce numbers:

```bash
./build_all.sh && ./run_all.sh
```

To post the response, see `PR_DRAFTS/README.md`.
