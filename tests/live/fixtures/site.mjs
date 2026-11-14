// Local site for the M6 live scripts: a three-step login with TOTP, captcha widgets, ad and
// cookie-banner pages. Usage: node site.mjs PORT   (no npm packages)
import { createHmac } from 'node:crypto';
import http from 'node:http';

const PORT = Number(process.argv[2]);
const PASSWORD = process.env.SITE_PASSWORD ?? 'S3cret-Live-9f2';
const TOTP_SECRET = process.env.SITE_TOTP_SECRET ?? 'JBSWY3DPEHPK3PXP';
const STEP_S = 30;
const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
// 1x1 PNG, enough for the image captcha selector and screenshot.
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

const hits = [];

function base32Decode(text) {
  let bits = '';
  for (const ch of text.replace(/=+$/, '')) bits += BASE32.indexOf(ch).toString(2).padStart(5, '0');
  const bytes = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) bytes.push(parseInt(bits.slice(i, i + 8), 2));
  return Buffer.from(bytes);
}

// RFC 6238, SHA-1, 6 digits.
function totp(window) {
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(window));
  const mac = createHmac('sha1', base32Decode(TOTP_SECRET)).update(counter).digest();
  const offset = mac[mac.length - 1] & 0xf;
  const code = (mac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000;
  return String(code).padStart(6, '0');
}

// The previous window too, like real sites: a code typed at the edge of a window still counts.
function codeAccepted(code) {
  const window = Math.floor(Date.now() / 1000 / STEP_S);
  return code === totp(window) || code === totp(window - 1);
}

const page = (body, head = '') => `<!doctype html><html><head><meta charset=utf-8>${head}</head><body>${body}</body></html>`;
const hidden = (user) => `<input type=hidden name=user value="${user}">`;
const userForm = (action) =>
  `<form method=post action=${action}><input id=u name=username autocomplete=username><button type=submit>Next</button></form>`;
const passwordForm = (action, user) =>
  `<form method=post action=${action}>${hidden(user)}<input id=p type=password name=password autocomplete=current-password><button type=submit>Sign in</button></form>`;
const otpForm = (user) =>
  `<form method=post action=/otp>${hidden(user)}<input id=o name=code autocomplete=one-time-code><button type=submit>Verify</button></form>`;

const RECAPTCHA = `<div class="g-recaptcha" data-sitekey="live-sitekey"></div>
<textarea name="g-recaptcha-response"></textarea>
<script>window.___grecaptcha_cfg = {clients: {0: {w: {callback: (t) => { document.title = 'solved:' + t; }}}}};</script>`;
const IMAGE_CAPTCHA = `<img id=captcha-img alt="captcha" src="data:image/png;base64,${PNG}" width=120 height=40>
<input id=captcha-answer name=captcha placeholder="type the code">`;
// evaluate is refused on a page holding a filled secret, so the page reports the masking of the
// code field itself, at submit time; the form then hides to count as logged in.
const STICKY = `<form onsubmit="event.preventDefault(); this.style.display = 'none'; document.title = 'ts:' + getComputedStyle(document.getElementById('o')).webkitTextSecurity"><input id=u name=username>
<input id=o name=code autocomplete=one-time-code><button type=submit>Go</button></form>`;
const ADS = '<p id=content>ads</p><script src=/ad.js></script><script src=/ok.js></script>';
const COOKIES = `<div id=cookie-notice>
<button id=cookie-notice-accept class=cookie-notice__accept--custom>Accept</button>
<button id=cookie-notice-reject onclick="document.cookie='consent=rejected; path=/'; document.getElementById('cookie-notice').remove()">Reject</button></div><p>content</p>`;
const EASYLIST = '/ad.js\n';

function readForm(req) {
  return new Promise((resolve) => {
    let body = '';
    req.on('data', (chunk) => (body += chunk));
    req.on('end', () => resolve(Object.fromEntries(new URLSearchParams(body))));
  });
}

function send(res, status, body, type = 'text/html', headers = {}) {
  res.writeHead(status, { 'content-type': type, ...headers });
  res.end(body);
}

const sessionOf = (req) => /(?:^|; )session=([^;]*)/.exec(req.headers.cookie ?? '')?.[1];

async function handle(req, res) {
  const url = new URL(req.url, 'http://x');
  const path = url.pathname;
  if (req.method === 'POST') {
    const form = await readForm(req);
    if (path === '/user') return send(res, 200, page(passwordForm('/password', form.username ?? '')));
    if (path === '/password') {
      return form.password === PASSWORD
        ? send(res, 200, page(otpForm(form.user)))
        : send(res, 200, page(passwordForm('/password', form.user)));
    }
    if (path === '/otp') {
      if (!codeAccepted(form.code ?? '')) return send(res, 200, page(otpForm(form.user)));
      return send(res, 302, '', 'text/plain', {
        location: '/account',
        'set-cookie': `session=${encodeURIComponent(form.user)}; Path=/`,
      });
    }
    // The variant that never takes the password.
    if (path === '/wuser') return send(res, 200, page(passwordForm('/wpass', form.username ?? '')));
    if (path === '/wpass') return send(res, 200, page(passwordForm('/wpass', form.user)));
    return send(res, 404, 'not found', 'text/plain');
  }

  switch (path) {
    case '/login':
      return send(res, 200, page(userForm('/user')));
    case '/wrong':
      return send(res, 200, page(userForm('/wuser')));
    case '/sticky':
      return send(res, 200, page(STICKY));
    case '/account': {
      const user = sessionOf(req);
      return send(res, 200, page(user ? `<p>Logged in as ${decodeURIComponent(user)}</p>` : '<p>Please sign in</p>'));
    }
    case '/recaptcha':
      return send(res, 200, page(RECAPTCHA));
    case '/image-captcha':
      return send(res, 200, page(IMAGE_CAPTCHA));
    case '/ads':
      return send(res, 200, page(ADS));
    case '/ad.js':
    case '/ok.js':
      hits.push(path.slice(1));
      return send(res, 200, `window.${path === '/ad.js' ? 'adLoaded' : 'okLoaded'} = true;`, 'text/javascript');
    case '/hits':
      return send(res, 200, JSON.stringify(hits), 'application/json');
    case '/cookies':
      return send(res, 200, page(COOKIES));
    case '/lists/easylist.txt':
      return send(res, 200, EASYLIST, 'text/plain');
    default:
      return send(res, 404, 'not found', 'text/plain');
  }
}

http.createServer((req, res) => handle(req, res).catch((error) => send(res, 500, String(error), 'text/plain'))).listen(PORT, '127.0.0.1');
