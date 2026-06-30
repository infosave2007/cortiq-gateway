// Fast, dependency-free mock upstream for gateway benchmarks.
// Responds instantly with a fixed OpenAI-compatible chat completion so the
// measured latency reflects GATEWAY overhead, not model time.
//
//   node bench/mock.js [port]   # default 8100
const http = require("http");
const PORT = parseInt(process.argv[2] || "8100", 10);

const COMPLETION = JSON.stringify({
  id: "chatcmpl-bench",
  object: "chat.completion",
  model: "bench-model",
  choices: [
    { index: 0, message: { role: "assistant", content: "ok" }, finish_reason: "stop" },
  ],
  usage: { prompt_tokens: 8, completion_tokens: 1, total_tokens: 9 },
});

const ROUTE = JSON.stringify({
  request_id: "bench",
  decision: { task_label: "general", complexity: { score: 0.2, tier: "low" } },
});

const server = http.createServer((req, res) => {
  if (req.method === "GET") {
    res.writeHead(200, { "content-type": "text/plain" });
    res.end("ok");
    return;
  }
  // drain the request body, then reply
  req.on("data", () => {});
  req.on("end", () => {
    const body = req.url.includes("/route") ? ROUTE : COMPLETION;
    res.writeHead(200, { "content-type": "application/json" });
    res.end(body);
  });
});

server.keepAliveTimeout = 60000;
server.listen(PORT, "127.0.0.1", () => console.log(`mock upstream on :${PORT}`));
