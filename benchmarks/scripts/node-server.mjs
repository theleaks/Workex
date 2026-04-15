/**
 * Node.js (V8) HTTP server — same endpoints as Workex for k6 benchmarking.
 * Usage: node benchmarks/scripts/node-server.mjs [port]  (default: 3002)
 */

import http from "node:http";

const port = parseInt(process.argv[2] || "3002", 10);

function fib(n) {
  if (n <= 1) return n;
  return fib(n - 1) + fib(n - 2);
}

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://localhost:${port}`);
  const path = url.pathname;

  let body;
  let contentType = "text/plain";

  switch (path) {
    case "/health":
      body = "ok";
      break;
    case "/json":
      contentType = "application/json";
      body = JSON.stringify({ status: "ok", path, runtime: "node-v8" });
      break;
    case "/compute":
      contentType = "application/json";
      body = JSON.stringify({ fib30: fib(30) });
      break;
    default:
      body = "Hello from Node.js!";
      break;
  }

  res.writeHead(200, {
    "content-type": contentType,
    server: `node/${process.version}`,
  });
  res.end(body);
});

server.listen(port, "127.0.0.1", () => {
  process.stderr.write(`Node.js server listening on http://127.0.0.1:${port}\n`);
});
