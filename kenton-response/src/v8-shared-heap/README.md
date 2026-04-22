# V8 Shared Context Pool — Phase 1 + Phase 2 proposal

Deeper engineering follow-up to `src/v8-cow/`. Where the cow-builtins
RFC targets ~10-15% per-isolate memory savings, this one targets
75-90% by extending sharing beyond the RO snapshot into:

- Shared context template (invariant native-context slots)
- Process-wide heap region pool (recycle committed-but-unused pages)

## Files

| File | What it is | Runs? |
|---|---|---|
| `RFC.md` | v8-dev@ proposal — two-phase plan | — |
| `data_structure_prototype.cc` | Shared RO slab + SharedContextTemplate + per-isolate COW shadow. Standalone C++, no V8. | ✓ |
| `heap_region_pool.cc` | Process-level region pool with MADV_FREE recycling. Standalone C++. | ✓ |

## Quick results

### `data_structure_prototype`

1000 tenant isolates, each ~300 RO-lookups, 5% mutate 1-3 built-ins:

```
  naive total:        4.3 MB
  COW total:          9.5 KB
  savings:            99.79%
  isolates that wrote: 41 / 1000 (4.1%)
```

The 99.79% is synthetic (30 built-ins × 64 B object), not directly
comparable to V8. What it proves: **the sharing + promote algorithm
is not pathological** — rare writes don't blow up the per-isolate
memory overhead, and common reads are O(1).

### `heap_region_pool`

1000 isolates × 2 regions each, Phase 1 acquire + Phase 2 release
half + Phase 3 reacquire:

```
  reserved regions:   2000 (500 MB virtual)
  new allocs:         2000
  warm hits:          1000      <- zero faults on reacquire
  cold hits:          0
  naive:              3000 × 256 KB = 750 MB peak
  with pool:          2000 × 256 KB = 500 MB peak
  savings:            33.3% virtual (Linux: + physical via MADV_FREE)
```

On Linux, `MADV_FREE` lets the OS reclaim physical pages of released
regions while keeping the virtual mapping. Peak RSS (not just virtual)
drops meaningfully — the Windows prototype only shows virtual because
`madvise` isn't a thing there.

## Build

```bash
# Either:
g++ -O2 -std=c++20 -pthread data_structure_prototype.cc -o sdp
g++ -O2 -std=c++20 -pthread heap_region_pool.cc -o hrp
# Or MSVC:
cl /nologo /std:c++20 /EHsc /O2 data_structure_prototype.cc
cl /nologo /std:c++20 /EHsc /O2 heap_region_pool.cc
```

Both run in <1 second.

## Relationship to `src/v8-cow/`

`src/v8-cow/` contains the skeleton V8 patch for Phase 1 only
(shared RO slab). It's the concrete, applies-cleanly contribution
for a Gerrit CL.

`src/v8-shared-heap/` contains the fuller RFC covering Phase 2
(context template + region pool), plus prototypes that validate
the Phase 2 data structures without needing a V8 build.

The actual V8 implementation order would be:
1. Ship Phase 1 (`--shared-readonly-builtins`) — the CL in
   `PR_DRAFTS/v8-cl-description.md`
2. After it's stable, v8-dev@ thread on Phase 2 design
3. Phase 2 CL series (probably 2-3 CLs, each ~1000 LOC)

## What this RFC is NOT

- Not a claim that we've solved multi-tenant V8 memory. It's the
  concrete shape of the work that would solve a large fraction of
  it.
- Not a criticism of workerd's current design — workerd already
  extracts maximum value from today's V8. This proposal is about
  giving workerd more to work with, upstream.
- Not an alternative to the existing V8 "shared heap" project
  (shared strings, shared code). This extends in the same direction.
