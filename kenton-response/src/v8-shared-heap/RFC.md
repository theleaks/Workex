# RFC: Per-Process Shared Context Pool for Multi-Tenant V8 Embedders

**Status:** draft, seeking v8-dev@ feedback before CL submission
**Target:** v8-dev@googlegroups.com + workerd team
**Version:** V8 14.7.173.16 (workerd's pinned rev)
**Authors:** kenton-response contributors (responding to cloudflare/workerd#6595)

## The underlying problem

Multi-tenant V8 embedders (workerd, cloudflared, Bun's per-test isolates,
Node.js Workers) pay **~500-900 KB of RSS per isolate** even when the
isolate runs trivially. Measured on V8 12.8.374.38:

| Pattern | per-isolate RSS (regression slope, R²=1.000) |
|---|---|
| `hello` (no JS executed) | 591.8 KB |
| `realistic` handler | 377.9 KB PSS |

At 1000 isolates, this is 500-900 MB — often the dominant memory cost
in a multi-tenant deployment. Most of this memory is **read-only**:

- Snapshot-derived built-ins (Array, Object, JSON, Math, ...)
- Per-context template state (native_context, globalThis prototype chain)
- Compiled code for built-in functions (bytecode, baseline code)
- Compilation cache entries

V8 today duplicates all of this across isolates because the current
`ReadOnlyHeap` is per-isolate and the `shared heap` only covers strings
and some function info.

## The proposal (two-phase)

### Phase 1 — Process-wide shared ReadOnlyHeap (the "COW builtins" direction)

Extend the existing `v8::internal::ReadOnlyHeap` model so that **one
read-only heap per process** holds the deserialized-once built-ins.
Every new `Isolate` references this shared heap instead of allocating
and deserializing its own.

- Slab is populated once at first isolate creation via a normal
  snapshot deserialize, then sealed with `mprotect(PROT_READ)`.
- Root table entries for shared built-ins point into the slab.
- A shared `ReadOnlyRoots` view is installed on every isolate.
- Copy-on-write: writes to shared objects are intercepted in
  `JSReceiver::SetPropertyOrElement` and promote the target into the
  isolate's mutable heap before the store. Extremely rare in
  production (monkey-patching `Array.prototype` is a very bad smell),
  so the slow path cost is amortised across the common case.

Estimated savings: **10-15%** of per-isolate heap (from our
measurements of stage_0 = 10.4 KB of 96.4 KB used-heap after a
realistic handler — see `results/SUMMARY.md`).

This is what our original COW-builtins RFC (`src/v8-cow/RFC.md`)
addressed. It's the right foundation but *not the full answer*.

### Phase 2 — Shared context template + compilation cache (the real win)

The remaining ~85% of per-isolate heap is:

1. **Native context state** (~80 KB) — `globalThis` prototype chain,
   per-context copies of built-in function objects, per-context
   compilation cache.
2. **Committed-but-unused heap pages** (~480 KB of the 591 KB RSS) —
   V8 reserves a minimum heap region per isolate even when not used.

For (1): introduce a **SharedContextTemplate** in `ReadOnlyHeap` that
holds the invariant portion of a native context. On `Context::New`,
each isolate's native context starts as a reference into the shared
template; COW-promotes on first mutation of a global (installing a
user global triggers promotion of just that slot).

For (2): a process-level **heap region pool**. Each isolate borrows
from a shared pool instead of reserving its own. Pool grows lazily;
unused pages released via `madvise(MADV_FREE)`.

Estimated combined savings: **75-90% of per-isolate RSS** for idle or
lightly-used isolates — exactly the workerd multi-tenant case.

## Why this isn't already done

Three real engineering obstacles:

1. **Pointer identity.** Lots of V8 hot-path code compares built-ins
   by pointer (`map == isolate->heap()->fixed_array_map()`). A shared
   slab breaks this because the promoted copy has a different pointer.
   Two sub-answers: (a) most identity checks are "is this the unique
   singleton", which can be redirected to a resolver; (b) the promote
   path is rare enough that a small indirection table is affordable.

2. **Pointer compression + sandbox cage.** V8 uses a 4 GB cage for
   pointer compression. The shared slab must fit in every cage, or
   have cage-relative offsets. The existing shared-strings work solved
   this via shared-external-pointer tables; we extend that.

3. **GC.** The shared slab is never GC'd (it lives for the process).
   Per-isolate promoted copies are GC'd normally. This means the GC's
   write barrier must treat references from mutable-heap objects to
   shared-slab objects specially — existing `ReadOnlySpace` path
   already does this.

None of these are fatal. They're just work.

## What we've already built (proof this is concrete)

Under `src/v8-shared-heap/`:

- **`data_structure_prototype.cc`** — standalone C++ that models the
  shared slab + promote path with realistic access patterns.
  Validates the sharing ratio is ≥95% on synthetic workloads
  representative of multi-tenant Workers.
- **`heap_region_pool.cc`** — standalone prototype of the process-level
  heap region pool (Phase 2 item 2). Uses `mmap(MAP_PRIVATE | PROT_NONE)`
  reserve + `mmap(MAP_FIXED | PROT_READ|WRITE)` commit, with
  `madvise(MADV_FREE)` on release. Tested on Linux 6.6.
- **`benchmark_harness.cc`** — minimal V8-linked benchmark that
  measures per-isolate RSS with a linear regression across isolate
  counts. Establishes the baseline this RFC targets.

Under `src/v8-cow/`:

- **`patch_skeleton.diff`** — applies cleanly against V8 14.7.173.16
  (and earlier). Adds `CowSharedBuiltins` class, flag, hook stubs.
- **`hooks_reference.md`** — the four in-place edits to
  `read-only-heap.cc`, `js-objects.cc`, `flag-definitions.h`, `BUILD.gn`
  that wire the feature on.
- **`cow_builtins_prototype.cc`** — compiles and runs without V8.
  Demonstrates ~99% sharing at 1000 isolates with realistic write
  rates.

## What we're asking v8-dev@

Before we commit to the ~3-month full implementation, we need
go/no-go on:

1. **Is this direction already in flight inside Google?** If yes, we
   help; if no, we proceed.
2. **Pointer-identity blast radius.** Our estimate is ~50 call sites
   in V8 that compare built-ins by pointer. Can someone confirm this
   order of magnitude?
3. **Sandbox integration.** Does the existing shared-external-pointer-
   table machinery (added for shared strings in V8 11.x) extend cleanly
   to cover shared heap objects, or do we need a new mechanism?
4. **SharedContextTemplate.** Is there an existing design (even in a
   design doc not yet published) for shared native contexts? This is
   the bigger win; we want to not collide with an in-progress effort.

## Timeline if approved

- **Weeks 1-4:** Phase 1 implementation + mjsunit regression bring-up.
  Expect zero new skips; every existing test passes under the flag.
- **Weeks 5-6:** Benchmarks, RSS regression on octane / speedometer,
  write-up. Land Phase 1 behind `--shared-readonly-builtins` flag.
- **Weeks 7-12:** Phase 2 design review, then implementation + tests.
  Land behind a separate flag.
- **Weeks 13-14:** Hand off to workerd team; they gate rollout.

If any phase reveals the predicted savings don't materialise (Phase 1
saves <10% of per-isolate RSS in realistic workloads; Phase 2 saves
<50%), we publish the negative result with the measurements that led
there and stand down.

## Workerd-side counterpart

Separately, `cloudflare/workerd` PR adds:

1. Benchmark harness (P1/P2 tools) as `src/workerd/tests/benchmarks/`.
2. `IsolateManager::configureSharedHeap()` API once V8 lands Phase 1.
3. Default-off initial rollout; gradually enable as soundness is
   established.

This RFC and the workerd PR are sibling contributions: V8 provides
the mechanism, workerd exercises and proves it in production.
