(function () {
  const CAP = 2048;
  const INLINE_SCRIPT_CAP = 200000;

  const cap = (text) => (typeof text === 'string' ? text.slice(0, CAP) : null);
  const attr = (el, name) => (el ? cap(el.getAttribute(name)) : null);
  const param = (url, name) => {
    const match = new RegExp('[?&#]' + name + '=([^&#]+)').exec(url);
    if (!match) return null;
    try {
      return cap(decodeURIComponent(match[1]));
    } catch (_) {
      return cap(match[1]);
    }
  };
  const host = (url) => {
    try {
      return cap(new URL(url).hostname);
    } catch (_) {
      return null;
    }
  };
  const visible = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  const found = (type, fields) =>
    Object.assign(
      {
        type,
        sitekey: null,
        action: null,
        data_s: null,
        enterprise: false,
        invisible: false,
        api_subdomain: null,
        gt: null,
        challenge: null,
        geetest_version: null,
        image_selector: null,
        input_selector: null,
      },
      fields,
    );

  const frames = Array.from(document.querySelectorAll('iframe[src]'), (frame) => frame.src);
  const scripts = Array.from(document.querySelectorAll('script[src]'), (script) => script.src);
  const v3Script = () => {
    for (const url of scripts) {
      const match = /recaptcha\/(api|enterprise)\.js\?(?:[^#]*&)?render=([^&#]+)/.exec(url);
      if (match && match[2] !== 'explicit') return match;
    }
    return null;
  };

  const selector = (el) => {
    const parts = [];
    for (let node = el; node && node.nodeType === 1; node = node.parentElement) {
      if (node.id && node.id.length <= CAP && document.querySelectorAll('#' + CSS.escape(node.id)).length === 1) {
        parts.unshift('#' + CSS.escape(node.id));
        break;
      }
      if (!node.parentElement) {
        parts.unshift(node.localName);
        break;
      }
      const same = Array.from(node.parentElement.children).filter((c) => c.localName === node.localName);
      parts.unshift(node.localName + ':nth-of-type(' + (same.indexOf(node) + 1) + ')');
    }
    return parts.join(' > ');
  };

  const recaptchaV2 = () => {
    const widget = document.querySelector('.g-recaptcha[data-sitekey]');
    // The badge iframe of a v3 page carries the v3 render key; that page is not a v2 challenge
    const v3 = v3Script();
    const anchor = frames.find(
      (url) =>
        /(google\.com|recaptcha\.net)\/recaptcha\/(api2|enterprise)\/anchor/.test(url) &&
        param(url, 'k') &&
        !(v3 && param(url, 'k') === v3[2]),
    );
    if (!widget && !anchor) return null;
    return found('recaptcha_v2', {
      sitekey: attr(widget, 'data-sitekey') || param(anchor, 'k'),
      data_s: attr(widget, 'data-s'),
      enterprise:
        (!!anchor && anchor.includes('/recaptcha/enterprise/')) || scripts.some((url) => /recaptcha\/enterprise\.js/.test(url)),
      invisible: attr(widget, 'data-size') === 'invisible' || (!!anchor && param(anchor, 'size') === 'invisible'),
    });
  };

  const recaptchaV3 = () => {
    const match = v3Script();
    if (!match) return null;
    return found('recaptcha_v3', {
      sitekey: cap(match[2]),
      action: attr(document.querySelector('[data-action]'), 'data-action'),
      enterprise: match[1] === 'enterprise',
    });
  };

  const hcaptcha = () => {
    const widget = document.querySelector('.h-captcha[data-sitekey]');
    const frame = frames.find((url) => /hcaptcha\.com\/captcha/.test(url) && param(url, 'sitekey'));
    if (!widget && !frame) return null;
    return found('hcaptcha', { sitekey: attr(widget, 'data-sitekey') || param(frame, 'sitekey') });
  };

  const turnstile = () => {
    const widget = document.querySelector('.cf-turnstile[data-sitekey]');
    const frame = frames.find((url) => /challenges\.cloudflare\.com/.test(url));
    const frameKey = frame && /\/(0x[0-9A-Za-z_-]{6,})(?:\/|$)/.exec(frame);
    if (!widget && !frameKey) return null;
    const holder = widget || document.querySelector('.cf-turnstile');
    return found('turnstile', {
      sitekey: attr(widget, 'data-sitekey') || (frameKey ? cap(frameKey[1]) : null),
      action: attr(holder, 'data-action'),
      data_s: attr(holder, 'data-cdata'),
    });
  };

  const funcaptcha = () => {
    const holder = document.querySelector('[data-pkey]');
    const source = frames.concat(scripts).find((url) => /(arkoselabs|funcaptcha)\.com/.test(url));
    const pk = attr(holder, 'data-pkey') || (source && (param(source, 'pk') || (/\/v2\/([0-9A-Fa-f-]{36})\//.exec(source) || [])[1]));
    if (!pk) return null;
    return found('funcaptcha', { sitekey: cap(pk), api_subdomain: source ? host(source) : null });
  };

  const geetest = () => {
    const inline = Array.from(document.querySelectorAll('script:not([src])'), (script) =>
      (script.textContent || '').slice(0, INLINE_SCRIPT_CAP),
    );
    const v4Source = scripts.find((url) => /gcaptcha4\.js/.test(url));
    for (const text of inline) {
      const v4 = /initGeetest4\s*\(\s*\{[^}]*?captcha_id\s*:\s*['"]([^'"]+)['"]/.exec(text);
      if (v4) return found('geetest', { gt: cap(v4[1]), geetest_version: 4 });
    }
    const v4Id = v4Source && param(v4Source, 'captcha_id');
    if (v4Id) return found('geetest', { gt: v4Id, geetest_version: 4 });

    const holder = document.querySelector('.geetest_holder[data-gt]');
    if (holder) {
      return found('geetest', { gt: attr(holder, 'data-gt'), challenge: attr(holder, 'data-challenge'), geetest_version: 3 });
    }
    for (const text of inline) {
      if (!text.includes('initGeetest')) continue;
      const gt = /\bgt\s*:\s*['"]([0-9a-f]{32})['"]/.exec(text);
      const challenge = /\bchallenge\s*:\s*['"]([^'"]+)['"]/.exec(text);
      if (gt && challenge) return found('geetest', { gt: cap(gt[1]), challenge: cap(challenge[1]), geetest_version: 3 });
    }
    return null;
  };

  const image = () => {
    const img = Array.from(document.querySelectorAll('img')).find(
      (el) => visible(el) && /captcha/i.test([el.getAttribute('src'), el.id, el.className, el.getAttribute('alt')].join(' ')),
    );
    if (!img) return null;
    const inputs = Array.from(document.querySelectorAll('input')).filter(
      (el) => ['', 'text', 'search', 'tel'].includes(el.type) && visible(el),
    );
    const label = (el) => [el.name, el.id, el.getAttribute('placeholder')].join(' ');
    const input = inputs.find((el) => /captcha/i.test(label(el))) || inputs.find((el) => /code/i.test(label(el)));
    if (!input) return null;
    const imageSelector = selector(img);
    const inputSelector = selector(input);
    if (imageSelector.length > CAP || inputSelector.length > CAP) return null;
    return found('image', { image_selector: imageSelector, input_selector: inputSelector });
  };

  for (const detect of [recaptchaV2, recaptchaV3, hcaptcha, turnstile, funcaptcha, geetest, image]) {
    const result = detect();
    if (result) return result;
  }
  return null;
})()
