export default {
  async fetch(request: Request): Promise<Response> {
    return new Response("Hello from Workex!", {
      headers: { "content-type": "text/plain" },
    });
  },
};
