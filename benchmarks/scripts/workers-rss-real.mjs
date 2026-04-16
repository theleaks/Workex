/**
 * Workers (Miniflare/workerd) RSS Benchmark.
 * Creates N workers, each runs an actual fetch handler. Measures real RSS.
 *
 * Usage: node --expose-gc benchmarks/scripts/workers-rss-real.mjs [count]
 * Default count: 100 (Miniflare is heavy — extrapolates to 10K)
 */

import { Miniflare } from "miniflare";

const COUNT = parseInt(process.argv[2] || "100", 10);

const WORKER_SOURCE = `
export default {
  async fetch(request) {
    const url = new URL(request.url);
    const body = JSON.stringify({ status: "ok", path: url.pathname, ts: Date.now() });
    return new Response(body, {
      headers: { "content-type": "application/json" }
    });
  }
};
`;

async function main() {
  if (global.gc) global.gc();
  const rssBefore = process.memoryUsage().rss;

  const workers = [];
  for (let i = 0; i < COUNT; i++) {
    const mf = new Miniflare({ modules: true, script: WORKER_SOURCE });
    // Actually execute the Worker once
    const res = await mf.dispatchFetch(`https://example.com/${i}`);
    const body = await res.text();
    if (i === 0) {
      const parsed = JSON.parse(body);
      process.stderr.write(`[Workers] First Worker verified: status=${res.status}, body.status=${parsed.status}\n`);
    }
    workers.push(mf);

    if ((i + 1) % 25 === 0) {
      process.stderr.write(`  ${i + 1}/${COUNT} Workers created\n`);
    }
  }

  if (global.gc) global.gc();
  const rssAfter = process.memoryUsage().rss;
  const delta = rssAfter - rssBefore;
  const perWorker = Math.round(delta / COUNT);

  // Extrapolate to 10K
  const extrapolated10k = delta * (10000 / COUNT);

  // Cleanup
  for (const mf of workers) await mf.dispose();

  const result = {
    runtime: "workers",
    test: "real_worker_rss",
    actual_count: COUNT,
    extrapolated_to: 10000,
    rss_before_bytes: rssBefore,
    rss_after_bytes: rssAfter,
    rss_delta_bytes: delta,
    rss_delta_mb: (delta / 1024 / 1024).toFixed(1),
    rss_per_worker_bytes: perWorker,
    rss_per_worker_kb: (perWorker / 1024).toFixed(1),
    extrapolated_10k_mb: (extrapolated10k / 1024 / 1024).toFixed(1),
    gc_available: typeof global.gc === "function",
  };

  console.log(JSON.stringify(result, null, 2));
}

main().catch(e => { console.error(e); process.exit(1); });
