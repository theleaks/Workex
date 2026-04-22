# P3 — Copy-on-Write Built-ins for V8 (Design)

## The question Kenton raised

> "The _right_ way to do this is to design a JS engine which minimizes
> memory usage without the need to do any serialization / parsing on
> every I/O. The engine's heap representation should _already_ contain
> just the 'live variables' and nothing else. In theory this is
> possible, but to actually implement the JS language, you would need
> some way to keep the built-in objects off of the heap unless the app
> code actually modifies them — some sort of copy-on-write. Otherwise,
> the built-ins alone occupy quite a bit of space."

This is a real, open problem in V8. If we can demonstrate a workable
prototype, it's the kind of contribution that actually changes what
workerd can do — without replacing it.

## What already exists in V8 (our starting point)

V8 has a snapshot system for fast isolate startup:

- `Isolate::CreateParams::snapshot_blob` — a binary blob containing the
  pre-initialised heap state for built-in objects (Array, Object, JSON,
  etc.)
- On `Isolate::New()`, V8 deserializes the blob into the isolate's heap.
  Fast (~1 ms) compared to re-running all built-in initializers.
- BUT: the deserialized objects live on the new isolate's heap. So
  every isolate still pays the memory cost of all built-ins.

Kenton's proposal, translated to V8 terms: **don't deserialize into the
isolate's heap at all. Map the snapshot read-only, fault on write,
lazily copy only the modified built-ins into the isolate's writable
heap.**

## Prototype scope (what we ACTUALLY build)

V8 is massive. We don't reimplement the GC. We do the minimum that
proves the concept:

1. Add a new allocator mode `kReadOnlyShared` to V8's heap, similar to
   the existing `read_only_space` but mapped once-per-process instead
   of once-per-isolate.
2. Tag every pointer to a built-in object as coming from `kReadOnlyShared`.
3. Intercept writes to those objects via V8's existing
   `SetPropertyOrElement` path — on first write, promote the object to
   the per-isolate heap (shallow copy) and update the reference.
4. Do this only for the top-level built-in constructors (~30 objects:
   Array.prototype, Object.prototype, String.prototype, Number,
   Boolean, Symbol, Error subclasses, Promise, Map, Set, JSON,
   Math, Date, RegExp, ArrayBuffer, Typed Arrays).

## Success criteria

A reproducible benchmark that shows:

- hello-world isolate memory: baseline X MB → with prototype Y MB (Y < X)
- Every existing V8 test in the `test/mjsunit/` "builtins" category
  still passes.
- CPU overhead per built-in access < 2% (we measure with
  octane / speedometer).

If Y/X improvement is <20%, the prototype is not worth it and we say so.

If Y/X improvement is >50%, we have a case to take to V8 v8-users@
and then propose it as a workerd optimization path.

## Implementation plan

### Week 1: build + measure baseline
- Clone V8 at stable version matching workerd's
- Build with gn/ninja on Linux
- Write an isolate-memory benchmark in C++ (links V8 directly)
- Measure: 1/10/100/1000 isolates memory footprint with various built-in
  access patterns
- This benchmark is shareable with the V8 team regardless of the
  prototype outcome

### Week 2: identify built-ins on heap
- Dump an isolate's heap after minimal Worker run
- Classify objects: "touched by user code" vs "pure built-in"
- Quantify how much memory is spent on untouched built-ins (upper bound
  on how much COW can save)

### Week 3-4: implement
- Add `kReadOnlyShared` space backed by `mmap(MAP_SHARED | PROT_READ)`
- Move snapshot built-ins into it
- Add write-barrier check — if write to `kReadOnlyShared` object,
  copy-on-write into regular heap
- Update pointer-following paths to handle cross-space references

### Week 5: test + measure
- Run V8 test suite, triage regressions
- Re-run isolate memory benchmark, report delta
- Write up the prototype as either:
  - A V8 blog post-style write-up (if >20% memory savings)
  - A negative result report with the measurements (if not)

## What we're NOT doing

- Shipping this. It's a prototype. V8 team would need to productionize it.
- Claiming we invented this. Firefox's SpiderMonkey has investigated
  similar; V8 has discussed it in v8-dev@ threads. We're making it
  concrete enough to evaluate.
- Replacing V8. We're making V8 better, in the one specific way Kenton
  pointed at.

## Decision points

- If V8 build fails on the target host after 2 days of debugging →
  pivot to hybrid: work on V8 in Docker, document exact setup.
- If the read-only snapshot is impossible to separate from the
  writable heap without touching the GC → escalate to v8-dev@ as an
  RFC before more code.
