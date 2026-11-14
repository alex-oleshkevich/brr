// Answers like a Cloudflare trace endpoint, for proxy.check_url.
// usage: node trace-server.js PORT
require("http")
  .createServer((_, res) => res.end("ip=203.0.113.7\nloc=NL\n"))
  .listen(+process.argv[2], "127.0.0.1");
