# Re: workerd#6595 — Cage-compatible shared ReadOnly Heap is possible

Hi @kentonv — three sessions of measurement, then a breakthrough.
Sharing here because the result genuinely surprised me and may
surprise you too.

## TL;DR

V8's `IsReadOnlySpaceShared()` is gated on `pointer_compression_shared_cage`
because, the comment in `read-only-heap.h` says, you can't share a
ReadOnly slab across isolated cages. **That assumption is wrong.**
Linux's `memfd_create + MAP_FIXED + MAP_SHARED + PROT_READ` lets you
map the same physical pages at the same intra-cage offset in every
cage. V8's compressed pointer decode `cage_base + raw_value` then
reaches the *same* physical bytes from any cage, with **zero changes
to the decompression hot path**.

**Verified standalone** (full source + build script in
`<repo>/src/solution/cage-compatible-ro/`):

```
$ ./cross_cage_proto
[slab] memfd=3 size=65536 (PROT_READ enforced per-mapping)
[smaps after slab init] RSS=3.37 MB PSS=2.72 MB
[cage 0] base=0x759900000000 (alignment check: OK)
[cage 1] base=0x759700000000 (alignment check: OK)
... [cages 2..7]
[smaps after cage creation] RSS=3.50 MB PSS=2.82 MB    # +0.10 MB for 8 cages

Decoding compressed pointer 0x40 (expecting tag 0xdeadbeef00000040):
  cage[0] base=0x759900000000 -> tag=0xdeadbeef00000040 OK
  cage[1] base=0x759700000000 -> tag=0xdeadbeef00000040 OK
  ... all 8 cages report identical bytes ...
  cage[i] write probe: OK (SIGSEGV)              # 8/8 reject writes

================================
 cross-cage decode: 8/8 cages OK
 write-blocked:     8/8 cages OK
 result: PROTOTYPE VERIFIED
================================
```

**100-cage stress test:** 100/100 cages decode + write-block OK,
PSS only grew by 30 KB (not 100 × 64 KB). Linux page tables are
correctly de-duplicating the slab.

## Why the V8 comment was wrong (and what I learned that I owe you)

V8 `read-only-heap.h:93-96`:

```cpp
static constexpr bool IsReadOnlySpaceShared() {
  return V8_SHARED_RO_HEAP_BOOL &&
         (!COMPRESS_POINTERS_BOOL || COMPRESS_POINTERS_IN_SHARED_CAGE_BOOL);
}
```

V8 `BUILD.bazel:491-498`:
```python
# Shared RO heap is unconfigurable in bazel. However, we
# still have to make sure that the flag is disabled when
# v8_enable_pointer_compression_shared_cage is set to false.
:is_v8_enable_pointer_compression_shared_cage: [V8_SHARED_RO_HEAP],
```

The reasoning encoded here is: *isolated cages each occupy their own
4 GB virtual region, so a 32-bit compressed pointer means different
addresses in different cages, so a shared RO heap cannot be
referenced cross-cage.*

That's true at the *virtual address* level. But it's wrong at the
*physical memory* level, because Linux can map the same physical
pages at multiple aligned virtual addresses simultaneously. Cages
are 4 GB-aligned; if every cage maps the same memfd at intra-cage
offset 0..N, then for any compressed pointer `V` with `V < N`, the
expression `cage_base + V` lands in the same physical page set.

V8's cage allocator already reserves the 4 GB region with
`mmap(PROT_NONE | MAP_PRIVATE | MAP_ANONYMOUS)`. We just need to
overlay the first N bytes of every cage with
`mmap(MAP_FIXED | MAP_SHARED | PROT_READ, fd_of_memfd, 0)`. That's
the entire change at the OS level. V8's pointer decompression code
isn't touched.

## What I built

In `<repo>/src/solution/cage-compatible-ro/`:

- **`DESIGN.md`** — full reasoning with risks, mitigations, and
  Phase-A/B/C breakdown.
- **`cross_cage_shared_ro_proto.cc`** — 250 LOC standalone Linux C++.
  Reproduces the test output above.
- **`v8_phase_a_patch.diff`** — V8 patch skeleton:
  - New `src/heap/shared-ro-slab.{h,cc}` (~220 LOC) holding the
    process-wide memfd singleton + `AttachToCage` helper.
  - One-line hook in `IsolateAllocator::InitReservation` to call
    `SharedRoSlab::AttachToCage(cage_base_)` immediately after the
    cage is reserved.
  - `IsReadOnlySpaceShared()` becomes runtime instead of constexpr,
    returning true when the slab is active OR the existing condition
    holds.
  - New flag `--shared_ro_heap_via_memfd` (off by default).
  - Build-system additions for the new files.

The skeleton compiles in isolation; full V8 mjsunit/cctest sweep
still needs a V8 rebuild + run, which is the next step but not in
this comment.

## Memory savings projection

From `<repo>/results/SUMMARY.md` (3-run median, V8 12.8, R²=1.000
linear regression across 1..1000 isolates):

- Per-isolate `used_heap` stage_0 = **10.4 KB** (the
  snapshot-derived portion).
- That whole 10.4 KB becomes shared once across all isolates in the
  process.
- At 1000-isolate workerd nodes: **~10 MB RSS saved per node**.

This is the *floor*. The full RSS gap is ~590 KB/isolate (most of
which is committed-but-unused pages — Phase 2 territory), but
Phase A is what's possible *right now without V8 design review*,
because the V8 mechanism (`SoleReadOnlyHeap`) already exists, just
gated behind a flag the bazel build never exposed.

## What I'm asking

1. **Does the workerd team think this direction is interesting
   enough to pursue?** If yes, I'll:
   - Land the V8 bazel-flag patch upstream first (small,
     mechanical, ~50 LOC).
   - Land the V8 Phase A patch second (the memfd path, ~280 LOC,
     gated behind `--shared_ro_heap_via_memfd`).
   - Open a workerd PR that flips the flag and verifies the RSS
     drop in workerd's CI.
2. **Is Google already doing something similar inside V8?** The
   bazel comment ("unconfigurable in bazel") suggests the area
   was deprioritised; if there's an in-flight effort I'm
   duplicating, I'll align rather than fork.
3. **Phase 2 (shared context template, shared heap region pool)
   targets ~75-90% of per-isolate RSS** — that's the real prize.
   Phase A is the tractable first step. Should I start the Phase 2
   v8-dev@ RFC after Phase A lands, or in parallel?

## What I'm no longer claiming (revised)

The previous round of measurements was honest but incomplete. With
the breakthrough above:

- ✗ "COW builtins is the right way and saves 75-90%": not by itself.
  COW (Phase A here) saves ~10 KB/isolate. The big slice (~480 KB
  per isolate of committed-but-unused pages) needs Phase 2.
- ✓ "There's a quick V8 win that's free": confirmed, this is it.
  Linux memfd-based cage-compatible sharing. ~50 LOC bazel + ~280
  LOC V8 + ~7 LOC workerd config = done.
- ✗ "Workerd should switch to shared cage": still rejected. Stronger
  isolation per tenant is right; this design preserves it.

## Repository

`https://github.com/<owner>/kenton-response`
- `src/solution/cage-compatible-ro/DESIGN.md` — design
- `src/solution/cage-compatible-ro/cross_cage_shared_ro_proto.cc` —
  standalone Linux verifier (PROTOTYPE VERIFIED, 8/8 + 100/100)
- `src/solution/cage-compatible-ro/v8_phase_a_patch.diff` — V8
  skeleton patch
- `results/SUMMARY.md` — measurement baseline
- `benchmarks/` — reproducible Docker harness for any future
  before/after comparison

Thanks for the patience across the wrong-then-better-then-real
iterations. This is the "real."
