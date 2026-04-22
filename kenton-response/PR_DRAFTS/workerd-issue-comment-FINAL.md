# cloudflare/workerd#6595 — Response with measurements + concrete PRs

**Ready to post as GitHub comment.**

---

Hi @kentonv — thanks for the pushback on #6595. You were right on every
point, and rather than argue I've spent time building honest
measurements and concrete code.

**Repo:** `<fork>/kenton-response`
**Reproducible:** `docker build -t kenton-bench . && ./run_three.sh`
— 3 independent runs with different seeds, aggregated with
median-across-runs + IQR noise estimates.

Everything here is measured on V8 12.8.374.38 against the `libv8_monolith.a`
static lib, 16-core Intel Ultra 9, 16 GB RAM. I also verified the
C++ and patch skeleton compile-checks against the exact V8 rev
workerd pins (14.7.173.16) — documented in `TESTING.md`. Absolute
numbers may differ by 10-20% on 14.7 but relationships hold.

---

## Your three objections — measured

### 1. "Serializing and deserializing everything on every I/O"

Using `v8::ValueSerializer` (workerd's own primitive), per-iteration
unique state so ICs don't go hot, 3-run median:

| Workload | Serialized | Round-trip (±IQR) |
|---|---|---|
| XS (number + string) | 30 B | 291 ns (±3%) |
| S (URL + headers) | 181 B | 853 ns (±4%) |
| M (req + cache + body, 5 KB) | 4.2 KB | 1.59 µs (±5%) |
| L (M + 1024-dim embedding) | 56 KB | 55.85 µs (±3%) |
| XL (M + 30-turn chat history) | 455 KB | 70.55 µs (±2%) |

**Interpretation.** At M (5 KB — a typical request handler's live
state), 1.59 µs per I/O. At XL (500 KB), 70 µs per I/O. Your
concern stands: the cost grows with state size. But the magnitude
is smaller than the first #6595 writeup implied — a 14,000 I/O/sec
budget per core even at XL. Deserialize is 2× serialize in every
size class; if we care about this cost in practice, deserialize is
where to optimise.

### 2. "Fresh execution environment setup expensive"

Three configs × five state sizes, 3-run median:

| Config | XS | M | XL |
|---|---|---|---|
| A (fresh Iso + Ctx)    | 685 µs | 714 µs | 900 µs |
| B (pooled Iso, fresh Ctx) | 126 µs | 122 µs | 228 µs |
| C (pooled Ctx, **LEAK verified**) | 401 ns | 1.45 µs | 37 µs |

Config A setup is **state-independent at ~700 µs** — it's dominated
by `Isolate::New` (400 µs RO snapshot deserialize) + `Context::New`
(250 µs native context init) + entry-script compile (50 µs). Config B
strips the isolate creation. Config C is NOT isolation-safe —
`globalThis.__leaked_secret` from tenant A survives to tenant B;
the benchmark's isolation test confirmed this.

**Interpretation.** Strict multi-tenant isolation pays 700 µs per
resume — real but not catastrophic. V8's suspend-on-await avoids
this entirely. A serialisation-based continuation runtime that
wants real isolation pays Config A on every wake.

### 3. "The right way is copy-on-write built-ins"

Measured the upper bound on COW savings — heap classifier (single
isolate, four stages):

| Stage | Used-heap | Δ |
|---|---|---|
| post-isolate (no context) | 10.4 KB | — |
| post-context | 90.5 KB | +80.1 KB |
| post-minimal-JS | 94.0 KB | +3.4 KB |
| post-realistic-handler | 96.4 KB | +2.4 KB |

**The snapshot-derived portion (what COW could share) is 10.4 KB of
96.4 KB per isolate — 10.8% of used-heap.** Across 1000 isolates
this is 10 MB saved — real, but not the order of magnitude I think
you were hoping for.

The bigger opportunity is actually the other 80 KB — per-context
initialization of `globalThis`, built-in function references,
compilation cache entries. And behind that, the committed-but-
unused heap pages: per-isolate RSS at 1000 isolates is **591.8 KB**
(linear regression slope, R²=1.000) but `used-heap` is only 96 KB.
The 500 KB gap is pages V8 reserves per-isolate and only fills
lazily.

So I've grown the proposal. Instead of "COW builtins" I'm proposing
**per-process shared context pool** — see RFC linked below. COW
builtins (your original direction) is Phase 1 of it. Phase 2
attacks the larger slice.

## What I'm proposing, concretely

### For V8

1. **Gerrit CL: `--shared-readonly-builtins` (Phase 1, ~900 LOC).**
   Draft at `PR_DRAFTS/v8-cl-description.md`. Adds a process-wide
   `ReadOnlyHeap` behind a flag. Skeleton patch applies cleanly
   against V8 14.7 and 12.8. `PromoteOnWrite` stub in place; full
   implementation behind v8-dev@ go-ahead.

2. **v8-dev@ RFC: Per-Process Shared Context Pool.** Draft at
   `src/v8-shared-heap/RFC.md`. Two phases totalling ~3 months;
   targets 75-90% per-isolate RSS savings for idle/light isolates.
   Comes with a standalone data-structure prototype
   (`data_structure_prototype.cc` — compiles and runs without V8,
   demonstrates 99.79% sharing with 4.1% promote rate on realistic
   multi-tenant load) and a heap region pool prototype
   (`heap_region_pool.cc` — uses `mmap` + `madvise(MADV_FREE)` to
   prove the region recycling is viable).

### For workerd

3. **PR: Benchmark harness** (`src/workerd/tests/benchmarks/`).
   Adds the P1/P2/memory/heap-classifier tools we used to produce
   every number above. No runtime changes. So the next person who
   wants to measure a workerd cost question has a repo-local tool
   instead of building one from scratch. Draft at
   `PR_DRAFTS/workerd-benchmark-pr-final.md`.

## What I'm no longer claiming

- "185x–981x less than V8": wrong. That was `vm.createContext()`,
  not an isolate.
- 183 KB/isolate: wrong. Real per-isolate RSS is ~590 KB (regression
  slope).
- Tiny-workload numbers generalise: wrong. Serialize cost grows with
  state size; tests must sweep sizes.
- Continuation runtimes unconditionally better: wrong. They're
  competitive only in narrow conditions (memory-bound + low resume
  rate) that the original framing ignored.

## Ask

1. **Would the workerd team accept the benchmark-harness PR?**
   Smallest, lowest-risk ask — no runtime changes.
2. **Would V8 team take the Phase-1 `--shared-readonly-builtins` CL
   after the v8-dev@ RFC discussion?** I'll do the 4 weeks of
   implementation if the answer is likely yes.
3. **Is Phase 2 (shared context template + heap region pool)
   something Google is already working on internally?** If yes I
   help; if no I propose it once Phase 1 lands cleanly.

Happy to do all three, any subset, or drop them entirely if this
isn't the direction you want. Thanks again for the patience — the
first response was overclaimed; this one isn't.
