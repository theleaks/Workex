# P3 — Copy-on-Write Built-ins for V8

Response to Kenton's "the right way is copy-on-write built-ins" remark
in cloudflare/workerd#6595.

We are *not* claiming to ship a working V8 patch here. We're shipping
**the honest shape of the contribution** so the V8 team can decide
whether to accept a bigger PR.

## What's in this folder

| File | What it is | Runs? |
|---|---|---|
| `RFC.md` | Design proposal, written for v8-dev@ | — |
| `cow_builtins_prototype.cc` | Standalone C++ prototype of the shared-slab + COW-promote data structure | yes (no V8 needed) |
| `memory_benchmark.cc` | Isolate memory footprint benchmark against stock V8 | yes (V8 required) |
| `heap_classifier.cc` | Measures the upper bound on COW savings by classifying heap contents | yes (V8 required) |
| `patch_skeleton.diff` | Additive V8 patch: adds `src/heap/cow-shared-builtins.{h,cc}` | applies cleanly (no-op until hooks are wired) |
| `hooks_reference.md` | Edits needed in existing V8 files (done by hand because line numbers drift) | — |
| `build.sh` | Builds the three C++ tools | |
| `apply_patch_and_build.sh` | Applies the skeleton patch to a V8 checkout and rebuilds | |

## Quickstart — no V8 needed

To see the sharing ratio the data structure achieves on a synthetic
workload:

```bash
./build.sh
./build/cow_builtins_prototype
```

You'll see ~99% memory savings at 1000 isolates — which is
correct-but-synthetic. The real V8 question is whether the shape of
V8's heap and GC permit this in practice; that's what the RFC lays out.

## With a V8 checkout

```bash
# one-time: fetch + build V8 (takes 30-90 min, 20 GB)
cd ../../benchmarks && ./fetch_and_build_v8.sh && cd -

# baseline measurements (no patch)
./build.sh
./build/memory_benchmark memory.baseline.json
./build/heap_classifier  heap.snapshot.json

# apply the skeleton patch and rebuild V8 to verify it applies cleanly
./apply_patch_and_build.sh
```

## Why not just write the full patch

Estimated work (RFC.md, "implementation plan" section): 4-8 weeks of
V8-specialist time, because:

- Slab-targeted deserialisation needs a reader variant of
  `src/snapshot/deserializer.{h,cc}` and the ability to reuse pointer
  layout between shared and per-isolate copies.
- The promotion path needs to play nicely with pointer compression,
  the sandbox, and `DCHECK_OBJECT_ALIGNMENT`-style invariants.
- The mjsunit test suite has to stay green under the flag.

Shipping this without consulting V8 reviewers first would waste that
time if it turns out (for example) that the sandbox cage model makes
shared slabs impractical. The RFC exists to get that question answered
first.

## Open questions we want V8's answer to

Listed in `RFC.md` "Open questions for V8 reviewers". Short version:

1. Pointer-identity assumptions in the codebase — how many hot paths
   compare by pointer rather than value?
2. Snapshot versioning for the shared slab — how do we guarantee every
   isolate sees a compatible slab?
3. Sandbox + cage interaction — can the slab live inside the cage, and
   how?
4. Is Google already doing this internally?

If the answer to 4 is yes, great — we stand down. Otherwise, these are
the inputs we need to commit to the 4-8 weeks.
