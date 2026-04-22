# V8 RFC: Copy-on-Write Shared Built-ins (draft for v8-dev@)

**Send to:** v8-dev@googlegroups.com
**Subject:** RFC: copy-on-write shared built-ins across isolates
**Thread posture:** RFC looking for a go/no-go before writing the patch.

---

Hi v8-dev,

Posting this as an RFC before writing the patch, because the
implementation scope is large enough that I'd like early feedback.

## Context

This is prompted by a discussion on workerd#6595
(https://github.com/cloudflare/workerd/issues/6595), where Kenton
Varda (workerd maintainer) articulated that the "right way" to
minimise per-isolate memory for the Workers-style multi-tenant case
would be to keep the built-ins off the isolate heap until user code
mutates them — i.e. copy-on-write built-ins.

Kenton's exact words:

> The right way to do this is to design a JS engine which minimizes
> memory usage without the need to do any serialization / parsing on
> every I/O. The engine's heap representation should already contain
> just the "live variables" and nothing else. In theory this is
> possible, but to actually implement the JS language, you would need
> some way to keep the built-in objects off of the heap unless the app
> code actually modifies them — some sort of copy-on-write. Otherwise,
> the built-ins alone occupy quite a bit of space.

I believe this is implementable as an extension of the existing
`ReadOnlySpace` / `ReadOnlyHeap` work, plus a write barrier hook.

## Proposal

Add a new space `kReadOnlySharedSpace` that is:

- **Process-wide**, mmap'd once from a snapshot file (`MAP_PRIVATE |
  PROT_READ`).
- Populated at process init; sealed before any isolate runs.
- Visible to every new isolate via the root table.
- **Copy-on-write** at the object level: on first store into a shared
  object, promote the object into the isolate's mutable heap and
  rewrite the reference.

Scope: ~30 top-level built-ins (Array, Object, String, JSON, Math,
Promise, Error subclasses, Map, Set, Date, RegExp, ArrayBuffer, …).
Chosen because they're large collectively, rarely mutated, and reached
from a fixed set of roots.

## Why this specifically

Three cost profiles in current V8:

1. **Fresh-isolate time (minutes of real work per isolate):** snapshot
   already fixed most of this.
2. **Per-isolate memory:** still dominated by the RO snapshot,
   duplicated per isolate. This is what COW targets.
3. **Cross-isolate shared data:** the shared heap project handles
   strings and code. This RFC is that approach applied to RO built-ins.

## What I've already done (to make the conversation concrete)

Available at
https://github.com/bedirhanplatinplus/workex/tree/master/kenton-response/src/v8-cow:

1. `RFC.md` — fuller design doc (what you're reading is the summary).
2. `cow_builtins_prototype.cc` — standalone C++ prototype of the
   shared-slab + COW-promote data structure, independent of V8. Shows
   the sharing ratio is good on a synthetic workload and that the
   promote path is simple.
3. `memory_benchmark.cc` — stock-V8 isolate memory footprint benchmark
   (links libv8_monolith.a). Gives the baseline COW would improve on.
4. `heap_classifier.cc` — measures per-isolate heap at four stages
   (post-isolate, post-context, post-minimal-JS, post-realistic-JS)
   so we can quantify the upper bound on savings.
5. `patch_skeleton.diff` — dry-run patch: adds the
   `--shared-readonly-builtins` flag, a `CowSharedBuiltins` class,
   and comments at the three hook locations (`ReadOnlyHeap::SetUp`,
   `SetPropertyOrElement`, snapshot deserialiser). Applies to
   `branch-heads/12.8`.

None of this is merge-ready. The patch skeleton compiles but
`PromoteOnWrite` is `UNIMPLEMENTED()`. The RFC is what I want your
feedback on before I spend the 4-8 weeks to make it real.

## Questions I want answered before I commit

1. **Pointer identity:** how many hot paths in V8 compare built-ins
   by pointer? If `Array.prototype` gets a promoted twin in isolate
   X, any check `obj.prototype == isolate->initial_array_prototype()`
   breaks unless the check goes through a resolver. Looking for a
   sense of blast radius.

2. **Snapshot versioning:** the shared slab is populated from one
   snapshot and reused by every subsequent isolate created in that
   process. What's the contract? (I assume: snapshot versions must
   exactly match, enforced by hash; any version mismatch refuses to
   use the shared slab.)

3. **Sandbox + cage interaction:** V8_ENABLE_SANDBOX pins isolate
   heaps to a 4GB cage. The shared slab either needs to fit inside
   every cage, or needs an indirection. Is this already a solved
   problem for the existing shared heap / shared strings path, and I
   just reuse that?

4. **Is Google already doing this?** If this is the externalised
   shape of an in-flight internal effort, I'd rather help than
   duplicate. If it's not on the roadmap and you'd take a patch,
   that's the signal I need to proceed.

## Success criteria (what I'll measure)

- Per-isolate RSS at 100 and 1000 isolates: ≥20% reduction.
- octane / speedometer / jetstream: no regression beyond noise
  (<1% p50, <3% p99).
- mjsunit: zero new skips; every existing test still passes under
  --shared-readonly-builtins.
- PromoteOnWrite: <500 ns per first-write.

If I can't hit the 20% floor, I'll publish the negative result with
measurements and stand down.

Thanks. Happy to discuss here, on Gerrit once there's a CL, or in a
chat if that's easier.

— [your name], for cloudflare/workerd#6595 response
