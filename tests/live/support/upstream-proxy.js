// An HTTP proxy that requires Basic auth and dials every target from 127.0.0.2, so a test server
// can tell traffic that came through it from a direct connection.
// usage: node upstream-proxy.js PORT USER PASS
const http = require("http");
const net = require("net");

const [port, user, pass] = process.argv.slice(2);
const expected = "Basic " + Buffer.from(`${user}:${pass}`).toString("base64");
const LOCAL_ADDRESS = "127.0.0.2";

const authorized = (req) => req.headers["proxy-authorization"] === expected;

const server = http.createServer((req, res) => {
  if (!authorized(req)) {
    res.writeHead(407, { "Proxy-Authenticate": 'Basic realm="brr"' }).end();
    return;
  }

  const target = new URL(req.url);
  const upstream = http.request(
    {
      host: target.hostname,
      port: target.port || 80,
      path: target.pathname + target.search,
      method: req.method,
      headers: req.headers,
      localAddress: LOCAL_ADDRESS,
    },
    (reply) => {
      res.writeHead(reply.statusCode, reply.headers);
      reply.pipe(res);
    },
  );
  upstream.on("error", () => res.writeHead(502).end());
  req.pipe(upstream);
});

server.on("connect", (req, client, head) => {
  if (!authorized(req)) {
    client.end('HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="brr"\r\n\r\n');
    return;
  }

  const [host, targetPort] = req.url.split(":");
  const upstream = net.connect({ host, port: +targetPort, localAddress: LOCAL_ADDRESS }, () => {
    client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
    upstream.write(head);
    upstream.pipe(client);
    client.pipe(upstream);
  });
  upstream.on("error", () => client.end("HTTP/1.1 502 Bad Gateway\r\n\r\n"));
  client.on("error", () => upstream.destroy());
});

server.listen(+port, "127.0.0.1");
