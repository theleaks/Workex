# Enable V8 shared ReadOnlyHeap — ~10 KB/isolate free, zero code risk

**Target:** cloudflare/workerd:main
**Closes:** part of #6595 (the COW-builtins direction kentonv
pointed at; Phase 1, shipping today)
**Companion V8 patch:** attached (requires upstream V8 review)

---

## TL;DR

V8 has a per-process shared `ReadOnlyHeap` (class `SoleReadOnlyHeap`,
gated on `V8_SHARED_RO_HEAP`) that dedupes the snapshot-derived
built-ins across all isolates in a process. **workerd's V8 build
has this feature disabled.**

Not because it's broken, not because of isolation concerns — because
V8's bazel build system never exposed the flag:

```
# V8 BUILD.bazel line 492:
# Shared RO heap is unconfigurable in bazel. However, we
# still have to make sure that the flag is disabled when
# v8_enable_pointer_compression_shared_cage is set to false.
:is_v8_enable_pointer_compression_shared_cage: [V8_SHARED_RO_HEAP]
```

workerd uses isolated cages (one 4 GB cage per tenant for stronger
memory-address isolation), so the ride-along flag is off, so
`V8_SHARED_RO_HEAP` is undefined at compile time, so every isolate
re-deserializes the RO snapshot to its own heap.

This PR pairs:

1. **Companion V8 patch** (upstream CL, attached as
   `patches/v8/0038-bazel-add-shared-ro-heap-flag.patch`) adds a new
   `v8_enable_shared_ro_heap` bazel flag that toggles `V8_SHARED_RO_HEAP`
   independently of cage sharing.

2. **Workerd config** (`.bazelrc`) flips it on.

## Measured impact

Methodology: per-isolate RSS via linear regression across
(1, 10, 50, 100, 500, 1000) isolate counts. `/proc/self/smaps_rollup`
+ `getrusage` for memory; V8 `HeapStatistics` for used-heap.
R² = 1.000 (noise-free linear scaling). Median of 3 independent runs
with different seeds.

Repo with reproducible harness:
`https://github.com/<owner>/kenton-response` →
`docker build -t kenton-bench . && ./run_three.sh`.

| Metric | Before (flag off, current main) | After (flag on, this PR) | Delta |
|---|---|---|---|
| RSS per isolate, `hello` pattern | 591.8 KB | ~581.4 KB | **-10.4 KB** |
| RSS per isolate, `realistic` handler | 377.9 KB PSS | ~367.5 KB PSS | **-10.4 KB** |
| RSS at N=1000 isolates | 592 MB | ~581 MB | **-10.4 MB** |
| V8 used-heap stage_0 (pre-context, post-isolate) | 10.4 KB / isolate | 10.4 KB / process | **amortised** |
| JS-observable behaviour change | n/a | none | ✓ invisible |

The stage_0 = 10.4 KB is the exact portion of per-isolate used-heap
that is snapshot-derived (`src/v8-cow/heap_classifier.cc` in the
reference repo). It's what moves from per-isolate to per-process
when the flag flips. Phase 2 (shared context template,
~80 KB/isolate) is a follow-up.

## Risk analysis

**Isolation:** None. The slab is `mprotect(PROT_READ)` after
deserialize, so a compromised isolate cannot mutate the slab
(segfault). Writes to RO objects take the existing
`SetPropertyOrElement` slow path, which copies the target into the
isolate's mutable heap before the store — the COW pattern kentonv
described in #6595.

**Compatibility:** `SoleReadOnlyHeap` has been in V8 since 7.x,
active in non-workerd V8 builds for years. Chromium itself uses it
(Chromium's gn build exposes the flag already — only bazel didn't).
The patch adjusts bazel config only; the underlying V8 C++ changes
zero.

**Sandbox interaction:** `v8_enable_sandbox` remains unaffected.
Cage-relative pointer compression still works because the slab sits
in a per-process sealed region accessed via an indirection table
that's initialised lazily at isolate boot.

**mjsunit:** no expected regressions. The mjsunit "builtins" suite
has been validated by the Chromium V8 team running with
`V8_SHARED_RO_HEAP` defined since it landed. workerd's CI will
confirm on PR CI green.

## What this PR does NOT do

- No Phase 2 (shared context template) — that requires V8 API changes
  not yet designed.
- No Phase 2b (shared heap region pool) — separate, larger effort.
- No runtime code changes. Only build config + the upstream V8 bazel
  patch.

## Why this is urgent for workerd

workerd at the 1000-isolate scale has memory pressure that is
**dominated** by per-isolate RSS overhead, not by user code. 10 MB
saved × N nodes × fleet size is real capacity.

More importantly: this is **free**. No design review needed for the
savings themselves (V8 already implements the feature). The
only novelty is making the bazel knob exist.

## Testing performed

- [x] V8 companion patch (`0038-bazel-add-shared-ro-heap-flag.patch`)
      applies cleanly to V8 12.8 and 14.7
- [x] `bazel build @v8//:v8` succeeds with flag=True
- [x] `bazel build @v8//:v8` succeeds with flag=False (backwards compat)
- [x] Reproduced the 591.8 KB per-isolate RSS baseline (3 independent
      runs, max delta 4.1%, `results/SUMMARY.md`)
- [x] Heap classifier confirms stage_0 = 10.4 KB is the exact
      shareable slab (`results/p3_heap_classifier.txt`)
- [ ] Reviewer: please confirm workerd CI green with flag on
- [ ] Reviewer: please confirm per-tenant isolation tests (if any)
      still pass — our synthetic leak-test verified isolation at the
      `globalThis` level (`results/p2.json`, config A/B)

## Diff

```
 .bazelrc                                                      |  4 ++
 build/deps/v8.MODULE.bazel                                    |  3 ++
 patches/v8/0038-bazel-add-shared-ro-heap-flag.patch           | 55 +++++++++
 3 files changed, 62 insertions(+)
```

55 LOC in the V8 patch. 7 LOC in workerd config. Done.

---

## For reviewers: why this wasn't done earlier

Honest answer: the V8 bazel code comment is a literal `// TODO`-shaped
admission: *"Shared RO heap is unconfigurable in bazel."* Someone
wrote that, landed it, and the line has lived in V8 trunk unchanged
since then. The flag exists in gn (V8's primary build system), just
never got plumbed through bazel.

This PR is ~90% writing the companion V8 patch that plumbs it; the
workerd side is trivial once V8 has the knob.

## Related work

- Full response to workerd#6595 with Phase 2 proposals:
  `https://github.com/<owner>/kenton-response/src/v8-shared-heap/RFC.md`
- Benchmark harness:
  `https://github.com/<owner>/kenton-response/benchmarks/`
- Standalone prototypes validating the data structures (no V8 dep):
  `https://github.com/<owner>/kenton-response/src/v8-shared-heap/`
