/**
 * Unified 3-Way Benchmark — V8 + Workers tests.
 * Called by Rust orchestrator. Runs identical tests for V8 and Workers (Miniflare).
 *
 * Usage: node --expose-gc benchmarks/scripts/unified-bench.mjs <runs> <type>
 *   type: "micro" | "rss" | "worker-compat"
 *   runs: number of iterations for averaging
 *
 * Output: JSON to stdout
 */

import vm from "node:vm";
import { Miniflare } from "miniflare";
import { performance } from "node:perf_hooks";

const RUNS = parseInt(process.argv[2] || "5", 10);
const TYPE = process.argv[3] || "all";

// ─── Stats helper ────────────────────────────────
function stats(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  const n = sorted.length;
  const sum = sorted.reduce((a, b) => a + b, 0);
  const mean = sum / n;
  const variance = sorted.reduce((s, v) => s + (v - mean) ** 2, 0) / n;
  return {
    runs: n,
    mean: mean,
    median: sorted[Math.floor(n / 2)],
    stddev: Math.sqrt(variance),
    min: sorted[0],
    max: sorted[n - 1],
    p95: sorted[Math.floor(n * 0.95)] || sorted[n - 1],
    p99: sorted[Math.floor(n * 0.99)] || sorted[n - 1],
  };
}

function measure(iterations, fn) {
  const samples = [];
  for (let i = 0; i < iterations; i++) {
    const t = performance.now();
    fn();
    samples.push((performance.now() - t) * 1e6); // ns
  }
  return stats(samples);
}

async function measureAsync(iterations, fn) {
  const samples = [];
  for (let i = 0; i < iterations; i++) {
    const t = performance.now();
    await fn();
    samples.push((performance.now() - t) * 1e6);
  }
  return stats(samples);
}

// ─── WORKER SOURCE ───────────────────────────────
const WORKER_JS = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const body = JSON.stringify({ status: "ok", path: url.pathname, ts: Date.now() });
    return new Response(body, { headers: { "content-type": "application/json" } });
  }
};`;

const V8_WORKER = `
function Response(body, init) {
  this.body = String(body);
  this.status = (init && init.status) || 200;
  this.headers = (init && init.headers) || {};
  this.__is_response = true;
}
function handleFetch(request) {
  var body = JSON.stringify({ status: "ok", path: request.url, ts: Date.now() });
  return new Response(body, { headers: { "content-type": "application/json" } });
}`;

// ═══════════════════════════════════════════════════
// MICRO BENCHMARKS
// ═══════════════════════════════════════════════════
function microV8() {
  // Cold start
  const coldStart = measure(RUNS * 100, () => {
    const ctx = vm.createContext({ Date, JSON });
    vm.runInContext(V8_WORKER, ctx);
    vm.runInContext('handleFetch({url:"https://x.com/a"})', ctx);
  });

  // Warm exec
  const ctx = vm.createContext({ Date, JSON });
  vm.runInContext(V8_WORKER, ctx);
  for (let i = 0; i < 10000; i++) vm.runInContext('handleFetch({url:"https://x.com/w"})', ctx);
  const warmExec = measure(RUNS * 1000, () => {
    vm.runInContext('handleFetch({url:"https://x.com/w"})', ctx);
  });

  return { cold_start_ns: coldStart, warm_exec_ns: warmExec };
}

async function microWorkers() {
  // Cold start
  const coldSamples = [];
  for (let i = 0; i < Math.min(RUNS, 10); i++) {
    const t = performance.now();
    const mf = new Miniflare({ modules: true, script: WORKER_JS });
    const r = await mf.dispatchFetch("https://x.com/cold");
    await r.text();
    coldSamples.push((performance.now() - t) * 1e6);
    await mf.dispose();
  }
  const coldStart = stats(coldSamples);

  // Warm exec
  const mf = new Miniflare({ modules: true, script: WORKER_JS });
  for (let i = 0; i < 100; i++) { const r = await mf.dispatchFetch("https://x.com/w"); await r.text(); }
  const warmExec = await measureAsync(Math.min(RUNS * 200, 2000), async () => {
    const r = await mf.dispatchFetch("https://x.com/warm");
    await r.text();
  });
  await mf.dispose();

  return { cold_start_ns: coldStart, warm_exec_ns: warmExec };
}

// ═══════════════════════════════════════════════════
// RSS BENCHMARK
// ═══════════════════════════════════════════════════
function rssV8(count) {
  const results = [];
  for (let run = 0; run < RUNS; run++) {
    if (global.gc) global.gc();
    const before = process.memoryUsage().rss;
    const contexts = [];
    for (let i = 0; i < count; i++) {
      const ctx = vm.createContext({ Date, JSON });
      vm.runInContext(V8_WORKER, ctx);
      vm.runInContext('handleFetch({url:"https://x.com/' + i + '"})', ctx);
      contexts.push(ctx);
    }
    if (global.gc) global.gc();
    const after = process.memoryUsage().rss;
    results.push({ delta: after - before, per_ctx: Math.round((after - before) / count) });
    // Force free
    contexts.length = 0;
    if (global.gc) global.gc();
  }
  return {
    count,
    runs: RUNS,
    rss_delta: stats(results.map(r => r.delta)),
    per_context: stats(results.map(r => r.per_ctx)),
  };
}

async function rssWorkers(count) {
  const actualCount = Math.min(count, 100);
  const results = [];
  for (let run = 0; run < Math.min(RUNS, 3); run++) {
    if (global.gc) global.gc();
    const before = process.memoryUsage().rss;
    const workers = [];
    for (let i = 0; i < actualCount; i++) {
      const mf = new Miniflare({ modules: true, script: WORKER_JS });
      const r = await mf.dispatchFetch("https://x.com/" + i);
      await r.text();
      workers.push(mf);
    }
    if (global.gc) global.gc();
    const after = process.memoryUsage().rss;
    const delta = after - before;
    results.push({ delta, per_worker: Math.round(delta / actualCount) });
    for (const w of workers) await w.dispose();
    if (global.gc) global.gc();
  }
  return {
    actual_count: actualCount,
    target_count: count,
    runs: results.length,
    rss_delta: stats(results.map(r => r.delta)),
    per_worker: stats(results.map(r => r.per_worker)),
    extrapolated_10k_mb: stats(results.map(r => r.per_worker * 10000 / 1024 / 1024)),
  };
}

// ═══════════════════════════════════════════════════
// WORKER COMPAT
// ═══════════════════════════════════════════════════
function compatV8() {
  const ctx = vm.createContext({ Date, JSON });
  vm.runInContext(V8_WORKER, ctx);
  const r = vm.runInContext('var r = handleFetch({url:"https://example.com/test",method:"GET"}); ({body:r.body,status:r.status,ct:r.headers["content-type"]})', ctx);
  return {
    correct: r.status === 200 && r.ct === "application/json",
    status: r.status,
    has_body: typeof r.body === "string" && r.body.length > 0,
  };
}

async function compatWorkers() {
  const mf = new Miniflare({ modules: true, script: WORKER_JS });
  const res = await mf.dispatchFetch("https://example.com/test");
  const body = await res.text();
  const ct = res.headers.get("content-type");
  await mf.dispose();
  return {
    correct: res.status === 200 && ct === "application/json",
    status: res.status,
    has_body: body.length > 0,
  };
}

// ═══════════════════════════════════════════════════
// MAIN
// ═══════════════════════════════════════════════════
async function main() {
  const result = { runs: RUNS, type: TYPE };

  if (TYPE === "all" || TYPE === "micro") {
    process.stderr.write("[V8] micro benchmarks...\n");
    result.v8_micro = microV8();
    process.stderr.write("[Workers] micro benchmarks...\n");
    result.workers_micro = await microWorkers();
  }

  if (TYPE === "all" || TYPE === "rss") {
    process.stderr.write("[V8] RSS 10K...\n");
    result.v8_rss = rssV8(10000);
    process.stderr.write("[Workers] RSS...\n");
    result.workers_rss = await rssWorkers(10000);
  }

  if (TYPE === "all" || TYPE === "compat") {
    process.stderr.write("[V8] compat...\n");
    result.v8_compat = compatV8();
    process.stderr.write("[Workers] compat...\n");
    result.workers_compat = await compatWorkers();
  }

  result.v8_version = process.versions.v8;
  result.node_version = process.version;

  console.log(JSON.stringify(result, null, 2));
}

main().catch(e => { console.error(e); process.exit(1); });
