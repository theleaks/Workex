# Cage-Compatible Shared ReadOnly Heap for V8

**Problem:** V8 today excludes shared ReadOnlyHeap when using isolated
pointer-compression cages (the workerd config). The reason in
`read-only-heap.h`:

```cpp
static constexpr bool IsReadOnlySpaceShared() {
  return V8_SHARED_RO_HEAP_BOOL &&
         (!COMPRESS_POINTERS_BOOL || COMPRESS_POINTERS_IN_SHARED_CAGE_BOOL);
}
```

When each isolate has its own 4 GB cage, a 32-bit compressed pointer
in cage A decodes to a different physical address than the same
32-bit value in cage B. So a shared RO object cannot be reached from
both cages — *unless* the same physical bytes appear at the same
intra-cage offset in every cage.

That **unless** is the whole design.

## The trick

V8 cages are 4 GB aligned (`kPtrComprCageBaseAlignment`). Compressed
pointer decompression is `cage_base + raw_value`. If we arrange for
the first N bytes of every cage to be backed by the *same physical
memory*, then a compressed pointer with `raw_value < N` will decode
to the same physical bytes regardless of which cage's base is added.

Linux can do this directly: `mmap(MAP_FIXED | MAP_SHARED, fd)` against
a `memfd_create` file. Every cage maps the same fd at offset 0..N
into its own virtual region [cage_base, cage_base+N). The kernel's
page table has multiple virtual regions pointing at one set of
physical pages.

## Step-by-step

### Phase A — Process-wide RO slab (one fd, sealed)

```cpp
// At process startup, before any isolate exists:
int fd = memfd_create("v8_ro_slab", MFD_CLOEXEC | MFD_ALLOW_SEALING);
ftruncate(fd, kSlabSize);

// Map writable, populate from snapshot.
void* w = mmap(nullptr, kSlabSize, PROT_READ|PROT_WRITE,
               MAP_SHARED, fd, 0);
DeserializeReadOnlySnapshotInto(w, snapshot_blob);
munmap(w, kSlabSize);

// Seal the fd: no more growth, no more writes from any future mapping.
fcntl(fd, F_ADD_SEALS, F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_WRITE);

// Stash fd in the process-level singleton.
SharedRoSlab::SetFd(fd, kSlabSize);
```

After `F_SEAL_WRITE`, even a `MAP_SHARED` mapping cannot be made
PROT_WRITE. The kernel enforces it. **Every isolate's mapping of
this fd is read-only at the kernel level**, which is the same
isolation property V8's per-isolate `mprotect(PROT_READ)` provides
today, just process-wide.

### Phase B — Per-cage attach

When V8 allocates a cage, it currently does
`mmap(addr, kCageSize, PROT_NONE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`
to reserve the 4 GB virtual region, then commits sub-regions later.

We hook in *immediately after* the cage reservation:

```cpp
void Cage::Attach(Address cage_base) {
  // Overwrite the first kSlabSize bytes of the cage with our shared
  // mapping. MAP_FIXED replaces whatever was there.
  void* p = mmap(reinterpret_cast<void*>(cage_base),
                 SharedRoSlab::Size(),
                 PROT_READ,
                 MAP_FIXED | MAP_SHARED,
                 SharedRoSlab::Fd(), 0);
  CHECK_EQ(p, reinterpret_cast<void*>(cage_base));
}
```

After this, dereferencing `cage_base + raw_value` for any
`raw_value < kSlabSize` from any cage hits the same physical pages.
The compressed-pointer decompression code is not changed.

### Phase C — Hook the ReadOnlyHeap

`SoleReadOnlyHeap::shared_ro_heap_` is a process-level singleton.
Today it's only populated when `V8_COMPRESS_POINTERS_IN_SHARED_CAGE`
is on. We add a parallel populated-via-memfd path:

```cpp
// In SoleReadOnlyHeap::Setup
if (v8_flags.shared_ro_heap_via_memfd && SharedRoSlab::IsAvailable()) {
  // Use the slab fd; root pointers reference offsets into the slab
  // which are visible in every cage that has Attach()'d.
  AttachSharedSlabRoots(isolate);
  return;
}
```

The root table entries become offsets into the slab. Since every
cage maps the slab at offset 0, the offsets are universal.

### Phase D — IsReadOnlySpaceShared()

The current check stays for backward compat. We add a parallel test:

```cpp
static constexpr bool IsReadOnlySpaceShared() {
  return (V8_SHARED_RO_HEAP_BOOL &&
          (!COMPRESS_POINTERS_BOOL || COMPRESS_POINTERS_IN_SHARED_CAGE_BOOL))
         || v8_flags.shared_ro_heap_via_memfd;
}
```

This must be `constexpr false` when the feature is off. The
`v8_flags.shared_ro_heap_via_memfd` reference would lift the
`constexpr`; for the prototype we make it a runtime check (fine
because the only callsites that matter are init-time).

## Why this works (and why it took us 3 hours of staring at V8 to see)

1. **Cage base alignment.** `kPtrComprCageBaseAlignment = 4 GB`. So
   `cage_base & 0xFFFFFFFF == 0`. The bottom 32 bits of any address
   in the cage *equal* the compressed pointer value. That's the
   whole point of pointer compression.

2. **MAP_FIXED is destructive.** It replaces whatever is at the
   target virtual address. Cages today reserve the whole 4 GB with
   `PROT_NONE`; overwriting the first kSlabSize bytes is fine.

3. **memfd + F_SEAL_WRITE = process-wide read-only.** No isolate can
   remap this region writable. Promote-on-write requires copying
   into the isolate's mutable heap (the existing
   `JSReceiver::SetPropertyOrElement` slow path), exactly as today's
   `SoleReadOnlyHeap` requires.

4. **No GC interaction.** The slab is never GC'd (process-lifetime).
   GC sees pointers into the slab as foreign, treats them like
   read-only-heap pointers it already knows about — same machinery.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| ASLR places cages at arbitrary addresses | Cage allocation always uses 4 GB alignment. ASLR varies *which* aligned slot, not the alignment itself. Slab attaches at cage_base + 0 always. |
| Cage_base + raw_value collides with non-slab object | Slab occupies first N bytes of cage; V8's memory layout already puts ReadOnlySpace at cage start (today, per-isolate). We just substitute the contents. |
| Pointer compression for sandbox external pointers | Sandbox uses a separate cage (`TrustedCage` / external pointer table). This change touches the main cage only. Sandbox is unaffected. |
| Windows | `memfd_create` is Linux-only. Windows equivalent: `CreateFileMapping(INVALID_HANDLE_VALUE, ...)` + `MapViewOfFileEx(..., addr)`. Same semantics. macOS: `shm_open` + `mmap`. Phase 1 ships Linux-only behind a flag. |
| Existing per-isolate ReadOnlyHeap allocations | Skip them when shared slab is active; root table points into slab instead. Per-isolate ReadOnlyHeap allocator runs but allocates zero pages. |

## What this prototype proves (in `cross_cage_shared_ro_proto.cc`)

A standalone C++ program that:

1. Creates a memfd, populates it with a "fake snapshot" (recognisable
   byte pattern), and seals it.
2. Reserves N "fake cages" (4 GB each) and attaches the memfd at
   each cage's offset 0.
3. Reads the same compressed pointer (e.g. `0x1234`) decoded against
   each cage's base — verifies all return the same byte pattern from
   the slab.
4. Tries to write to the slab via any of the cage mappings — verifies
   the kernel rejects it (SIGSEGV inside a `sigsetjmp` guard).
5. Reports the *physical* memory cost with `/proc/self/smaps` — shows
   the slab counts once across all "isolates" (PSS-style accounting).

This is the minimal viable proof that cage-compatible shared RO heap
is possible *with current Linux primitives*, no V8 changes needed in
the compressed-pointer path.

## What this prototype does NOT prove

- Doesn't run V8. A V8 integration is the actual CL (skeleton in
  `v8_phase_a_patch.diff`).
- Doesn't measure octane / speedometer regression. The hot decode path
  is unchanged from today's per-isolate decompression, so we expect
  zero regression — but only the V8 perf-bots can confirm.
- Doesn't address Windows / macOS — Linux-first, others follow.

## Memory savings projection

From `kenton-response/results/SUMMARY.md` (3-run median, V8 12.8):

- stage_0 (post-isolate, pre-context) heap: **10.4 KB / isolate**
- That whole chunk becomes shared once across all isolates in the
  process.

At workerd-scale (1000 isolates per node):
- Today: 1000 × 10.4 KB = 10.4 MB duplicated
- With this design: 10.4 KB once + 1000 × 0 = 10.4 KB total
- **Saving: ~10.4 MB per process** (RSS, not just virtual)

This is conservative — it counts only the snapshot-derived used-heap.
The committed-but-unused page reduction (Phase 2 in our wider RFC)
is much larger but lives separately from this CL.

## Engineering effort

- Phase A prototype (this folder): **2 hours**, done.
- V8 Phase B+C patch (production-quality, mjsunit-clean): **2-3 weeks**.
  Mostly review iteration.
- Windows + macOS port: **1 week each**, mostly mechanical.

Total to merge in V8 trunk: **~2 months**, gated mostly by V8 review
cycles, not raw implementation time.
