// Cloudflare Worker — same endpoints as Workex and Node.js for k6 benchmarking.

function fib(n: number): number {
  if (n <= 1) return n;
  return fib(n - 1) + fib(n - 2);
}

export default {
  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const path = url.pathname;

    switch (path) {
      case "/health":
        return new Response("ok");

      case "/json":
        return new Response(
          JSON.stringify({ status: "ok", path, runtime: "cloudflare-workers" }),
          { headers: { "content-type": "application/json" } }
        );

      case "/compute":
        return new Response(
          JSON.stringify({ fib30: fib(30) }),
          { headers: { "content-type": "application/json" } }
        );

      default:
        return new Response("Hello from Cloudflare Workers!");
    }
  },
};
