/**
 * V8 10K Context RSS Benchmark.
 * Measures real OS-level RSS before/after creating 10K vm contexts.
 *
 * Usage: node --expose-gc benchmarks/scripts/v8-rss-bench.mjs
 * Output: JSON to stdout
 */

import vm from "node:vm";

// Force GC before measurement
if (global.gc) global.gc();

const rssBefore = process.memoryUsage().rss;
const heapBefore = process.memoryUsage().heapUsed;

const COUNT = 10_000;
const contexts = [];

for (let i = 0; i < COUNT; i++) {
  const ctx = vm.createContext({});
  // Compile a function in each context (like a Worker with a fetch handler)
  vm.runInContext("function fetch(req) { return 'Hello from V8 context ' + " + i + "; }", ctx);
  contexts.push(ctx);
}

// Force GC to get accurate numbers (GC'd garbage doesn't count)
if (global.gc) global.gc();

const rssAfter = process.memoryUsage().rss;
const heapAfter = process.memoryUsage().heapUsed;

const rssDelta = rssAfter - rssBefore;
const heapDelta = heapAfter - heapBefore;

const result = {
  runtime: "v8",
  node_version: process.version,
  count: COUNT,
  rss_before_bytes: rssBefore,
  rss_after_bytes: rssAfter,
  rss_delta_bytes: rssDelta,
  rss_delta_mb: (rssDelta / 1024 / 1024).toFixed(1),
  rss_per_context_bytes: Math.round(rssDelta / COUNT),
  rss_per_context_kb: (rssDelta / COUNT / 1024).toFixed(1),
  heap_delta_bytes: heapDelta,
  heap_delta_mb: (heapDelta / 1024 / 1024).toFixed(1),
  heap_per_context_bytes: Math.round(heapDelta / COUNT),
  gc_available: typeof global.gc === "function",
};

console.log(JSON.stringify(result, null, 2));
