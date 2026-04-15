/**
 * Shared measurement utilities for V8 and Workers benchmarks.
 * Both scripts import this to ensure identical measurement methodology.
 */

import { performance } from "node:perf_hooks";

/**
 * Measure a function with warmup, then collect samples.
 * Returns stats in the same schema as Rust's Stats struct.
 */
export function measure(warmup, iterations, fn) {
  // Warmup
  for (let i = 0; i < warmup; i++) fn();

  // Collect samples (nanoseconds)
  const samples = new Float64Array(iterations);
  const wallStart = performance.now();
  for (let i = 0; i < iterations; i++) {
    const t = performance.now();
    fn();
    samples[i] = (performance.now() - t) * 1e6;
  }
  const totalNs = (performance.now() - wallStart) * 1e6;

  return computeStats(samples, warmup, iterations, totalNs);
}

/**
 * Async version of measure for Workers fetch() calls.
 */
export async function measureAsync(warmup, iterations, fn) {
  for (let i = 0; i < warmup; i++) await fn();

  const samples = new Float64Array(iterations);
  const wallStart = performance.now();
  for (let i = 0; i < iterations; i++) {
    const t = performance.now();
    await fn();
    samples[i] = (performance.now() - t) * 1e6;
  }
  const totalNs = (performance.now() - wallStart) * 1e6;

  return computeStats(samples, warmup, iterations, totalNs);
}

function computeStats(samples, warmup, iterations, totalNs) {
  const sorted = Array.from(samples).sort((a, b) => a - b);
  const n = sorted.length;
  const sum = sorted.reduce((a, b) => a + b, 0);
  const mean = sum / n;
  const variance = sorted.reduce((a, s) => a + (s - mean) ** 2, 0) / n;

  return {
    iterations,
    warmup,
    mean_ns: mean,
    median_ns: Math.round(sorted[Math.floor(n * 0.5)]),
    stddev_ns: Math.sqrt(variance),
    min_ns: Math.round(sorted[0]),
    max_ns: Math.round(sorted[n - 1]),
    p50_ns: Math.round(sorted[Math.floor(n * 0.5)]),
    p95_ns: Math.round(sorted[Math.floor(n * 0.95)]),
    p99_ns: Math.round(sorted[Math.floor(n * 0.99)]),
    total_ns: Math.round(totalNs),
  };
}

/**
 * Parse CLI mode flag.
 */
export function getConfig(argv) {
  const mode = argv[2] || "--standard";
  return (
    {
      "--fast": { warmup: 100, iterations: 1000 },
      "--standard": { warmup: 1000, iterations: 10000 },
      "--heavy": { warmup: 5000, iterations: 50000 },
    }[mode] || { warmup: 1000, iterations: 10000 }
  );
}

/**
 * Standard benchmark names — every runtime MUST implement all of these.
 */
export const BENCH_NAMES = [
  "cold_start",          // Create context + first execution
  "warm_exec_add",       // add(a,b) in warm context
  "warm_exec_json",      // JSON.parse + JSON.stringify
  "warm_exec_fib35",     // Recursive fibonacci(35)
  "request_throughput",  // Full request→response cycle
  "memory_per_isolate",  // Heap bytes per context
  "concurrency_10k",     // Create 10K contexts, measure RAM
  "gc_pressure",         // Alloc + GC/cleanup overhead
];
