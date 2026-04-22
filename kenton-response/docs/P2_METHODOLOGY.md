# P2 — Resume Cost: Fresh Execution Environment (Methodology)

## The question Kenton raised

> "you presumably need to set up a new execution environment (isolate)
> every time you want to deserialize the state to run a continuation,
> and that is presumably pretty expensive"

In real production, agents must be isolated — so reusing a warm
scheduler/context across unrelated agents isn't acceptable. Every
resume, in the strict model, pays fresh-isolate setup.

## Three configurations measured

Measured on V8 (not QuickJS — the original methodology mentioned QuickJS
because we were looking at Workex; the honest answer is to measure V8,
which is what workerd actually runs):

| Config | Description |
|---|---|
| **A** | Fresh `v8::Isolate` + `v8::Context` per resume. No pool. Strict isolation. |
| **B** | Pooled Isolate (one per process), fresh Context per resume. This is the reference "warm" strategy used by most workerd-style systems. |
| **C** | Pooled Context — reused across resumes. NOT isolation-safe; verified by explicit leak test, reported with `isolation_safe: false`. |

## What we measure

Iteration counts:
- A = 500 (fresh isolate is slow; this keeps wall-clock bounded)
- B = 10,000
- C = 10,000

Plus 100 warmup iterations per config, discarded.

Per resume:
- `setup_ns`: time to create `Context` + compile entry-point script
- `deserialize_ns`: `ValueDeserializer::ReadValue` (shared M-sized state)
- `first_instruction_ns`: entry function call, returns first user result
- `total_resume_ns`: sum of the above

## Isolation test

For config B and C, verify that agent A's state cannot leak into
agent B:

```js
// Agent A writes to a global
globalThis.__leaked_secret = "from_A";

// After resume of agent B, this MUST be undefined
assert(globalThis.__leaked_secret === undefined);
```

Any config that fails this test is reported as "NOT isolation-safe" and
its throughput numbers come with a caveat.

## Output

`benchmarks/p2_resume_cost.cc` emits a JSON array to stdout; `run.sh`
redirects it to `benchmarks/results/p2.json`:

```json
[
  {
    "config": "A",
    "isolation_safe": true,
    "isolation_check_passed": true,
    "iterations": 500,
    "setup_ns":              { "mean": ..., "median": ..., "p99": ..., "min": ..., "max": ..., "samples": 500 },
    "deserialize_ns":        { ... },
    "first_instruction_ns":  { ... },
    "total_resume_ns":       { ... }
  },
  { "config": "B", ... },
  { "config": "C", "isolation_safe": false, "isolation_check_passed": true, ... }
]
```

For C, `isolation_safe` is always `false` by design; the benchmark
verifies the leak is observable (and `isolation_check_passed` confirms
the test correctly detected it).

## Honest comparison to V8

V8 isolates retain their execution env on suspension (await); no resume
setup cost at all — they just continue. Our P2 numbers are the cost
V8 simply doesn't have. We report that gap plainly in the final report.

The continuation approach wins only if:

- Memory pressure is the dominant constraint (e.g. millions of idle
  agents), AND
- The resume rate is low enough that the P2 cost doesn't dominate
  latency (e.g. long LLM waits).

Both of these need to be argued with numbers, not asserted.
