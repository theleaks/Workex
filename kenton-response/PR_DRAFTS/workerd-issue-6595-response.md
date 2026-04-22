# Response comment for cloudflare/workerd#6595

**Post as:** comment on issue 6595 (or a new issue if the thread is
stale). Format: GitHub markdown.

---

Thanks @kentonv — every point you raised landed, and I've been working
on the answer rather than responding faster.

Short version: the original issue measured the wrong thing, the
numbers it cited don't hold, and instead of trying to patch that up I
put together a response that takes each of your three technical
objections and measures them on V8 directly (not on our Rust runtime,
because your point about alternative JS engines is fair and
complicating the comparison with a different engine was exactly what
I shouldn't do).

Everything below is in
https://github.com/bedirhanplatinplus/workex/tree/master/kenton-response ,
reproducible on a fresh Linux x86_64 box via `./build_all.sh` +
`./run_all.sh`. Every number has a JSON file, every JSON file has the
code that produced it, and the code links libv8_monolith.a so the
measurements are V8's own numbers, not anything derived from Workex.

## Your three points, addressed

### 1. "Serializing and deserializing everything on every I/O"

`benchmarks/p1_serialization_cost.cc`. Uses `v8::ValueSerializer` /
`ValueDeserializer` — i.e. the mechanism workerd itself would use if
it wanted to snapshot a live state. Five workload sizes (XS 50 B, S
500 B, M 5 KB, L 50 KB, XL 500 KB). Reports mean / median / p99 for
serialize, deserialize, round-trip at each size, plus the break-even
request rate above which CPU loss exceeds memory saved.

This gives the **floor** on what a continuation-style suspend/resume
on V8 would cost. Any continuation scheme that uses ValueSerializer
pays at least this. A custom serializer could be faster; I don't
claim the floor is tight.

### 2. "You presumably need to set up a new execution environment..."

`benchmarks/p2_resume_cost.cc`. Three configurations, all measured on
V8:

- **A** — fresh Isolate + Context per resume. Strict isolation.
- **B** — pooled Isolate, fresh Context. (Warm reuse, single-tenant.)
- **C** — pooled Context. Not isolation-safe; I verify the leak
  explicitly in the test and label the result accordingly.

For each: `setup_ns`, `deserialize_ns`, `first_instruction_ns`,
`total_resume_ns`. Config A is the honest cost for strict
multi-tenant isolation and it's not hidden — it's in the output.

This makes concrete exactly what you said: V8 isolates retain their
execution env on suspension (no resume setup cost), and anything
serialisation-based pays the cost in A at least once per resume. The
numbers quantify the gap.

### 3. "The right way is copy-on-write built-ins"

`src/v8-cow/`. Not a merge-ready V8 patch — implementing this
properly is 4-8 weeks of V8-specialist time and I'd rather get v8-dev@
feedback before spending that. What's there:

- `RFC.md` — design for v8-dev@ covering the integration points
  (`ReadOnlyHeap::SetUp`, `SetPropertyOrElement`, snapshot
  deserialiser), scope (30 top-level built-ins), and open questions
  (pointer-identity, sandbox cage, snapshot versioning).
- `cow_builtins_prototype.cc` — standalone prototype of the shared-slab
  + promote data structure. Doesn't link V8; validates that the
  sharing+COW shape is non-pathological (~99% sharing on synthetic
  workloads with realistic write rates).
- `memory_benchmark.cc` + `heap_classifier.cc` — stock-V8
  measurements that establish the baseline COW would improve on. Both
  runnable today, useful to anyone working on isolate memory regardless
  of whether COW lands.
- `patch_skeleton.diff` — V8 patch that compiles but `UNIMPLEMENTED()`s
  at the promote site. Shows the file touches, flag, and hook
  locations. Applies against `branch-heads/12.8`.
- `PR_DRAFTS/v8-dev-rfc.md` — the message I'd send to v8-dev@.

## What I'm no longer claiming

- Workex is not faster than V8 on realistic workloads. The README
  claim was wrong; the "V8" column in that benchmark was measuring
  `vm.createContext()`, not a workerd isolate. I'm retiring that
  comparison.
- 183 KB per isolate is wrong; actual workerd isolate overhead is
  ~5 MB.
- "191–320 bytes of live state" was a toy workload. Any realistic
  agent has enough state that the serialisation CPU cost on every
  await is significant, which is exactly the point you made.
- The continuation-runtime framing as "solution" was overclaim. It's
  an experiment with a specific trade-off (memory vs. CPU), useful
  only if memory is the dominant constraint. P1+P2 make that
  trade-off quantifiable.

## What I'd like to contribute

Either (or both):

1. **Help on the V8 COW work** — starting with the RFC thread on
   v8-dev@. Happy to do the legwork; I need the "do it" /
   "don't duplicate internal effort" signal first. If the answer is
   yes, the 4-8 weeks is mine to spend.

2. **A benchmark harness PR to workerd** — P1 and P2 adapted to
   workerd's test tree, as a tool for the workerd team to measure
   isolate setup and serialisation costs without pulling in an
   external repo. Small, scoped, no runtime changes.

Happy to follow whichever of those is more useful, or to drop both if
the answer is that this isn't the right direction.
