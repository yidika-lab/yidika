const server = Bun.serve({
  port: 8080,
  fetch(req) {
    return new Response("Hello from Bun!");
  },
});
