/**
 * k6 load test — runs identical scenarios against a target server.
 *
 * Usage:
 *   k6 run -e TARGET=http://127.0.0.1:3001 benchmarks/scripts/k6-test.js
 *
 * Scenarios:
 *   1. health  — GET /health (minimal response)
 *   2. json    — GET /json   (JSON serialization)
 *   3. compute — GET /compute (CPU: fibonacci(30))
 *   4. hello   — GET /       (text response)
 */

import http from "k6/http";
import { check, sleep } from "k6";
import { Trend, Rate, Counter } from "k6/metrics";

const TARGET = __ENV.TARGET || "http://127.0.0.1:3001";

// Custom metrics
const healthLatency = new Trend("health_latency", true);
const jsonLatency = new Trend("json_latency", true);
const computeLatency = new Trend("compute_latency", true);
const helloLatency = new Trend("hello_latency", true);
const errorRate = new Rate("errors");

export const options = {
  scenarios: {
    // Ramp-up load test
    load_test: {
      executor: "ramping-vus",
      startVUs: 1,
      stages: [
        { duration: "5s", target: 10 },   // Ramp up
        { duration: "15s", target: 50 },   // Sustain
        { duration: "10s", target: 100 },  // Peak
        { duration: "5s", target: 0 },     // Ramp down
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<500", "p(99)<1000"],
    errors: ["rate<0.01"],
  },
};

export default function () {
  // 1. Health check
  const healthRes = http.get(`${TARGET}/health`);
  healthLatency.add(healthRes.timings.duration);
  check(healthRes, {
    "health: status 200": (r) => r.status === 200,
    "health: body ok": (r) => r.body === "ok",
  }) || errorRate.add(1);

  // 2. JSON endpoint
  const jsonRes = http.get(`${TARGET}/json`);
  jsonLatency.add(jsonRes.timings.duration);
  check(jsonRes, {
    "json: status 200": (r) => r.status === 200,
    "json: valid json": (r) => {
      try {
        const data = JSON.parse(r.body);
        return data.status === "ok";
      } catch {
        return false;
      }
    },
  }) || errorRate.add(1);

  // 3. Compute endpoint
  const computeRes = http.get(`${TARGET}/compute`);
  computeLatency.add(computeRes.timings.duration);
  check(computeRes, {
    "compute: status 200": (r) => r.status === 200,
    "compute: correct fib": (r) => {
      try {
        return JSON.parse(r.body).fib30 === 832040;
      } catch {
        return false;
      }
    },
  }) || errorRate.add(1);

  // 4. Hello endpoint
  const helloRes = http.get(`${TARGET}/`);
  helloLatency.add(helloRes.timings.duration);
  check(helloRes, {
    "hello: status 200": (r) => r.status === 200,
  }) || errorRate.add(1);

  sleep(0.01); // Small pause between iterations
}

export function handleSummary(data) {
  // Output JSON summary for programmatic comparison
  const summary = {
    target: TARGET,
    metrics: {
      health_p50: data.metrics.health_latency?.values["p(50)"],
      health_p95: data.metrics.health_latency?.values["p(95)"],
      health_p99: data.metrics.health_latency?.values["p(99)"],
      json_p50: data.metrics.json_latency?.values["p(50)"],
      json_p95: data.metrics.json_latency?.values["p(95)"],
      json_p99: data.metrics.json_latency?.values["p(99)"],
      compute_p50: data.metrics.compute_latency?.values["p(50)"],
      compute_p95: data.metrics.compute_latency?.values["p(95)"],
      compute_p99: data.metrics.compute_latency?.values["p(99)"],
      hello_p50: data.metrics.hello_latency?.values["p(50)"],
      hello_p95: data.metrics.hello_latency?.values["p(95)"],
      hello_p99: data.metrics.hello_latency?.values["p(99)"],
      rps: data.metrics.http_reqs?.values.rate,
      total_requests: data.metrics.http_reqs?.values.count,
      error_rate: data.metrics.errors?.values.rate || 0,
      http_duration_p50: data.metrics.http_req_duration?.values["p(50)"],
      http_duration_p95: data.metrics.http_req_duration?.values["p(95)"],
      http_duration_p99: data.metrics.http_req_duration?.values["p(99)"],
    },
  };

  const out = JSON.stringify(summary, null, 2) + "\n";

  // Write to file if OUTPUT env is set, otherwise stdout
  const result = { stdout: out };
  const outputFile = __ENV.OUTPUT;
  if (outputFile) {
    result[outputFile] = out;
  }
  return result;
}
