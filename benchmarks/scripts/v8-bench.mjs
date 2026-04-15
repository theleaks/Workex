/**
 * V8 (Node.js) benchmark — standard test suite.
 * Uses vm.createContext as the V8 isolate equivalent.
 *
 * Usage: node --expose-gc benchmarks/scripts/v8-bench.mjs [--fast|--standard|--heavy]
 */

import vm from "node:vm";
import { measure, getConfig } from "./bench-common.mjs";

const C = getConfig(process.argv);

// ═══════════════════════════════════════════════════════════
// 1. COLD START — create context + compile + first execution
// ═══════════════════════════════════════════════════════════
function benchColdStart() {
  const stats = measure(C.warmup, C.iterations, () => {
    const ctx = vm.createContext({});
    vm.runInContext("function add(a,b){return a+b;} add(3,4);", ctx);
  });
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 2. WARM EXEC: add(a,b) — hot function in pre-warmed context
// ═══════════════════════════════════════════════════════════
function benchWarmAdd() {
  const ctx = vm.createContext({});
  vm.runInContext("function add(a,b){return a+b;}", ctx);
  // TurboFan warmup
  for (let i = 0; i < 100_000; i++) vm.runInContext("add(3.0,4.0)", ctx);

  const stats = measure(C.warmup, C.iterations, () => {
    vm.runInContext("add(3.0,4.0)", ctx);
  });
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 3. WARM EXEC: JSON parse + stringify
// ═══════════════════════════════════════════════════════════
function benchWarmJson() {
  const ctx = vm.createContext({});
  vm.runInContext(
    `var payload = '{"user":"alice","count":42,"tags":["a","b","c"]}';
     function jsonRoundtrip() { return JSON.stringify(JSON.parse(payload)); }`,
    ctx
  );
  for (let i = 0; i < 10_000; i++) vm.runInContext("jsonRoundtrip()", ctx);

  const stats = measure(C.warmup, C.iterations, () => {
    vm.runInContext("jsonRoundtrip()", ctx);
  });
  return { stats, metadata: {} };
}

// ═══════════════════════════════════════════════════════════
// 4. WARM EXEC: fibonacci(35) — CPU-bound
// ═══════════════════════════════════════════════════════════
function benchFib35() {
  const ctx = vm.createContext({});
  vm.runInContext(
    "function fib(n){return n<=1?n:fib(n-1)+fib(n-2);}",
    ctx
  );
  // Warmup JIT
  vm.runInContext("fib(30)", ctx);

  const fibConfig = { warmup: 3, iterations: 10 };
  const stats = measure(fibConfig.warmup, fibConfig.iterations, () => {
    vm.runInContext("fib(35)", ctx);
  });
  return { stats, metadata: { fib_n: "35" } };
}

// ═══════════════════════════════════════════════════════════
// 5. REQUEST THROUGHPUT — simulate request→response in vm
// ═══════════════════════════════════════════════════════════
function benchRequestThroughput() {
  const ctx = vm.createContext({});
  vm.runInContext(
    `function handleRequest(url) {
       var u = url;
       var body = JSON.stringify({status:"ok",url:u});
       return {status:200, body:body};
     }`,
    ctx
  );
  for (let i = 0; i < 10_000; i++) {
    vm.runInContext('handleRequest("https://example.com/api")', ctx);
  }

  const stats = measure(C.warmup, C.iterations, () => {
    vm.runInContext('handleRequest("https://example.com/api")', ctx);
  });
  const rps = stats.mean_ns > 0 ? Math.round(1e9 / stats.mean_ns) : 0;
  return { stats, metadata: { requests_per_sec: String(rps) } };
}

// ═══════════════════════════════════════════════════════════
// 6. MEMORY PER ISOLATE
// ═══════════════════════════════════════════════════════════
function benchMemory() {
  if (global.gc) global.gc();
  const before = process.memoryUsage().heapUsed;
  const contexts = [];
  const sampleSize = 500;
  for (let i = 0; i < sampleSize; i++) {
    const ctx = vm.createContext({});
    vm.runInContext("function add(a,b){return a+b;}", ctx);
    contexts.push(ctx);
  }
  const after = process.memoryUsage().heapUsed;
  const perCtx = Math.round((after - before) / sampleSize);

  // Timing stats for creation
  const stats = measure(Math.min(C.warmup, 100), Math.min(C.iterations, 1000), () => {
    const ctx = vm.createContext({});
    vm.runInContext("1+1", ctx);
  });

  return {
    stats,
    metadata: {
      memory_bytes: String(perCtx),
      memory_kb: String(Math.round(perCtx / 1024)),
    },
  };
}

// ═══════════════════════════════════════════════════════════
// 7. CONCURRENCY 10K — create 10K contexts, measure total RAM
// ═══════════════════════════════════════════════════════════
function benchConcurrency() {
  const count = 10_000;

  const stats = measure(1, 3, () => {
    const ctxs = [];
    for (let i = 0; i < count; i++) {
      ctxs.push(vm.createContext({}));
    }
  });

  if (global.gc) global.gc();
  const before = process.memoryUsage().heapUsed;
  const ctxs = [];
  for (let i = 0; i < count; i++) {
    ctxs.push(vm.createContext({}));
  }
  const after = process.memoryUsage().heapUsed;
  const totalMem = after - before;

  return {
    stats,
    metadata: {
      isolate_count: String(count),
      total_memory_bytes: String(totalMem),
      total_memory_mb: (totalMem / 1024 / 1024).toFixed(1),
      avg_memory_kb: String(Math.round(totalMem / count / 1024)),
    },
  };
}

// ═══════════════════════════════════════════════════════════
// 8. GC PRESSURE — alloc objects then force GC
// ═══════════════════════════════════════════════════════════
function benchGC() {
  const hasGC = typeof global.gc === "function";

  const stats = measure(Math.min(C.warmup, 100), Math.min(C.iterations, 1000), () => {
    const arr = [];
    for (let i = 0; i < 100; i++) {
      arr.push({ value: i, data: "Hello from V8 benchmark!" });
    }
    if (hasGC) global.gc();
  });

  return {
    stats,
    metadata: { gc_available: String(hasGC) },
  };
}

// ═══════════════════════════════════════════════════════════
// RUN ALL
// ═══════════════════════════════════════════════════════════
const results = {
  runtime: "v8",
  node_version: process.version,
  v8_version: process.versions.v8,
  benchmarks: {
    cold_start: benchColdStart(),
    warm_exec_add: benchWarmAdd(),
    warm_exec_json: benchWarmJson(),
    warm_exec_fib35: benchFib35(),
    request_throughput: benchRequestThroughput(),
    memory_per_isolate: benchMemory(),
    concurrency_10k: benchConcurrency(),
    gc_pressure: benchGC(),
  },
};

console.log(JSON.stringify(results));
