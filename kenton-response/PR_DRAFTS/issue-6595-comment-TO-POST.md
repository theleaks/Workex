Hi @kentonv — three rounds of iteration, then an actual result.
Apologies for the wandering path; the original #6595 writeup was
overclaimed, the first correction still wrong, and only the third
cycle produced something I'll stand behind.

## Where I landed

1. **PR #6636** (this repo): adds an in-tree benchmark for
   `v8::ValueSerializer` round-trip cost across five realistic
   agent-state sizes. Follows the existing `wd_cc_benchmark`
   convention. No runtime changes. One target: the next person who
   wants to argue about continuation cost has a repo-local tool.

2. **V8 `shared-ro-slab.{h,cc}` skeleton** that compiles cleanly
   into the V8 12.8 tree via GN/ninja with V8's bundled clang and
   the full compile-flag set
   (`V8_COMPRESS_POINTERS_IN_SHARED_CAGE`, `V8_ENABLE_SANDBOX`,
   `V8_EXTERNAL_CODE_SPACE`, ...). All ten expected symbols in
   `nm` output. 6528-byte .o produced. Object file verified;
   mjsunit/cctest not yet run because the integration hook
   (`IsolateAllocator::InitReservation`) needs review first — see
   below.

3. **Standalone Linux prototype** validating the cross-cage
   memfd+MAP_FIXED trick. 8 cages and 100 cages both report
   100% cross-cage decode + 100% write-block via SIGSEGV, with
   PSS growing by 30-160 KB regardless of cage count. Five
   independent runs, identical behaviour. Source +
   reproducible Docker run in the repo.

## What this changes about the memory story

From the repo's reproducible 3-run median benchmark suite:

| | Per-isolate |
|---|---|
| `hello` RSS (regression slope, R²=1.000) | 591.8 KB |
| `realistic` PSS regression slope | 377.9 KB |
| V8 used-heap, stage_0 (post-isolate, pre-context) | 10.4 KB |
| V8 used-heap, stage_3 (post-realistic-handler) | 96.4 KB |

Phase A (shared RO slab across isolated cages) targets the 10.4 KB
stage_0 — so ~10 MB/process at 1000 isolates. Not revolutionary by
itself, but it's the piece V8 already has except for the bazel knob
and the cage-sharing gate. The standalone prototype demonstrated
that the cage-sharing gate can be worked around with memfd +
MAP_FIXED, removing the design assumption in V8's
`IsReadOnlySpaceShared()` comment (*"Shared RO heap is
unconfigurable in bazel"*).

## What I'm specifically asking

1. **Is the benchmark PR #6636 a shape the workerd team can
   merge?** No runtime risk. Happy to refactor toward bindings-
   shaped test data (KV / R2 / DO state) instead of the generic
   size-class workloads, if that's a better fit.

2. **Is the V8 Phase A direction worth pursuing upstream?** Three
   sub-questions:
   - Is Google already working on cage-compatible shared RO heap
     internally? If so I stand down; if not I'd open a Gerrit CL
     once the mjsunit bring-up is green.
   - The PROT_READ enforcement at the kernel level — is that
     equivalent to V8's current `mprotect(PROT_READ)` for
     security purposes? I believe yes; confirming.
   - Phase 2 (shared context template + heap region pool) targets
     the other 85% of per-isolate RSS. Should I sketch that as a
     follow-up RFC once Phase A is landed, or is it already in
     flight?

3. **Would the workerd team prefer a pull request with a flag-
   gated opt-in, or an issue for discussion first?**

## What I'm no longer claiming

- *"185x-981x less than V8"*: the original vm.createContext
  mismeasurement. Retired.
- *"COW builtins alone saves 90%"*: only 10.8% of stage_3
  used-heap is snapshot-derived. The big slice is elsewhere.
- *"A simple bazel flag flip unlocks it"*: I thought so mid-way
  through; it doesn't, because V8's runtime check still requires
  either shared cages or the new memfd path this design
  introduces. Mentioning so you see the reasoning chain.

Repo (design docs, prototypes, patches, benchmark results):
`https://github.com/theleaks/Workex/tree/dev/kenton-response`
*(public read-only; I'll keep it stable as a reference)*

Thanks for the patience.
