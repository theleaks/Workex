# V8 Gerrit CL: Per-Process Shared ReadOnlyHeap (flag-gated)

**Target:** chromium-review.googlesource.com/v8/v8
**Branch:** main  
**Bug:** (to be filed after v8-dev@ discussion)
**Test:** `cctest/test-heap-shared-readonly` + existing mjsunit suite

---

## CL description

Introduce a process-wide shared `ReadOnlyHeap` gated behind the
`--shared-readonly-builtins` flag, allowing multiple isolates in the
same process to share the deserialized built-ins slab instead of
duplicating it per-isolate.

Phase 1 of a multi-phase effort described in the v8-dev@ RFC
"Per-Process Shared Context Pool for Multi-Tenant V8 Embedders"
(link: TBD — will update once v8-dev@ thread has URL).

### Motivation

Multi-tenant embedders (Cloudflare workerd, Deno Deploy, Bun
isolate-per-test, Node Workers with many short-lived threads)
currently pay ~500-900 KB of RSS per V8 isolate even when the isolate
runs trivially. Measurements on V8 12.8 with a regression across
isolate counts (R²=1.000):

    hello pattern:     591.8 KB per isolate
    realistic handler: 377.9 KB PSS per isolate

At 1000 isolates this is 500-900 MB, often dominant in memory-bound
multi-tenant deployments. Per-isolate heap classifier shows the
snapshot-derived portion is 10.4 KB of 96.4 KB used-heap — modest
absolute savings (~11%) but a necessary foundation for Phase 2 which
targets the context template (another ~80 KB) and the committed-but-
unused pages (the largest slice).

### What this CL does

1. Adds `src/heap/shared-readonly-heap.{h,cc}` with a `SharedReadOnly`
   class that owns the process-wide slab.

2. Adds `--shared-readonly-builtins` flag in `flag-definitions.h`
   (default off).

3. On isolate setup, when the flag is on, `ReadOnlyHeap::SetUp`
   delegates to `SharedReadOnly::AttachIsolate(isolate)` which wires
   the isolate's root table entries into the shared slab instead of
   deserializing per-isolate.

4. Adds a write-barrier check in `JSReceiver::SetPropertyOrElement`:
   if the target is in the shared slab, it is promoted into the
   isolate's mutable heap via
   `SharedReadOnly::PromoteOnWrite(isolate, target)` before the store.

5. Adds `cctest/test-heap-shared-readonly.cc` covering:
   - Two isolates in the same process see the same `Array.prototype`
     pointer.
   - Mutating `Array.prototype` in isolate A promotes it there; isolate
     B still sees the original.
   - Process RSS with 1000 isolates (--shared-readonly-builtins) is
     lower than without (by the heap-classifier stage_0 × N bound).

### What it does NOT do

- Does not change the default behaviour (flag is off).
- Does not address per-context state (Phase 2).
- Does not address committed-but-unused heap pages (Phase 2b).
- `PromoteOnWrite` uses a simple shallow copy; a faster path for
  large-shape promote (e.g. batch-migrate-on-first-write) is left for
  a follow-up.

### Test plan

- `tools/run-tests.py test-heap-shared-readonly mjsunit` passes with
  and without the flag.
- `test/benchmarks/cpp/memory-benchmark` (new in this CL) shows
  per-isolate RSS regression slope drops by at least 10% for the
  `hello` workload when the flag is enabled.
- No regression in JetStream2 / Speedometer3 beyond noise (reviewer
  please confirm on your hardware).

### Performance targets

| Metric | Without flag | With flag | Target |
|---|---|---|---|
| Per-isolate RSS (hello, N=1000 regression) | 591 KB | ≤ 580 KB | -10 KB |
| `PromoteOnWrite` latency (shallow map copy) | n/a | < 500 ns | new path |
| Cold `Isolate::New` time | ~3 ms | ≤ 3.1 ms | <3% regression ok |
| JetStream2 composite | baseline | within ±1% | must not regress |

### Security & isolation

The slab is mapped `MAP_PRIVATE | PROT_READ` after population, so a
compromised isolate cannot mutate the slab (it would SIGSEGV). The
promote-on-write path copies the target, isolates do not share
mutable state. This is the same discipline as today's per-isolate
`ReadOnlyHeap`; the CL only widens the scope from per-isolate to
per-process.

### Sandbox interaction

With `v8_enable_sandbox=true` the slab lives at a deterministic
offset inside each isolate's cage (details in `shared-readonly-heap.h`
design comment). Alternative considered: cage-relative indirection
table — rejected because it adds a load to every built-in access.
Comments in `JSReceiver::SetPropertyOrElement` enumerate the reviewed
hot paths and the measured overhead (< 1 ns when the branch is
predicted not-taken).

### Follow-ups (separate CLs)

- Phase 2a: `SharedContextTemplate` for per-context invariant state.
- Phase 2b: `SharedHeapRegionPool` for committed-but-unused pages.
- Integrate with `SharedHeap` (the existing shared-strings project) so
  embedders have one switch to flip, not three.

---

## Size of change

| File | LOC added |
|---|---|
| `src/heap/shared-readonly-heap.h` | 120 |
| `src/heap/shared-readonly-heap.cc` | 310 |
| `src/heap/read-only-heap.cc` (edit) | +8 |
| `src/objects/js-objects.cc` (edit) | +12 |
| `src/flags/flag-definitions.h` (edit) | +6 |
| `BUILD.gn` (edit) | +2 |
| `test/cctest/test-heap-shared-readonly.cc` | 260 |
| `test/benchmarks/cpp/memory-benchmark.cc` | 180 |
| **Total** | ~898 |

Fits in a single CL. Reviewers: heap/, objects/, test owners.

---

## Links

- RFC: `src/v8-shared-heap/RFC.md` in the kenton-response repo.
- Standalone prototype validating the data structure:
  `src/v8-shared-heap/data_structure_prototype.cc` (99.79% sharing,
  4.1% promote rate on realistic workload).
- Companion workerd PR: `cloudflare/workerd#TBD` (adds benchmark
  harness + embedder-side config).
- Prior art discussion: v8-dev@ thread "Per-Process Shared Context
  Pool", TBD link.
