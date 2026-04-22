# RFC: Copy-on-Write Shared Built-ins for V8

**Status:** draft, seeking V8 team feedback before any PR.
**Author:** kenton-response contributors (responding to workerd#6595).
**Target:** v8-dev@ first, workerd team second.

## Problem statement

On workerd (and any multi-tenant V8 deployment), every isolate pays full
heap memory cost for all built-ins — `Array.prototype`, `Object`,
`String.prototype`, `JSON`, `Math`, `Promise`, `Error` subclasses, etc.
The snapshot mechanism deserializes them into the isolate's heap on
startup.

For a workload where the vast majority of isolates never mutate any
built-in (the realistic case — monkey-patching built-ins in production
workers is rare and visible), the per-isolate built-in footprint is
effectively read-only waste duplicated N times.

Kenton Varda (workerd maintainer) summarised the direction in
cloudflare/workerd#6595:

> The right way to do this is to design a JS engine ... the engine's heap
> representation should already contain just the "live variables" and
> nothing else. ... you would need some way to keep the built-in objects
> off of the heap unless the app code actually modifies them — some sort
> of copy-on-write.

This RFC is a concrete shape for that direction, scoped to something
implementable as an extension of V8's existing `ReadOnlySpace`.

## What already exists in V8

- `v8::internal::ReadOnlySpace` / `ReadOnlyHeap` — per-isolate, populated
  from the snapshot at `Isolate::New()`. Already marked read-only; writes
  fault.
- `SharedIsolate` / shared heap (v10+) — shares some immutable data
  (strings, shared function info) across isolates within a process.
- `HeapLayout::InReadOnlySpace()` — fast tag check used on the hot path.

V8 is already plumbed for "this object is not on the mutable heap". The
gap: the shared version is small today; extending it to cover the bulk
of built-ins is a scope-and-GC question, not a new-primitive question.

## Proposal

Add a new memory space called `kReadOnlySharedSpace` that is:

1. Allocated **once per process** via `mmap(…, MAP_PRIVATE | PROT_READ)`
   from a snapshot file. All isolates in the process point into the same
   physical pages. OS page-cache handles the sharing — no new V8
   machinery for that.
2. Populated from the `SnapshotData` blob at process startup (not at
   `Isolate::New()`).
3. Visible to every new isolate via a root table entry.
4. **Copy-on-write** at the object level: if user code ever mutates a
   property on an object in `kReadOnlySharedSpace`, the object is copied
   into the per-isolate mutable heap, the reference is patched, and the
   mutation proceeds on the copy.

### Integration points

Three code paths need to change:

1. **Snapshot deserialisation** (`src/snapshot/deserializer.cc`) — teach
   it that snapshot objects can live in shared space, skipping per-isolate
   allocation when the shared slab is already populated.

2. **Write barrier / property-set** (`src/objects/js-objects.cc`,
   `SetPropertyOrElement`) — detect writes to `kReadOnlySharedSpace`,
   invoke the COW-promote routine instead of the usual fast path.

3. **Root initialisation** (`src/init/setup-isolate.cc`) — populate the
   root table with pointers into the shared slab for the chosen built-ins.

### Scope: which built-ins

Start with 30 objects corresponding to the top-level JavaScript
intrinsics:

`Array`, `Array.prototype`, `Object`, `Object.prototype`, `String`,
`String.prototype`, `Number`, `Number.prototype`, `Boolean`,
`Boolean.prototype`, `Symbol`, `Symbol.prototype`, `Error`,
`TypeError`, `RangeError`, `SyntaxError`, `URIError`, `ReferenceError`,
`Promise`, `Promise.prototype`, `Map`, `Map.prototype`, `Set`,
`Set.prototype`, `JSON`, `Math`, `Date`, `Date.prototype`, `RegExp`,
`ArrayBuffer`.

These were picked because (a) they're large collectively, (b) they're
rarely mutated by production code, and (c) they're reachable from a
fixed, well-known set of roots, so the pointer-rewrite story is simple.

## What we're not proposing

- Sharing **mutable** built-in state (e.g., `Date.now()` state, `Math`
  random state) — those either don't exist or have to stay per-isolate.
- Sharing user-allocated objects across isolates. That's the shared-heap
  project; out of scope here.
- Changing the GC. The shared slab is never GC'd; it lives as long as
  the process. Per-isolate promotions are GC'd normally.

## Open questions for V8 reviewers

1. **Promotion identity:** when `Array.prototype` is promoted in isolate
   X, the reference in X's `native_context` must point at the new copy.
   But `Array.prototype.map` (in the shared slab) still points back at
   the original `Array.prototype`. When `map` is called from X, `this`
   is the promoted copy, so that's fine — but any place in V8 that
   compares prototypes by pointer identity would break. How much of the
   codebase does that? (I don't yet know.)

2. **Snapshot compatibility:** if the shared slab's snapshot version
   drifts, every isolate in the process must agree. What's the
   versioning story for the slab file?

3. **Security:** is `MAP_PRIVATE | PROT_READ` sufficient isolation
   between tenants? If one tenant can somehow trigger a write to a
   shared page, they'd get an isolation leak via segfault side-channel
   (timing). I believe V8 never issues direct writes to RO space today;
   confirming this would be part of the patch review.

4. **Sandbox interaction:** V8's pointer compression + sandbox model
   uses 4GB isolate cages. The shared slab must either fit in every
   cage (possibly using a dedicated cage region) or use an indirection.
   This is probably the biggest implementation question.

## Success criteria

We'd consider this worth merging if, with a realistic workload of N
fresh isolates each running a small worker handler once:

- Per-isolate RSS drops by ≥ 20%.
- No regression on octane / speedometer / jetstream beyond noise
  (<1% p50, <3% p99).
- All existing mjsunit builtin tests pass.
- The promote path is < 500 ns per first-write (measured; we'd include
  that microbench).

If we can't hit 20% at minimum, the engineering cost isn't justified
and we drop the proposal. We will publish the negative result with the
measurements that led to it.

## What's in this folder

- `memory_benchmark.cc` — stock-V8 isolate memory footprint benchmark.
  Run against an unpatched V8 to get the baseline. Self-contained C++.
- `heap_classifier.cc` — tells you what fraction of per-isolate heap is
  snapshot-derived (the COW candidate).
- `cow_builtins_prototype.cc` — standalone prototype of the shared-slab
  + promote data structure. Doesn't link V8; validates the concept
  independently of engine integration.
- `patch_skeleton.diff` — a dry run of the V8 file touches the full
  implementation would make (flag, class stubs, hook locations). This
  is shaped as a *starting point* for a real patch, not a merge-ready
  change.

## Asking V8 team

Before we spend 4-8 weeks implementing the full patch, we'd like
feedback on whether:

- The sandbox/cage question has a known answer we're missing.
- The pointer-identity concern is a showstopper.
- This direction is already in progress inside Google and we'd be
  duplicating effort.

If the answer to any of those is "no problem" / "do it", we'll build it.
