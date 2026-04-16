/**
 * V8 10K Real Worker RSS — each context compiles and runs an actual Worker function.
 * Not empty contexts, real work.
 *
 * Usage: node --expose-gc benchmarks/scripts/v8-rss-real.mjs
 */

import vm from "node:vm";

const WORKER_SOURCE = `
function Response(body, init) {
    this.__body = String(body);
    this.__status = (init && init.status) || 200;
    this.__headers = (init && init.headers) || {};
}

function handleFetch(request) {
    var url = request.url;
    var body = JSON.stringify({ status: "ok", path: url, ts: Date.now() });
    return new Response(body, {
        headers: { "content-type": "application/json" }
    });
}
`;

const COUNT = 10_000;

if (global.gc) global.gc();
const rssBefore = process.memoryUsage().rss;

const contexts = [];
for (let i = 0; i < COUNT; i++) {
    const ctx = vm.createContext({ Date, JSON });
    vm.runInContext(WORKER_SOURCE, ctx);
    // Actually call the function to force JIT compilation
    vm.runInContext('handleFetch({ url: "https://example.com/test" })', ctx);
    contexts.push(ctx);
}

if (global.gc) global.gc();
const rssAfter = process.memoryUsage().rss;
const delta = rssAfter - rssBefore;

console.log(JSON.stringify({
    runtime: "v8",
    test: "10k_real_worker_rss",
    count: COUNT,
    rss_before_bytes: rssBefore,
    rss_after_bytes: rssAfter,
    rss_delta_bytes: delta,
    rss_delta_mb: (delta / 1024 / 1024).toFixed(1),
    rss_per_context_bytes: Math.round(delta / COUNT),
    rss_per_context_kb: (delta / COUNT / 1024).toFixed(1),
    gc_available: typeof global.gc === "function",
}, null, 2));
