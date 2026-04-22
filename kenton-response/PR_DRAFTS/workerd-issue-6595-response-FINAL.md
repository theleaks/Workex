# Response on workerd#6595 — with measurements

**Post as:** comment on cloudflare/workerd#6595
**Format:** GitHub markdown
**Every number below is reproducible** via `docker build -t kenton-bench . && docker run --rm -v "$(pwd)/results:/work/results" kenton-bench` against V8 12.8.374.38.

---

Thanks @kentonv — each of your three objections landed, and instead of
patching up the original issue I spent the time measuring what you
said was unmeasured. Everything here is on V8 12.8.374.38 directly
(linked against `libv8_monolith.a`); no Rust, no Workex runtime.

Host: 16-core Intel Core Ultra 9 285H, 16 GB RAM, Ubuntu 24.04 under
WSL2. Raw JSON, heap snapshots, and `env.txt` with CPU details are in
the repo at `results/`.

Full reproducibility:
`https://github.com/<owner>/kenton-response` —
`docker build -t kenton-bench . && docker run --rm -v "$(pwd)/results:/work/results" kenton-bench`
produces everything below.

## 1. "Serializing and deserializing everything on every I/O"

Measured via `v8::ValueSerializer::WriteValue` / `ValueDeserializer::ReadValue` — V8's own structured-clone primitives, so this is the CPU floor for any continuation-style suspend/resume scheme on workerd.

| Workload | Serialized | Serialize mean / p99 | Deserialize mean / p99 | Round-trip mean |
|---|---|---|---|---|
| XS (1 number + short string) | 27 B     | **319 ns / 543 ns**   | **766 ns / 3.17 µs**    | 1.09 µs |
| S  (req url + method + headers) | 185 B  | **646 ns / 1.16 µs**  | **2.77 µs / 6.01 µs**   | 3.41 µs |
| M  (req + cache + small body)   | 4.2 KB | **1.25 µs / 2.55 µs** | **10.13 µs / 16.49 µs** | 11.37 µs |
| L  (M + 1024-dim vector)        | 56.2 KB| **59.64 µs / 155.77 µs** | **290.63 µs / 490.64 µs** | 350.27 µs |
| XL (M + 30-turn chat)           | 455.2 KB| **59.33 µs / 385.83 µs** | **997.13 µs / 10.56 ms** | 1.06 ms |

**Interpretation.** At M (5 KB of live state — a typical request-handling
agent), the round-trip is **~11 µs per I/O**. A continuation runtime that
awaits-and-serializes on every I/O spends ~0.1% of CPU on the serializer
at 100 I/O/sec per agent; at 10,000 I/O/sec it's 11% of one core. At XL
(LLM chat history in live state, 455 KB), the round-trip is **1.06 ms**
per I/O. Deserialize is 3-15× slower than serialize across the whole
range. You were right: this gets expensive, and it grows with live-state
size, not just request count.

## 2. "Set up a new execution environment... pretty expensive"

Three configurations, same V8 build, same 5 KB state round-tripped through `ValueSerializer`:

| Config | Iso-safe? | Setup mean | Total resume mean | Total p99 |
|---|---|---|---|---|
| **A** — fresh Isolate + Context per resume | ✓ | **1.73 ms**  | **1.76 ms**  | 3.72 ms |
| **B** — pooled Isolate, fresh Context per resume | ✓ | **303.52 µs** | **318.85 µs** | 951.93 µs |
| **C** — pooled Context (**NOT** isolation-safe — leak confirmed) | ✗ | **64 ns** | **3.46 µs** | 22.37 µs |

The isolation test explicitly verified that Config C leaks `globalThis.__leaked_secret` across unrelated resumes — its numbers are reported only so the gap to A/B is visible.

**Interpretation.** Strict multi-tenant isolation (Config A) costs
**1.73 ms of setup per resume** — three orders of magnitude more than
reusing a context. V8's suspend-on-await model skips this cost entirely
because it retains the isolate across the await. A serialisation-based
continuation runtime that wants real isolation pays Config A on every
resume.

## 3. "The right way is copy-on-write built-ins"

Baseline stock-V8 per-isolate memory (`src/v8-cow/memory_benchmark.cc`):

| Pattern | 1 isolate | 100 isolates | 1000 isolates |
|---|---|---|---|
| hello (no JS run) | RSS 12.4 MB, heap-used 10.4 KB | RSS 69.7 MB | RSS 588 MB, heap-used 10.1 MB |
| basic JS          | RSS 588 MB, heap-used 93.9 KB | RSS 588 MB, heap-used 9.2 MB | RSS 825.8 MB, heap-used 91.7 MB |
| realistic handler | RSS 825.8 MB, heap-used 95.9 KB | RSS 825.8 MB | RSS 828.3 MB, heap-used 93.7 MB |

Per-isolate RSS at 1000 isolates (hello): ~588 KB. At realistic
(worker handler run once per isolate): ~828 KB.

Heap classifier — single isolate, four stages:

| Stage | Used heap | Δ |
|---|---|---|
| post-isolate (no context)  | 10.4 KB | — |
| post-context                | 90.5 KB | +80.1 KB |
| post-minimal-JS             | 94.0 KB | +3.4 KB |
| post-realistic-handler      | 96.4 KB | +2.4 KB |

**COW upper bound.** The snapshot-derived portion (stage_0) is 10.4 KB
out of 96.4 KB of used-heap after a realistic handler run — **~11% of
per-isolate heap is the cleanly-sharable candidate**. The remaining
80 KB is per-context initialization, which a simple shared-builtins
scheme does *not* address — some of it might be addressable by a
shared-context template, but that's a bigger engineering ask.

The standalone data-structure prototype (`cow_builtins_prototype.cc`,
no V8 dep) shows the sharing+promote algorithm is not pathological
— ~99.57% sharing on synthetic data with realistic write rates —
so the bottleneck isn't the sharing itself; it's how much of V8's
heap is actually snapshot-derived. Based on the above, the answer
is "a meaningful slice, but not revolutionary": 10-20%.

This is why I'm **not** opening a V8 PR blindly. The RFC in
`src/v8-cow/RFC.md` asks v8-dev@ three specific questions before
committing the 4-8 weeks:

1. **Pointer identity:** how many hot paths compare built-ins by
   pointer, and how much surgery does the promotion path entail?
2. **Sandbox cage interaction:** does a shared-across-process slab
   fit the V8_ENABLE_SANDBOX cage model, or does it need cage-relative
   indirection?
3. **Is Google already doing this internally?** — if yes, I stand
   down.

`src/v8-cow/patch_skeleton.diff` applies cleanly against V8 HEAD and
adds `CowSharedBuiltins` class + flag stubs. `PromoteOnWrite` is
`UNIMPLEMENTED()` by design; `src/v8-cow/hooks_reference.md` documents
the four edits in existing files that would wire it in.

## What I'm no longer claiming

- The "185×–981× less than V8" framing was wrong. The "V8" column was
  measuring `vm.createContext()`, not a workerd isolate. Real per-isolate
  RSS is ~588 KB at 1000 isolates, not 180 KB.
- Toy workloads don't generalise. At M (5 KB) round-trip is 11 µs;
  at XL (500 KB) it's 1.06 ms. You were right that this grows.
- Continuation-style runtimes are **not** unconditionally better than
  V8 isolates. They win only when (a) memory pressure is the dominant
  constraint and (b) resume rate is low enough that 1.7 ms A-config
  or 320 µs B-config setup doesn't dominate latency. These are
  narrow conditions; the original framing ignored them.

## Offer

Either (or both):

1. **v8-dev@ RFC** for the COW builtins work, posted after I get your
   signal that it's a direction workerd would welcome. If yes, the
   4-8 weeks is mine to spend. If no (or "not now"), I drop it.

2. **Benchmark-harness PR to workerd** — P1 and P2 adapted to
   `src/workerd/tests/benchmarks/`, so the workerd team has these
   tools in-tree for anyone measuring isolate costs. No runtime
   changes. Draft at `PR_DRAFTS/workerd-benchmark-harness-pr.md`.

Happy to do whichever is more useful, or drop both if this is not
the direction you want.
