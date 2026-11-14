// A test page server. `/` shows who connected, `/set` sets the brr cookie, `/echo` shows it back.
// usage: node peer-server.js PORT
const http = require("http");

const html = (title) => `<!doctype html><title>${title}</title><body>${title}</body>`;
const cookieValue = (header) => /(?:^|;\s*)brr=([^;]*)/.exec(header || "")?.[1] ?? "none";

http
  .createServer((req, res) => {
    const peer = req.socket.remoteAddress.replace(/^::ffff:/, "");
    const headers = { "content-type": "text/html" };
    let title = `peer:${peer}`;

    if (req.url.startsWith("/set")) {
      headers["set-cookie"] = "brr=online; Path=/; Max-Age=3600";
      title = "cookie-set";
    } else if (req.url.startsWith("/echo")) {
      title = `cookie:${cookieValue(req.headers.cookie)}`;
    }
    res.writeHead(200, headers).end(html(title));
  })
  .listen(+process.argv[2], "127.0.0.1");
