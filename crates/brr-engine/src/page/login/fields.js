(() => {
    const visible = (el) => {
        if (el.disabled || el.type === 'hidden') return false;
        const style = getComputedStyle(el);
        if (style.display === 'none' || style.visibility === 'hidden') return false;
        const box = el.getBoundingClientRect();
        return box.width > 0 && box.height > 0;
    };
    const tokens = (el) => (el.getAttribute('autocomplete') || '').toLowerCase().split(/\s+/);
    const inputs = [...document.querySelectorAll('input')].filter(visible);
    const named = (el, pattern) => pattern.test(`${el.name} ${el.id} ${el.getAttribute('autocomplete') || ''}`);

    const passwords = inputs.filter((el) => el.type === 'password');
    const password = inputs.find((el) => tokens(el).includes('current-password')) || passwords[0] || null;

    const totp = inputs.find((el) => tokens(el).includes('one-time-code')) || inputs.find((el) => {
        if (!['text', 'number', 'tel'].includes(el.type) || !/otp|totp|2fa|mfa|code/i.test(`${el.name} ${el.id}`)) return false;
        const length = Number(el.getAttribute('maxlength'));
        return (length >= 4 && length <= 8) || el.getAttribute('inputmode') === 'numeric';
    }) || null;

    const usernameTypes = ['text', 'email', 'tel'];
    let username = inputs.find((el) => el !== password && tokens(el).includes('username')) || null;
    if (!username && password) {
        const before = inputs.slice(0, inputs.indexOf(password))
            .filter((el) => el !== totp && usernameTypes.includes(el.type));
        const sameForm = before.filter((el) => el.form === password.form);
        username = (sameForm.length ? sameForm : before).pop() || null;
    } else if (!username) {
        username = inputs.find((el) => el !== totp && (el.type === 'email' || (el.type === 'text' && named(el, /user|email|login|account/i)))) || null;
    }

    const selector = (el) => {
        if (el.id) {
            const byId = `#${CSS.escape(el.id)}`;
            if (document.querySelectorAll(byId).length === 1) return byId;
        }
        const path = [];
        for (let node = el; node && node !== document.body; node = node.parentElement) {
            const siblings = [...node.parentElement.children].filter((sibling) => sibling.localName === node.localName);
            path.unshift(`${node.localName}:nth-of-type(${siblings.indexOf(node) + 1})`);
        }
        return ['body', ...path].join(' > ');
    };

    const found = [username, password, totp].filter(Boolean);
    const last = found.sort((a, b) => inputs.indexOf(a) - inputs.indexOf(b)).pop();
    let submit = null;
    if (last && last.form) {
        const buttons = [...last.form.querySelectorAll('button, input[type=submit]')]
            .filter((el) => el.type === 'submit' && visible(el));
        submit = buttons[0] || null;
    }

    return {
        username: username && selector(username),
        password: password && selector(password),
        totp: totp && selector(totp),
        submit: submit && selector(submit),
        passwordVisible: passwords.length > 0,
    };
})()
