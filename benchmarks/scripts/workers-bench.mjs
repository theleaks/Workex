/**
 * Cloudflare Workers benchmark — uses Miniflare (workerd) locally.
 * Same test suite as V8 and Workex for fair 3-way comparison.
 *
 * Usage: node benchmarks/scripts/workers-bench.mjs [--fast|--standard|--heavy]
 */

import { Miniflare } from "miniflare";
import { measure, measureAsync, getConfig } from "./bench-common.mjs";

const C = getConfig(process.argv);

// Worker source scripts
const WORKER_ADD = `
export default {
  async fetch(req) {
    function add(a, b) { return a + b; }
    const result = add(3.0, 4.0);
    return new Response(String(result));
  }
};`;

const WORKER_JSON = `
export default {
  async fetch(req) {
    const payload = '{"user":"alice","count":42,"tags":["a","b","c"]}';
    const result = JSON.stringify(JSON.parse(payload));
    return new Response(result);
  }
};`;

const WORKER_FIB = `
export default {
  async fetch(req) {
    function fib(n) { return n <= 1 ? n : fib(n-1) + fib(n-2); }
    return new Response(String(fib(35)));
  }
};`;

const WORKER_HANDLER = `
export default {
  async fetch(req) {
    const url = new URL(req.url);
    const body = JSON.stringify({ status: "ok", path: url.pathname });
    return new Response(body, {
      headers: { "content-type": "application/json" },
    });
  }
};`;

const WORKER_GC = `
export default {
  async fetch(req) {
    const arr = [];
    for (let i = 0; i < 100; i++) {
      arr.push({ value: i, data: "Hello from Workers benchmark!" });
    }
    return new Response("ok");
  }
};`;

// ═══════════════════════════════════════════════════════════
// Helper: create a Miniflare instance
// ═══════════════════════════════════════════════════════════
async function createMF(script) {
  return new Miniflare({
    modules: true,
    script,
  });
}

// ═══════════════════════════════════════════════════════════
// 1. COLD START — create worker + first request
// ═══════════════════════════════════════════════════════════
async function benchColdStart() {
  const coldConfig = { warmup: 2, iterations: 20 };
  const stats = await measureAsync(coldConfig.warmup, coldConfig.iterations, async () => {
    const mf = new Miniflare({ modules: true, script: WORKER_ADD });
    const res = await mf.dispatchFetch("http://localhost/");
    await res.text();
    await mf.dispose();
  });
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 2. WARM EXEC: add — warm worker, measure request
// ═══════════════════════════════════════════════════════════
async function benchWarmAdd() {
  const mf = await createMF(WORKER_ADD);
  // Warmup
  for (let i = 0; i < 100; i++) {
    const r = await mf.dispatchFetch("http://localhost/");
    await r.text();
  }

  const stats = await measureAsync(
    Math.min(C.warmup, 500),
    Math.min(C.iterations, 5000),
    async () => {
      const r = await mf.dispatchFetch("http://localhost/");
      await r.text();
    }
  );

  await mf.dispose();
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 3. WARM EXEC: JSON roundtrip
// ═══════════════════════════════════════════════════════════
async function benchWarmJson() {
  const mf = await createMF(WORKER_JSON);
  for (let i = 0; i < 100; i++) {
    const r = await mf.dispatchFetch("http://localhost/");
    await r.text();
  }

  const stats = await measureAsync(
    Math.min(C.warmup, 500),
    Math.min(C.iterations, 5000),
    async () => {
      const r = await mf.dispatchFetch("http://localhost/");
      await r.text();
    }
  );

  await mf.dispose();
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 4. WARM EXEC: fibonacci(35)
// ═══════════════════════════════════════════════════════════
async function benchFib35() {
  const mf = await createMF(WORKER_FIB);
  // One warmup
  const wr = await mf.dispatchFetch("http://localhost/");
  await wr.text();

  const stats = await measureAsync(1, 5, async () => {
    const r = await mf.dispatchFetch("http://localhost/");
    await r.text();
  });

  await mf.dispose();
  return { stats, metadata: { fib_n: "35" } };
}

// ═══════════════════════════════════════════════════════════
// 5. REQUEST THROUGHPUT
// ═══════════════════════════════════════════════════════════
async function benchRequestThroughput() {
  const mf = await createMF(WORKER_HANDLER);
  for (let i = 0; i < 200; i++) {
    const r = await mf.dispatchFetch("http://localhost/api");
    await r.text();
  }

  const stats = await measureAsync(
    Math.min(C.warmup, 500),
    Math.min(C.iterations, 5000),
    async () => {
      const r = await mf.dispatchFetch("http://localhost/api");
      await r.text();
    }
  );

  const rps = stats.mean_ns > 0 ? Math.round(1e9 / stats.mean_ns) : 0;
  await mf.dispose();
  return { stats, metadata: { requests_per_sec: String(rps) } };
}

// ═══════════════════════════════════════════════════════════
// 6. MEMORY PER ISOLATE — create multiple workers, measure heap
// ═══════════════════════════════════════════════════════════
async function benchMemory() {
  if (global.gc) global.gc();
  const before = process.memoryUsage().heapUsed;
  const workers = [];
  const sampleSize = 20; // Miniflare instances are heavy
  for (let i = 0; i < sampleSize; i++) {
    workers.push(await createMF(WORKER_ADD));
  }
  const after = process.memoryUsage().heapUsed;
  const perWorker = Math.round((after - before) / sampleSize);

  // Timing
  const stats = await measureAsync(1, 10, async () => {
    const mf = new Miniflare({ modules: true, script: WORKER_ADD });
    const r = await mf.dispatchFetch("http://localhost/");
    await r.text();
    await mf.dispose();
  });

  for (const w of workers) await w.dispose();

  return {
    stats,
    metadata: {
      memory_bytes: String(perWorker),
      memory_kb: String(Math.round(perWorker / 1024)),
    },
  };
}

// ═══════════════════════════════════════════════════════════
// 7. CONCURRENCY — multiple workers simultaneously
// ═══════════════════════════════════════════════════════════
async function benchConcurrency() {
  // Workers/Miniflare is heavy — use 100 instead of 10K
  const count = 100;

  if (global.gc) global.gc();
  const before = process.memoryUsage().heapUsed;
  const startTime = performance.now();
  const workers = [];
  for (let i = 0; i < count; i++) {
    workers.push(await createMF(WORKER_ADD));
  }
  const creationMs = performance.now() - startTime;
  const after = process.memoryUsage().heapUsed;
  const totalMem = after - before;

  // Extrapolate to 10K
  const extrapolated10k = totalMem * 100;

  for (const w of workers) await w.dispose();

  const stats = {
    iterations: 1, warmup: 0,
    mean_ns: creationMs * 1e6, median_ns: Math.round(creationMs * 1e6),
    stddev_ns: 0, min_ns: Math.round(creationMs * 1e6),
    max_ns: Math.round(creationMs * 1e6),
    p50_ns: Math.round(creationMs * 1e6),
    p95_ns: Math.round(creationMs * 1e6),
    p99_ns: Math.round(creationMs * 1e6),
    total_ns: Math.round(creationMs * 1e6),
  };

  return {
    stats,
    metadata: {
      isolate_count: String(count),
      actual_count: String(count),
      total_memory_bytes: String(totalMem),
      total_memory_mb: (totalMem / 1024 / 1024).toFixed(1),
      avg_memory_kb: String(Math.round(totalMem / count / 1024)),
      extrapolated_10k_mb: (extrapolated10k / 1024 / 1024).toFixed(1),
    },
  };
}

// ═══════════════════════════════════════════════════════════
// 8. GC PRESSURE — alloc-heavy worker
// ═══════════════════════════════════════════════════════════
async function benchGC() {
  const mf = await createMF(WORKER_GC);
  for (let i = 0; i < 50; i++) {
    const r = await mf.dispatchFetch("http://localhost/");
    await r.text();
  }

  const stats = await measureAsync(
    Math.min(C.warmup, 200),
    Math.min(C.iterations, 1000),
    async () => {
      const r = await mf.dispatchFetch("http://localhost/");
      await r.text();
    }
  );

  await mf.dispose();
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// RUN ALL
// ═══════════════════════════════════════════════════════════
async function main() {
  const benchmarks = {};

  process.stderr.write("  [1/8] Cold start...\n");
  benchmarks.cold_start = await benchColdStart();

  process.stderr.write("  [2/8] Warm exec add...\n");
  benchmarks.warm_exec_add = await benchWarmAdd();

  process.stderr.write("  [3/8] Warm exec JSON...\n");
  benchmarks.warm_exec_json = await benchWarmJson();

  process.stderr.write("  [4/8] Fibonacci(35)...\n");
  benchmarks.warm_exec_fib35 = await benchFib35();

  process.stderr.write("  [5/8] Request throughput...\n");
  benchmarks.request_throughput = await benchRequestThroughput();

  process.stderr.write("  [6/8] Memory per isolate...\n");
  benchmarks.memory_per_isolate = await benchMemory();

  process.stderr.write("  [7/8] Concurrency...\n");
  benchmarks.concurrency_10k = await benchConcurrency();

  process.stderr.write("  [8/8] GC pressure...\n");
  benchmarks.gc_pressure = await benchGC();

  const results = {
    runtime: "workers",
    miniflare_note: "Using Miniflare (workerd) locally",
    benchmarks,
  };

  console.log(JSON.stringify(results));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
