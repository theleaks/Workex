/**
 * Worker Compatibility Test — runs the SAME Worker script on V8 and Miniflare.
 * Workex runs the same test via Rust (rss-bench or worker_execution tests).
 *
 * Usage: node benchmarks/scripts/worker-compat-test.mjs
 * Output: JSON with results for both runtimes
 */

import vm from "node:vm";
import { Miniflare } from "miniflare";
import { performance } from "node:perf_hooks";

// The Worker source — same as tests/workers/hello.ts but plain JS
const WORKER_SOURCE = `
export default {
  async fetch(request) {
    return new Response("Hello from Workex!", {
      headers: { "content-type": "text/plain" },
    });
  },
};`;

// JS version for vm.runInContext (no export default)
const V8_SOURCE = `
function Response(body, init) {
  this.body = body;
  this.status = (init && init.status) || 200;
  this.headers = (init && init.headers) || {};
}
var handler = {
  fetch: function(request) {
    return new Response("Hello from Workex!", {
      headers: { "content-type": "text/plain" },
    });
  },
};
`;

const ITERATIONS = 1000;
const WARMUP = 100;

// ── V8 Test ──
function testV8() {
  const ctx = vm.createContext({});
  vm.runInContext(V8_SOURCE, ctx);

  // Correctness
  const result = vm.runInContext(
    'var r = handler.fetch({url:"https://example.com/",method:"GET"}); ({body:r.body, status:r.status, ct:r.headers["content-type"]})',
    ctx
  );
  const correct =
    result.body === "Hello from Workex!" &&
    result.status === 200 &&
    result.ct === "text/plain";

  // Warmup
  for (let i = 0; i < WARMUP; i++) {
    vm.runInContext('handler.fetch({url:"https://x.com/",method:"GET"})', ctx);
  }

  // Measure
  const samples = [];
  for (let i = 0; i < ITERATIONS; i++) {
    const t = performance.now();
    vm.runInContext('handler.fetch({url:"https://x.com/",method:"GET"})', ctx);
    samples.push((performance.now() - t) * 1e6);
  }
  samples.sort((a, b) => a - b);

  return {
    runtime: "v8",
    correct,
    body: result.body,
    status: result.status,
    content_type: result.ct,
    latency: {
      p50_ns: Math.round(samples[Math.floor(samples.length * 0.5)]),
      p95_ns: Math.round(samples[Math.floor(samples.length * 0.95)]),
      p99_ns: Math.round(samples[Math.floor(samples.length * 0.99)]),
      mean_ns: Math.round(samples.reduce((a, b) => a + b, 0) / samples.length),
    },
    iterations: ITERATIONS,
  };
}

// ── Miniflare (Workers) Test ──
async function testWorkers() {
  const mf = new Miniflare({
    modules: true,
    script: WORKER_SOURCE,
  });

  // Correctness
  const res = await mf.dispatchFetch("https://example.com/");
  const body = await res.text();
  const status = res.status;
  const ct = res.headers.get("content-type");
  const correct =
    body === "Hello from Workex!" && status === 200 && ct === "text/plain";

  // Warmup
  for (let i = 0; i < WARMUP; i++) {
    const r = await mf.dispatchFetch("https://x.com/");
    await r.text();
  }

  // Measure
  const samples = [];
  for (let i = 0; i < ITERATIONS; i++) {
    const t = performance.now();
    const r = await mf.dispatchFetch("https://x.com/");
    await r.text();
    samples.push((performance.now() - t) * 1e6);
  }
  samples.sort((a, b) => a - b);

  await mf.dispose();

  return {
    runtime: "workers",
    correct,
    body,
    status,
    content_type: ct,
    latency: {
      p50_ns: Math.round(samples[Math.floor(samples.length * 0.5)]),
      p95_ns: Math.round(samples[Math.floor(samples.length * 0.95)]),
      p99_ns: Math.round(samples[Math.floor(samples.length * 0.99)]),
      mean_ns: Math.round(samples.reduce((a, b) => a + b, 0) / samples.length),
    },
    iterations: ITERATIONS,
  };
}

// ── Run both ──
async function main() {
  process.stderr.write("[V8] Testing hello worker...\n");
  const v8 = testV8();

  process.stderr.write("[Workers] Testing hello worker...\n");
  const workers = await testWorkers();

  console.log(JSON.stringify({ v8, workers }, null, 2));
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
