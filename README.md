# Workex

**Agent-native JavaScript runtime. Drop-in Cloudflare Workers replacement.**

Built in Rust. Continuation Runtime. QuickJS. Cranelift AOT. Arena allocator. Zero GC.

```
10,000,000 suspended agents = 2.99 GB  (320 bytes each)
V8 would need 1,745 GB (1.7 TB)        (183 KB each)
585x less memory.
```

---

## The Problem

Cloudflare CEO Matthew Prince, April 14 2026:

> "24 million simultaneous sessions. We're not a little short on compute. **We're orders of magnitude away.**"

V8: 24M × 183KB = **4.1 TB**. Workex: 24M × 320B = **~22 GB**. One server.

---

## Benchmarks

Real measurements. Same machine, same conditions, 5 runs averaged. Three runtimes side-by-side.

| Component | Config |
|---|---|
| **Workex** | Rust release, rquickjs 0.9, SharedRuntime, Continuation VM, ContinuationSlab, Arc\<str\>, Cranelift native inject |
| **V8** | Node.js v24.12.0, `--expose-gc`, `vm.createContext()` |
| **CF Workers** | Miniflare v4.20 (real workerd). Results vary by version. |
| **Memory** | OS-level RSS: `GetProcessMemoryInfo` (Win) / `/proc/self/status` (Linux) |

### 1. 10M Suspended Agents

| Metric | Workex | V8 | Factor |
|---|---|---|---|
| **10M RSS** | **2.99 GB** | 1,745 GB* | **585x** |
| **Per agent** | **320 bytes** | 183 KB* | **585x** |
| **Rate** | 1.12M/sec | — | — |

```
cargo run -p workex-bench --release --bin ten-million-bench
```

### 2. 1M Suspended Agents

| Metric | Workex | V8 | Factor |
|---|---|---|---|
| **1M** | **182 MB** | 174.5 GB* | **981x** |
| **Per agent** | **191 bytes** | 183 KB* | **981x** |

```
cargo run -p workex-bench --release --bin continuation-bench
```

> At 1M: 191 bytes (981x). At 10M: 320 bytes (585x). Arc\<str\> shares constant strings zero-copy.

### 3. Active Contexts (10K, 3-way)

| Metric | Workex | V8 | CF Workers | vs V8 | vs Workers |
|---|---|---|---|---|---|
| **Per context** | **59 KB** | 183 KB | 691 KB | **3.1x** | **11.8x** |

```
cargo run -p workex-bench --release --bin shared-bench
```

### 4. Execution (5 runs avg)

| Metric | Workex | V8 | CF Workers | vs V8 | vs Workers |
|---|---|---|---|---|---|
| **Cold start** | 298 us | 260 us | 104 ms | 0.9x | **349x** |
| **Warm exec** | **5.4 us** | 2.7 us | 1.07 ms | 0.5x | **200x** |
| **Compat** | PASS | PASS | PASS | | |

```
cargo run -p workex-bench --release --bin unified-bench -- --runs 5
```

V8 faster on warm exec (JIT vs interpreter). Workex wins on density (585x less memory). Typed functions run via Cranelift native (~1ns/call).

### 5. Worker Compat — hello.ts (warm pool)

| Metric | Workex | V8 | CF Workers | vs Workers |
|---|---|---|---|---|
| **p50** | **5.4 us** | 1.9 us | 1.01 ms | **187x** |
| **p99** | **5.9 us** | 8.7 us | 2.65 ms | **449x** |

```
cargo run -p workex-bench --release --bin worker-test
```

### 6. 10K Real Worker RSS (3-way)

| Metric | Workex | V8 | CF Workers | vs V8 | vs Workers |
|---|---|---|---|---|---|
| **Per Worker** | **114 KB** | 183 KB | 462 KB | **1.6x** | **4.1x** |

```
cargo run -p workex-bench --release --bin rss-real-bench
```

### 7. 24M Projection

| | Workex | V8 |
|---|---|---|
| **Suspended (99%)** | 23.76M × 320B = **7.6 GB** | 23.76M × 183KB = **4.1 TB** |
| **Active (1%)** | 240K × 59KB = **14 GB** | 240K × 183KB = **43 GB** |
| **Total** | **~22 GB** | **~4.1 TB** |

> Uses 320 bytes/agent (10M measurement) — conservative production estimate.

---

## Quick Start

```bash
cargo build --release      # Build
cargo test                  # 166 tests
bash demo.sh                # 5-minute demo
workex dev                  # Dev server (reads wrangler.toml)
workex dev --workerd-compat # workerd protocol compatible
```

---

## Architecture

```
Worker (.ts/.js)
  │
  ├─► CPS Transformer → Bytecode → Workex VM
  │     ├── SUSPEND → 320 bytes/agent (Arc<str>, ContinuationSlab)
  │     ├── Agent Scheduler (10M+, parallel Promise.all)
  │     └── Hibernation (survives restart)
  │
  ├─► QuickJS SharedRuntime (59KB/ctx, pre-compiled dispatch)
  │     └── Cranelift native functions injected (typed → ~1ns/call)
  │
  └─► Cranelift JIT (function add(a:number,b:number):number → native fadd)

  Arena (O(1) reset) → Hyper (tokio async)
```

| Path | Memory |
|---|---|
| Continuation VM (async) | **320 bytes** suspended |
| QuickJS SharedRuntime | 59 KB active |
| Cranelift native | ~1ns/call |

---

## Workers API — Zero Mocks

| API | Status |
|---|---|
| `export default { fetch }` | Real |
| Response / Request / Headers | Real |
| fetch() outbound | Real (reqwest) |
| Promise / async await / Promise.all | Real |
| try/catch across await | Real |
| KV Namespace | Real (sled) |
| D1 Database | Real (rusqlite) |
| ReadableStream | Real |
| WebSocketPair | Real |
| Agent hibernation | Real |
| CPU limits | Real |
| workerd protocol | Real |
| wrangler.toml | Real |
| Cranelift native typed functions | Real |

---

## Bug Fixes

| Bug | Impact | Fix |
|---|---|---|
| fetch() called twice | Silent data corruption | Single dispatch wrapper |
| String clone per agent | 481→320 bytes at 10M | Arc\<str\> zero-copy |
| 300-char eval per request | 13.5us warm exec | Pre-compiled dispatch (5.4us) |
| worker_test cold start | 40us fake "warm" | Pool-based measurement (5.4us) |
| No Cranelift in hot path | typed fns interpreted | Native inject into QuickJS |
| Extra eval for response | overhead per request | Direct return path for sync |

---

## Progress

| Metric | Start | Now |
|---|---|---|
| **Warm exec** | 17.0 us | **5.4 us** |
| **Worker p50** | 113 us | **5.4 us** |
| **10M/agent** | — | **320 bytes** |
| **10M factor** | — | **585x vs V8** |
| **1M factor** | — | **981x vs V8** |
| **Tests** | 81 | **166** |
| **Mocks** | 8 | **0** |

---

## Test Suite — 166 tests, 0 failures, 0 mocks

```
cargo test
```

---

## Benchmark Commands

```bash
cargo run -p workex-bench --release --bin ten-million-bench     # 10M
cargo run -p workex-bench --release --bin continuation-bench    # 1M
cargo run -p workex-bench --release --bin unified-bench -- --runs 5  # 3-way
cargo run -p workex-bench --release --bin shared-bench          # 10K SharedRuntime
cargo run -p workex-bench --release --bin rss-real-bench        # 10K real RSS
cargo run -p workex-bench --release --bin worker-test           # hello.ts compat
bash benchmarks/scripts/run-k6.sh                               # k6 HTTP load
bash demo.sh                                                    # 5-min demo
```

---

## License

MIT
