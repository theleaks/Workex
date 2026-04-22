# Kenton's Feedback — Technical Response Plan

Kenton (workerd#6595) rejected the pluggable-runtime direction. But he
gave three concrete technical problems that any continuation-style
runtime has to solve. This folder exists to solve them, measurably, and
send a PR Cloudflare can merge.

## What Kenton said, in his own words

> "presumably gets very expensive as the in-memory state grows, as you
> end up serializing and deserializing everything on every I/O"

> "you presumably need to set up a new execution environment (isolate)
> every time you want to deserialize the state to run a continuation,
> and that is presumably pretty expensive"

> "The _right_ way to do this is to design a JS engine which minimizes
> memory usage without the need to do any serialization / parsing on
> every I/O. The engine's heap representation should _already_ contain
> just the 'live variables' and nothing else. In theory this is
> possible, but to actually implement the JS language, you would need
> some way to keep the built-in objects off of the heap unless the app
> code actually modifies them — some sort of copy-on-write."

## Three problems, three targeted responses

### P1 — Serialization cost per I/O (Kenton's "presumably expensive")

He says "presumably". Nobody has measured. We measure it, honestly,
including all the parts our V9 benchmarks skipped:

- Full suspend cycle: live-variable scan + serialize + heap free
- Full resume cycle: new execution env setup + deserialize + rebind refs
- CPU time per I/O at 1/10/100 KB of live state
- Break-even analysis: at what I/O rate does CPU cost exceed memory gain?

Output: `benchmarks/p1_serialization_cost.cc` (links V8 directly; uses
`v8::ValueSerializer` / `ValueDeserializer` so numbers are V8's own)

### P2 — Resume requires fresh execution environment

He's right — our current scheduler reuses a warm context. Real
production would need isolation between agents, so every resume pays
the setup cost of a fresh isolate. We measure that cost, both in the
"cold fresh isolate" and "pooled pre-warmed" case, and report the gap.

Output: `benchmarks/p2_resume_cost.cc` (three configs A/B/C, each
isolation-tested; A = fresh isolate per resume, B = pooled isolate +
fresh context, C = pooled context with explicit leak verification)

### P3 — Copy-on-write built-ins

This is the one Kenton said is "the right way". V8 doesn't do this
today. A full patch is 4-8 weeks of V8-specialist work. We ship the
*honest shape* of the contribution instead — RFC for v8-dev@, runnable
baseline benchmarks, a standalone C++ prototype of the data structure,
and a skeleton patch showing all hook locations:

- `src/v8-cow/RFC.md` — design for v8-dev@
- `src/v8-cow/memory_benchmark.cc` — stock-V8 isolate memory footprint
- `src/v8-cow/heap_classifier.cc` — per-isolate heap-composition at
  four stages, upper-bound on COW savings
- `src/v8-cow/cow_builtins_prototype.cc` — standalone shared-slab +
  promote data structure (no V8 dep)
- `src/v8-cow/patch_skeleton.diff` — V8 patch: flag, class, hook
  locations, `UNIMPLEMENTED()` at the promote site

Output: `src/v8-cow/` — RFC + benchmarks + prototype + skeleton patch

## What we send to Cloudflare

One cohesive report:

1. Honest numbers on P1/P2 — showing the real CPU/memory trade-off
2. A working V8 COW builtins prototype (P3) with numbers
3. A narrow, mergeable PR: either (a) V8 patch if the prototype is
   clean enough, or (b) a workerd benchmark harness contribution that
   lets the workerd team measure isolate memory themselves

The goal isn't to make Workex win. It's to hand Cloudflare three pieces
of work they can use, with every number reproducible.

## Folder layout

```
kenton-response/
├── PLAN.md                           # this file
├── README.md                         # quickstart + status
├── build_all.sh                      # top-level: fetch V8, build everything
├── run_all.sh                        # top-level: run all benchmarks
├── docs/
│   ├── P1_METHODOLOGY.md             # how we measure serialization cost
│   ├── P2_METHODOLOGY.md             # how we measure resume cost
│   └── P3_DESIGN.md                  # V8 COW builtins design
├── benchmarks/
│   ├── common.h / common.cc          # shared workload + stats + V8 init
│   ├── p1_serialization_cost.cc      # P1 (links V8)
│   ├── p2_resume_cost.cc             # P2 (links V8)
│   ├── CMakeLists.txt
│   ├── fetch_and_build_v8.sh         # one-time V8 fetch + build
│   ├── build.sh / run.sh
│   └── README.md
├── src/v8-cow/
│   ├── RFC.md                        # v8-dev@ design proposal
│   ├── cow_builtins_prototype.cc     # standalone, no V8 dep
│   ├── memory_benchmark.cc           # isolate RSS benchmark (links V8)
│   ├── heap_classifier.cc            # heap-composition stages (links V8)
│   ├── patch_skeleton.diff           # V8 patch: flag + class + hooks
│   ├── CMakeLists.txt
│   ├── build.sh
│   ├── apply_patch_and_build.sh
│   └── README.md
└── PR_DRAFTS/
    ├── workerd-issue-6595-response.md   # comment on #6595
    ├── v8-dev-rfc.md                    # email to v8-dev@
    ├── workerd-benchmark-harness-pr.md  # optional benchmark PR
    └── README.md
```

## Execution order (actual, as shipped)

1. P1 benchmark — `benchmarks/p1_serialization_cost.cc` — done
2. P2 benchmark — `benchmarks/p2_resume_cost.cc` — done
3. P3 deliverables — `src/v8-cow/` — done (RFC + baseline benchmarks +
   standalone prototype + skeleton patch; full V8 implementation pending
   v8-dev@ feedback)
4. PR drafts — `PR_DRAFTS/` — done
5. Run + collect results — any Linux x86_64 with 20 GB free:
   `./build_all.sh && ./run_all.sh`

No Workex claims. No "981x". Just: "here are the numbers you asked for,
here's the shape of the thing you said is the right way."
