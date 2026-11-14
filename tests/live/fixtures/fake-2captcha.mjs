// A stand-in for the 2captcha JSON API. Usage: node fake-2captcha.mjs PORT   (no npm packages)
// createTask -> taskId n; getTaskResult is "processing" once, then ready (cost 0.00299, token
// tok-<n>); getBalance -> 3.5. Every request body is recorded at GET /_requests.
import http from 'node:http';

const PORT = Number(process.argv[2]);
const requests = [];
const tasks = new Map();
let nextId = 1;

function readJson(req) {
  return new Promise((resolve) => {
    let body = '';
    req.on('data', (chunk) => (body += chunk));
    req.on('end', () => {
      try {
        resolve(JSON.parse(body));
      } catch {
        resolve({});
      }
    });
  });
}

function reply(res, body) {
  res.writeHead(200, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body));
}

http
  .createServer(async (req, res) => {
    const path = new URL(req.url, 'http://x').pathname;
    if (req.method === 'GET' && path === '/_requests') return reply(res, requests);

    const body = await readJson(req);
    requests.push({ path, body });
    if (path === '/createTask') {
      const id = nextId++;
      tasks.set(id, { type: body.task?.type ?? '', polls: 0 });
      return reply(res, { errorId: 0, taskId: id });
    }
    if (path === '/getTaskResult') {
      const task = tasks.get(body.taskId);
      if (!task) return reply(res, { errorId: 1, errorCode: 'ERROR_NO_SUCH_CAPCHA_ID' });
      task.polls += 1;
      if (task.polls < 2) return reply(res, { errorId: 0, status: 'processing' });
      const solution = task.type === 'ImageToTextTask' ? { text: `txt-${body.taskId}` } : { gRecaptchaResponse: `tok-${body.taskId}`, token: `tok-${body.taskId}` };
      return reply(res, { errorId: 0, status: 'ready', cost: '0.00299', solution });
    }
    if (path === '/getBalance') return reply(res, { errorId: 0, balance: 3.5 });
    return reply(res, { errorId: 1, errorCode: 'ERROR_UNEXPECTED' });
  })
  .listen(PORT, '127.0.0.1');
