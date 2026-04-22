# P1 — Serialization Cost per I/O (Methodology)

## The question Kenton raised

> "as you end up serializing and deserializing everything on every I/O"

Our V9 benchmarks measured continuation size but NOT the cost of
building/tearing it down per await. That's the honest number we owe.

## Workload definition

Five live-state sizes, matching realistic agent scenarios:

| # | Size class | Contents |
|---|---|---|
| XS | ~50 B | one number, one short string (trivial) |
| S  | ~500 B | a request URL + method + 3 small headers |
| M  | ~5 KB | request + response cache + small JSON body |
| L  | ~50 KB | same + embeddings vector (1024 f32) |
| XL | ~500 KB | same + LLM chat history (30 messages) |

Each size × measured on V8 directly via `v8::ValueSerializer` /
`ValueDeserializer` — these are the primitives workerd itself would use
to snapshot live state, so the numbers are a floor on what any
continuation-style suspend/resume scheme on V8 would pay. No Workex, no
bincode.

## What we measure

Iterations scale with size (200k for XS/S, 100k for M, 20k for L, 2k
for XL) to cap wall-clock per class at a few minutes:

- `mean_ns`, `median_ns`, `p99_ns`, min, max for each of:
  - `serialize_ns` — `ValueSerializer::WriteValue` of the live state
  - `deserialize_ns` — `ValueDeserializer::ReadValue`
  - `roundtrip_ns` — sum of the two, per I/O
- `serialized_bytes` — ValueSerializer output size at each class
- `cycle_cpu_ns_mean` / `_p99` — convenient totals for the break-even
  calculation

## Break-even calculation

Given:
- CPU cost per I/O (measured above) = `C_io` ns
- Memory saved per suspended agent vs V8-idle = `M_save` bytes
- Request rate per agent = `R` I/O/sec
- Agent idle time = `T_idle` sec

Cost model:

```
CPU_loss  = N_agents × R × C_io × T_idle
Mem_gain  = N_agents × M_save
```

At what R does CPU_loss exceed Mem_gain × (CPU-cost per byte of memory)?
This tells us the regime where continuations are honestly a win vs
honestly a loss. Both regimes should be reported.

## Output

`benchmarks/p1_serialization_cost.cc` emits a JSON array to stdout;
`run.sh` redirects it to `benchmarks/results/p1.json`:

```json
[
  {
    "workload": "M",
    "target_bytes": 5120,
    "serialized_bytes": ...,
    "iterations": 100000,
    "serialize_ns":   { "mean": ..., "median": ..., "p99": ..., "min": ..., "max": ..., "samples": 100000 },
    "deserialize_ns": { ... },
    "roundtrip_ns":   { ... },
    "cycle_cpu_ns_mean": ...,
    "cycle_cpu_ns_p99":  ...
  },
  ...
]
```

## What is NOT fair to claim

- `ValueSerializer` is V8's own structured-clone path. A workerd-specific
  serializer could be faster or slower; we don't have that data.
- We do not include I/O latency itself (network round-trip). Only the
  CPU work workerd would have to do on top of the I/O.
- We report per-workload, not averaged. No cherry-picking across sizes.
- The workloads XS..XL are our best guess at realistic agent state;
  there's a subjective gap here. Results scale cleanly with
  `serialized_bytes` though, so readers can project for their own sizes.
