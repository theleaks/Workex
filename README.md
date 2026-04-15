# Workex

**Agent-native JavaScript runtime. Drop-in Cloudflare Workers replacement with 2.8x isolate density.**

Built in Rust. Boa JS engine. Arena allocator. Zero GC pauses. Zero cold start.

---

## Why

Cloudflare CEO Matthew Prince, April 14 2026:

> "If the more than 100 million knowledge workers in the US each used an agentic assistant, you'd need capacity for approximately 24 million simultaneous sessions. We're not a little short on compute. **We're orders of magnitude away.**"

V8 isolates consume ~183KB each. Workex isolates consume ~64KB each. Same Workers API. Same developer experience. **2.8x more agents on the same hardware.** Measured with real OS-level RSS, not estimates.

---

## Benchmarks

All numbers measured on the same machine, same conditions, same test scripts. No static estimates. Three runtimes tested side-by-side every run.

### 10,000 Simultaneous Isolates (Real RSS)

Measured with `GetProcessMemoryInfo` (Windows) / `/proc/self/status` (Linux). Not struct size math.

| Metric | Workex | V8 (Node.js v24) | Factor |
|---|---|---|---|
| **Total RSS** | 627 MB | 1,785 MB | **2.8x less** |
| **Per isolate** | 64 KB | 183 KB | **2.8x less** |

```
cargo run -p workex-bench --release --bin rss-bench
```

### Worker Compatibility (hello.ts)

The same TypeScript Worker runs on all three runtimes. Correctness verified: body, status, headers.

```typescript
export default {
  async fetch(request: Request): Promise<Response> {
    return new Response("Hello from Workex!", {
      headers: { "content-type": "text/plain" },
    });
  },
};
```

| Metric | Workex | V8 (Node.js) | CF Workers (Miniflare) |
|---|---|---|---|
| Correct? | YES | YES | YES |
| Latency p50 | 114 us | 2.6 us | 1.06 ms |
| Latency p99 | 369 us | 14 us | 2.8 ms |
| **vs Workers** | **9.3x faster** | 408x faster | baseline |

```
cargo run -p workex-bench --release --bin worker-test
```

### HTTP Load Test (k6, 35s, 100 VU peak)

Three HTTP servers running identical endpoints. k6 hits each with the same script.

| Metric | Workex | Node.js | CF Workers | vs V8 | vs Workers |
|---|---|---|---|---|---|
| **Requests/sec** | **8,401** | 598 | 445 | **14x** | **19x** |
| /health p95 | 6.2 ms | 210 ms | 272 ms | 34x | 44x |
| /json p95 | 6.2 ms | 222 ms | 259 ms | 36x | 42x |
| /compute p95 | 14 ms | 171 ms | 327 ms | 12x | 23x |
| Error rate | 0% | 0% | 0% | | |

```
bash benchmarks/scripts/run-k6.sh
```

### Internals

| Metric | Value |
|---|---|
| Isolate creation | 900 ns median |
| Arena alloc+reset (100 objects) | 300 ns median |
| AOT compile (Cranelift JIT) | 18 us median |
| Native exec `add(f64,f64)` | ~1 ns/call |
| E2E TypeScript to native | 22 us median |

---

## Architecture

```
TypeScript Worker source
     |
     v
  oxc parser ──> Typed AST (TS annotations preserved)
     |
     +──> Boa JS Engine (full Worker execution)
     |
     +──> HIR (typed IR) ──> Cranelift JIT ──> Native code
     |
  Arena Allocator (request-scoped, O(1) reset)
     |
  Isolate Pool (spawn/recycle, pre-warmed)
     |
  Hyper HTTP Server (tokio async)
```

## Tech Stack

| Layer | Technology | Why |
|---|---|---|
| Language | Rust | Memory safety, zero-cost abstractions |
| JS Engine | Boa (pure Rust) | No C dependencies, no libclang |
| TS Parser | oxc | Fastest TS parser, Rust-native AST |
| AOT Backend | Cranelift | Rust-native JIT, used by Wasmtime |
| Allocator | Custom arena | Request-scoped, O(1) free, no GC |
| Async | Tokio | Production-proven async runtime |
| HTTP | Hyper | Zero-copy HTTP/1.1 server |
| Load Test | k6 | Industry-standard HTTP benchmarking |

---

## Project Structure

```
workex/
├── Cargo.toml                  Workspace root
├── crates/
│   ├── workex-core/            Isolate lifecycle, arena allocator, RSS measurement
│   ├── workex-compiler/        oxc parser, HIR, Cranelift codegen
│   ├── workex-runtime/         Workers API (Request, Response, KV, D1) + Boa engine
│   ├── workex-cli/             HTTP server (hyper + tokio)
│   └── workex-bench/           Benchmarks (micro, RSS, worker-compat, k6)
├── benchmarks/
│   ├── scripts/                k6 test, Node.js server, Workers (Miniflare), v8-rss
│   └── results/                Versioned JSON results (v1.json, v2.json, ...)
└── tests/
    └── workers/hello.ts        Reference Worker script
```

---

## Quick Start

```bash
# Build
cargo build --release

# Run tests (81 tests)
cargo test

# Run the Worker
cargo run -p workex-cli --release --bin workex-server

# Benchmarks
cargo run -p workex-bench --release --bin rss-bench      # 10K isolate RSS
cargo run -p workex-bench --release --bin worker-test     # Worker compatibility
cargo run -p workex-bench --release                       # Full micro-benchmark suite
bash benchmarks/scripts/run-k6.sh                         # k6 HTTP load test
```

---

## Workers API Compatibility

| API | Status | Notes |
|---|---|---|
| `export default { fetch }` | Working | TypeScript and JavaScript |
| `new Response(body, init)` | Working | Status, headers |
| `Request` (url, method) | Working | Passed to fetch handler |
| `Headers` | Working | Case-insensitive get/set/append/delete |
| `JSON.parse/stringify` | Working | Via Boa JS engine |
| `KV Namespace` | Mock | In-memory HashMap, API surface complete |
| `D1 Database` | Mock | Prepare/bind/run API, no real SQL backend |
| `fetch()` outbound | Stub | Returns error, not yet implemented |
| Streaming responses | Not yet | Planned |
| WebSocket | Not yet | Planned |
| Durable Objects | Not yet | Planned |

---

## How It Works

**The GC problem**: V8 uses a generational concurrent garbage collector (Orinoco). Even with concurrent collection, there are stop-the-world pauses. For Workers with short request lifetimes, GC runs during the request.

**Workex solution**: Each isolate gets a bump arena allocator. All allocations during a request go into the arena. When the request completes, `arena.reset()` frees everything in O(1) - a single pointer reset, no tracing, no reference counting.

**The JIT problem**: V8 speculates on types, generates optimized code with deoptimization paths, and occasionally deoptimizes. This creates p99 latency spikes.

**Workex solution**: TypeScript type annotations are known at compile time. `function add(a: number, b: number): number` compiles directly to `fadd` via Cranelift - no speculation, no deopt paths, flat latency.

---

## Test Suite

81 tests. 90% real, 10% mock (KV/D1/fetch - these depend on Cloudflare remote services).

```
cargo test
```

| Category | Tests | Real/Mock |
|---|---|---|
| TypeScript parser (oxc) | 7 | Real |
| HIR + type lowering | 4 | Real |
| Cranelift codegen + execution | 5 | Real |
| E2E TS-to-native pipeline | 6 | Real |
| Arena allocator | 14 | Real |
| Isolate + pool | 8 | Real |
| OS-level RSS measurement | 2 | Real |
| Boa JS engine | 5 | Real |
| Worker script execution | 5 | Real |
| Headers / Request / Response | 11 | Real |
| Benchmark validation | 3 | Real |
| KV / D1 / fetch | 8 | Mock |

---

## License

MIT
