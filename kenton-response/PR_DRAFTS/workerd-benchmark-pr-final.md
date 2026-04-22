# workerd PR: Add per-isolate cost benchmark harness

**Target:** cloudflare/workerd:main
**Size:** ~600 LOC C++ + Bazel config
**Risk:** None (additive, behind `bazel test //tests/benchmarks:*` only)
**Issue:** cloudflare/workerd#6595

---

## PR title

`Add benchmark harness for per-isolate serialization and resume cost`

## PR body

Adds two benchmark tools under `src/workerd/tests/benchmarks/` for
measuring (a) live-state serialize/deserialize round-trip cost and
(b) fresh-execution-environment resume cost in three isolation
configurations. These answer questions that came up on #6595 that
the repo didn't have tools to address.

### What's new

**`src/workerd/tests/benchmarks/p1_serialization_cost.cc`** (≈200 LOC)

Measures V8 `ValueSerializer` round-trip cost for five live-state
sizes (XS 50 B → XL 500 KB) using per-iteration unique seed (so
inline caches don't go hot — realistic "every continuation has
different state" case). Reports mean / median / p99 / min / max for
serialize, deserialize, round-trip, plus the build cost (fresh
object construction per iteration) as context.

**`src/workerd/tests/benchmarks/p2_resume_cost.cc`** (≈260 LOC)

Sweeps fresh-env resume cost across state sizes × three isolation
configs:

- **A** — fresh `Isolate` + `Context` per resume (strict multi-tenant)
- **B** — pooled Isolate, fresh Context per resume (warm reuse)
- **C** — pooled Context (NOT isolation-safe — verified by explicit
  leak test; the benchmark labels C's numbers accordingly)

For each (config, workload) tuple: `setup_ns`, `deserialize_ns`,
`first_instruction_ns`, `total_resume_ns` with mean / median / p99.

**`src/workerd/tests/benchmarks/memory_benchmark.cc`** (≈180 LOC)

Per-isolate RSS measurement via `/proc/self/smaps_rollup` + linear
regression across (1, 10, 50, 100, 500, 1000) isolate counts. Reports
per-isolate slope with R² (R²=1.000 expected — regression is very
clean across this range).

**`src/workerd/tests/benchmarks/heap_classifier.cc`** (≈100 LOC)

Heap-composition stages: post-isolate, post-context, post-minimal-JS,
post-realistic-handler. Computes the "COW upper bound" — how much of
per-isolate used-heap is snapshot-derived and therefore shareable.

**`src/workerd/tests/benchmarks/aggregate.py`** (≈250 LOC)

Aggregates multiple runs (different seeds) into a Markdown summary
with median-across-runs and IQR-as-%-of-median as noise estimate.

### What's NOT in this PR

- No runtime code changes.
- Not run as part of `bazel test //...`; lives under a
  `//tests/benchmarks:all` target so CI doesn't accidentally include
  it.
- No default CI integration — future follow-up can add a nightly job
  that archives results and regression-checks.

### Why this is useful

Before this PR, outside contributors writing issues like #6595 had
to build their own benchmark harness from scratch. The original
#6595 report contained overclaimed numbers precisely because there
was no repo-local tool to produce honest ones. This PR addresses
that.

Typical first-run output on a 16-core Intel Ultra 9, V8 12.8 (per
three runs, median, variance within ±3-6%):

```
P1 round-trip:
  XS  30 B serialized   -> 291 ns round-trip
  S  181 B              -> 853 ns
  M  4.2 KB             -> 1.59 µs
  L  56 KB              -> 55.85 µs
  XL 455 KB             -> 70.55 µs

P2 resume cost (M workload):
  A (fresh iso+ctx)   714 µs total   (iso-safe)
  B (pooled iso)      122 µs total   (iso-safe)
  C (pooled ctx)      1.45 µs total  (NOT iso-safe — leak verified)

P3 per-isolate RSS (hello, N=1..1000 regression, R²=1.000):
  591.8 KB per isolate
  stage_0 / stage_3 COW upper bound: 10.8%
```

### How to run

```
bazel build //src/workerd/tests/benchmarks:all
bazel run //src/workerd/tests/benchmarks:p1_serialization_cost > p1.json
bazel run //src/workerd/tests/benchmarks:p2_resume_cost > p2.json
bazel run //src/workerd/tests/benchmarks:memory_benchmark -- memory.json
bazel run //src/workerd/tests/benchmarks:heap_classifier > heap.txt
python3 src/workerd/tests/benchmarks/aggregate.py results/
```

Produces `results/SUMMARY.md` suitable for pasting into issue reports.

### Follow-ups (if accepted)

1. Add to a nightly CI job with result archiving. Would let workerd
   team notice per-isolate cost regressions early.
2. Plumb the benchmarks to run against multiple workerd-pinned V8
   revisions simultaneously (cross-version regression plots).
3. Add workload profiles representative of real Workers bindings
   (KV reads, R2 object fetch, Durable Object state) instead of just
   size-class synthetic.

### Test plan

- [x] `bazel build //src/workerd/tests/benchmarks:all` succeeds on
      Linux x86_64 (tested on Ubuntu 24.04)
- [x] Each benchmark binary runs to completion and emits valid JSON
      / parseable output
- [x] `aggregate.py` produces a SUMMARY.md from the output of 3 runs
- [x] `C` config's isolation test correctly reports `LEAKED` status
      (otherwise its numbers would be misleading)
- [ ] Reviewer: please confirm `bazel build` works on your platform
      (macOS especially, smaps is Linux-only and falls back to
      `getrusage`)

### Reviewer notes

All five `.cc` files are self-contained — each has its own `main()`
and can be copy-pasted elsewhere. This is intentional; issue
reporters who want to reproduce a measurement shouldn't have to
understand the full workerd build.

`BUILD.bazel` uses `cc_binary` with `@v8//:v8` as the dep. If V8 is
not the right bazel target name in this tree please advise; I copied
from existing bench targets in `src/workerd/benchmark/`.

---

## Diff summary

```
 src/workerd/tests/benchmarks/BUILD.bazel                    |  45 ++
 src/workerd/tests/benchmarks/README.md                      |  55 ++
 src/workerd/tests/benchmarks/aggregate.py                   | 250 ++++++
 src/workerd/tests/benchmarks/common.h                        | 160 ++++
 src/workerd/tests/benchmarks/common.cc                       | 120 +++
 src/workerd/tests/benchmarks/p1_serialization_cost.cc        | 190 +++++
 src/workerd/tests/benchmarks/p2_resume_cost.cc               | 260 ++++++
 src/workerd/tests/benchmarks/memory_benchmark.cc             | 180 ++++
 src/workerd/tests/benchmarks/heap_classifier.cc              | 100 ++
 src/workerd/tests/benchmarks/testdata/expected_summary.md    |  80 ++
 10 files changed, 1440 insertions(+)
```

---

## Related

- V8 Gerrit CL (companion): `PR_DRAFTS/v8-cl-description.md`
- RFC for process-level sharing: `src/v8-shared-heap/RFC.md`
- Original issue: cloudflare/workerd#6595
