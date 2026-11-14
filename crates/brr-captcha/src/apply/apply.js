(function (p) {
  const resolve = (name) => {
    if (typeof name !== 'string') return undefined;
    return name.split('.').reduce((obj, key) => (obj == null ? undefined : obj[key]), window);
  };
  const set = (selector, value) => {
    for (const el of document.querySelectorAll(selector)) {
      el.value = value;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }
  };
  const call = (callback, token) => {
    const fn = typeof callback === 'function' ? callback : resolve(callback);
    if (typeof fn === 'function') fn(token);
  };
  const dataCallback = (selector, token) => {
    const el = document.querySelector(selector);
    if (el) call(el.getAttribute('data-callback'), token);
  };
  const recaptchaCallbacks = () => {
    const callbacks = [];
    const seen = new Set();
    const walk = (obj, depth) => {
      if (!obj || typeof obj !== 'object' || obj instanceof Node || seen.has(obj) || depth > 6) return;
      seen.add(obj);
      for (const key of Object.keys(obj)) {
        let value;
        try {
          value = obj[key];
        } catch (_) {
          continue;
        }
        if (key === 'callback' && (typeof value === 'function' || typeof value === 'string')) callbacks.push(value);
        else walk(value, depth + 1);
      }
    };
    walk(window.___grecaptcha_cfg && window.___grecaptcha_cfg.clients, 0);
    return callbacks;
  };

  switch (p.type) {
    case 'recaptcha_v2':
      set('textarea[name="g-recaptcha-response"]', p.token);
      for (const callback of recaptchaCallbacks()) call(callback, p.token);
      break;
    case 'recaptcha_v3': {
      const resolved = () => Promise.resolve(p.token);
      if (window.grecaptcha) {
        window.grecaptcha.execute = resolved;
        if (window.grecaptcha.enterprise) window.grecaptcha.enterprise.execute = resolved;
      }
      break;
    }
    case 'hcaptcha':
      set('textarea[name="h-captcha-response"], textarea[name="g-recaptcha-response"]', p.token);
      dataCallback('.h-captcha[data-callback]', p.token);
      break;
    case 'turnstile':
      set('input[name="cf-turnstile-response"]', p.token);
      dataCallback('.cf-turnstile[data-callback]', p.token);
      break;
    case 'funcaptcha':
      set('input#fc-token, [name="fc-token"]', p.token);
      break;
    case 'geetest':
      for (const [name, value] of Object.entries(p.fields)) set('[name="' + name + '"]', value);
      break;
  }
})
