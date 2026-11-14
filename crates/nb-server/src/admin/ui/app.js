// The platform's administration page (ADR-0008).
// Safety rules for this file: text only ever goes in through textContent or
// text nodes (markup is never built from strings), no code from strings,
// no inline handlers; every request carries the x-nb-admin header and the
// session cookie the scripts cannot read.
'use strict';

(function () {
  const S = { lang: 'ar', dir: 'rtl', words: {}, me: null, view: 'overview', arg: null };

  // ---------------------------------------------------------------- helpers

  function t(key, vars) {
    let s = Object.prototype.hasOwnProperty.call(S.words, key) ? S.words[key] : key;
    if (vars) {
      for (const k of Object.keys(vars)) s = s.split('{' + k + '}').join(String(vars[k]));
    }
    return s;
  }

  /** An element: h('div', {class: 'x', onclick: f}, child, 'text', …). */
  function h(tag, props) {
    const el = document.createElement(tag);
    const p = props || {};
    for (const k of Object.keys(p)) {
      const v = p[k];
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = String(v);
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else if (k === 'value') el.value = v;
      else if (k === 'checked') el.checked = !!v;
      else el.setAttribute(k, v === true ? '' : String(v));
    }
    for (let i = 2; i < arguments.length; i++) add(el, arguments[i]);
    return el;
  }

  function add(el, c) {
    if (c === undefined || c === null || c === false) return;
    if (Array.isArray(c)) { c.forEach(function (x) { add(el, x); }); return; }
    el.appendChild(c instanceof Node ? c : document.createTextNode(String(c)));
  }

  function clear(el) { while (el.firstChild) el.removeChild(el.firstChild); return el; }

  function store(key, value) {
    try {
      if (value === undefined) return window.localStorage.getItem(key);
      window.localStorage.setItem(key, value);
    } catch (e) { /* private window: nothing kept */ }
    return null;
  }

  class Failure extends Error {
    constructor(status, code) { super(code || String(status)); this.status = status; this.code = code; }
  }

  async function api(method, path, body) {
    const opts = {
      method: method,
      credentials: 'same-origin',
      cache: 'no-store',
      headers: { 'x-nb-admin': '1' },
    };
    if (body !== undefined) {
      opts.headers['content-type'] = 'application/json';
      opts.body = JSON.stringify(body);
    }
    const res = await fetch('/admin/api' + path, opts);
    if (res.status === 401 && path !== '/login') {
      S.me = null;
      renderLogin();
      throw new Failure(401, 'unauthorized');
    }
    if (res.status === 204) return null;
    let data = null;
    try { data = await res.json(); } catch (e) { data = null; }
    if (!res.ok) {
      const code = data && (data.error === 'bad_request' || data.error === 'conflict') ? data.detail : data && data.error;
      throw new Failure(res.status, code || '');
    }
    return data;
  }

  function failText(e) {
    if (!(e instanceof Failure)) return t('admin.err.network');
    if (e.status === 429) return t('admin.err.too_many');
    if (e.code === 'admin.code') return t('admin.err.code');
    const key = 'admin.err.' + String(e.code || '').replace(/^admin\./, '');
    if (Object.prototype.hasOwnProperty.call(S.words, key)) return t(key);
    if (e.status === 404) return t('admin.err.not_found');
    return t('admin.err.generic') + ' (' + e.status + ')';
  }

  let toastTimer = null;
  function toast(text) {
    const old = document.querySelector('.toast');
    if (old) old.remove();
    const el = h('div', { class: 'toast', role: 'status', text: text });
    document.body.appendChild(el);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { el.remove(); }, 3500);
  }

  async function act(fn, done) {
    try {
      await fn();
      if (done) toast(done);
      return true;
    } catch (e) {
      if (!(e instanceof Failure && e.status === 401)) toast(failText(e));
      return false;
    }
  }

  // Formats (display only; money stays integers in minor units).
  function locale() { return S.lang === 'ar' ? 'ar-DZ-u-nu-latn' : S.lang + '-DZ'; }
  function when(ms) {
    if (ms === null || ms === undefined) return '—';
    try {
      return new Intl.DateTimeFormat(locale(), { dateStyle: 'medium', timeStyle: 'short', timeZone: 'Africa/Algiers' }).format(new Date(ms));
    } catch (e) { return new Date(ms).toISOString(); }
  }
  function day(ms) {
    if (ms === null || ms === undefined) return '—';
    try {
      return new Intl.DateTimeFormat(locale(), { dateStyle: 'medium', timeZone: 'Africa/Algiers' }).format(new Date(ms));
    } catch (e) { return new Date(ms).toISOString().slice(0, 10); }
  }
  function ago(ms) {
    if (!ms) return t('admin.never');
    const m = Math.round((Date.now() - ms) / 60000);
    if (m < 1) return t('admin.just_now');
    if (m < 60) return t('admin.minutes_ago', { n: m });
    if (m < 1440) return t('admin.hours_ago', { n: Math.round(m / 60) });
    return t('admin.days_ago', { n: Math.round(m / 1440) });
  }
  function count(n) {
    try { return new Intl.NumberFormat(locale()).format(n); } catch (e) { return String(n); }
  }
  function money(minor, exp, cur) {
    const e = exp === undefined ? 2 : exp;
    const neg = minor < 0;
    let s = String(Math.abs(minor));
    while (s.length <= e) s = '0' + s;
    const whole = s.slice(0, s.length - e) || '0';
    const frac = e > 0 ? s.slice(s.length - e) : '';
    const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ' ');
    return (neg ? '-' : '') + grouped + (frac && frac !== '00' ? ',' + frac : '') + (cur ? ' ' + cur : '');
  }
  /** "1500" or "1500,50" → minor units (exp 2), or null. */
  function toMinor(text) {
    const m = String(text).trim().replace(/\s| /g, '').match(/^(\d{1,12})(?:[.,](\d{1,2}))?$/);
    if (!m) return null;
    return Number(m[1]) * 100 + Number((m[2] || '0').padEnd(2, '0'));
  }
  function bytes(n) {
    if (n === null || n === undefined) return '—';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0; let v = n;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return (i === 0 ? String(v) : v.toFixed(v < 10 ? 1 : 0)) + ' ' + units[i];
  }
  function isoDay(ms) {
    if (!ms) return '';
    const d = new Date(ms + 3600000);
    return d.toISOString().slice(0, 10);
  }
  function fromIsoDay(v) {
    if (!v) return null;
    const ms = Date.parse(v + 'T23:59:00+01:00');
    return Number.isFinite(ms) ? ms : null;
  }

  // ------------------------------------------------------------- dialogs

  function dialog(title, body, buttons) {
    return new Promise(function (resolve) {
      const veil = h('div', { class: 'veil' });
      const close = function (v) { veil.remove(); resolve(v); };
      const foot = h('div', { class: 'foot' });
      buttons.forEach(function (b) {
        foot.appendChild(h('button', {
          class: 'btn ' + (b.kind || ''), type: 'button', text: b.label,
          onclick: function () { close(b.value ? b.value() : null); },
        }));
      });
      foot.appendChild(h('button', {
        class: 'btn ghost', type: 'button', text: buttons.length ? t('admin.cancel') : t('admin.close'),
        onclick: function () { close(null); },
      }));
      veil.appendChild(h('div', { class: 'card dialog', role: 'dialog', 'aria-modal': 'true' },
        h('div', { class: 'bar' }, h('h2', { text: title })),
        h('div', { class: 'body' }, body),
        foot));
      veil.addEventListener('keydown', function (e) { if (e.key === 'Escape') close(null); });
      document.body.appendChild(veil);
      const first = veil.querySelector('input, select, textarea, .btn');
      if (first) first.focus();
    });
  }

  function codeInput() {
    return h('input', {
      class: 'input code', inputmode: 'numeric', autocomplete: 'one-time-code',
      maxlength: '6', pattern: '[0-9]*', placeholder: '000000', 'aria-label': t('admin.code'),
    });
  }

  /** A fresh code from the app (and what else the step needs). */
  async function askCode(title, warning, extra) {
    const code = codeInput();
    const body = [];
    if (warning) body.push(h('div', { class: 'note warn', text: warning }));
    if (extra) body.push(extra.el);
    body.push(h('label', { class: 'field' }, h('span', { text: t('admin.code_fresh') }), code));
    return dialog(title, body, [{
      label: t('admin.confirm'), kind: 'primary',
      value: function () { return { code: code.value.trim(), extra: extra ? extra.value() : null }; },
    }]);
  }

  async function askText(title, label, value) {
    const input = h('input', { class: 'input', value: value || '' });
    const r = await dialog(title, [h('label', { class: 'field' }, h('span', { text: label }), input)],
      [{ label: t('admin.confirm'), kind: 'primary', value: function () { return { text: input.value }; } }]);
    return r ? r.text : null;
  }

  // -------------------------------------------------------------- blocks

  function card(title, body, tools, flush) {
    return h('section', { class: 'card' },
      title || tools ? h('div', { class: 'bar' }, h('h2', { text: title || '' }), tools ? h('div', { class: 'row' }, tools) : null) : null,
      h('div', { class: 'body' + (flush ? ' flush' : '') }, body));
  }

  function table(cols, rows, onRow) {
    if (!rows.length) return h('div', { class: 'empty', text: t('admin.empty') });
    const head = h('tr', null, cols.map(function (c) { return h('th', { text: c.label }); }));
    const body = rows.map(function (r) {
      const tr = h('tr', { class: onRow ? 'click' : null, tabindex: onRow ? '0' : null },
        cols.map(function (c) {
          const v = c.cell(r);
          return h('td', { class: c.cls || null }, v);
        }));
      if (onRow) {
        tr.addEventListener('click', function (e) { if (!e.target.closest('button')) onRow(r); });
        tr.addEventListener('keydown', function (e) { if (e.key === 'Enter') onRow(r); });
      }
      return tr;
    });
    return h('div', { class: 'tablewrap' }, h('table', null, h('thead', null, head), h('tbody', null, body)));
  }

  function tile(label, value, tone) {
    return h('div', { class: 'card tile' + (tone ? ' ' + tone : '') },
      h('div', { class: 'muted small', text: label }),
      h('div', { class: 'v num', text: value }));
  }

  function badge(text, tone) { return h('span', { class: 'badge' + (tone ? ' ' + tone : ''), text: text }); }

  function field(label, input) { return h('label', { class: 'field' }, h('span', { text: label }), input); }

  function input(props) { return h('input', Object.assign({ class: 'input' }, props || {})); }

  function select(options, value) {
    const s = h('select', { class: 'input' });
    options.forEach(function (o) {
      s.appendChild(h('option', { value: o[0], text: o[1], selected: o[0] === value ? 'selected' : null }));
    });
    return s;
  }

  function button(label, onclick, kind) {
    const b = h('button', { class: 'btn ' + (kind || ''), type: 'button', text: label });
    b.addEventListener('click', async function () {
      if (b.disabled) return;
      b.disabled = true;
      try { await onclick(); } finally { b.disabled = false; }
    });
    return b;
  }

  function subStatus(sub) {
    if (!sub || !sub.status) return badge(t('admin.sub.none'));
    const now = Date.now();
    if (sub.status === 'cancelled') return badge(t('admin.sub.cancelled'), 'bad');
    if (sub.ends_at && sub.ends_at < now) return badge(t('admin.sub.ended'), 'bad');
    if (sub.ends_at && sub.ends_at < now + 7 * 86400000) return badge(t('admin.sub.ending'), 'warn');
    if (sub.status === 'trial') return badge(t('admin.sub.trial'), 'accent');
    return badge(t('admin.sub.active'), 'ok');
  }

  // --------------------------------------------------------------- sign in

  function renderLogin(message) {
    const app = clear(document.getElementById('app'));
    const login = input({ autocomplete: 'username', autocapitalize: 'none', spellcheck: 'false', dir: 'ltr' });
    const password = input({ type: 'password', autocomplete: 'current-password', dir: 'ltr' });
    const code = codeInput();
    const err = h('div', { class: 'note bad', hidden: message ? null : 'hidden', text: message || '' });
    const go = h('button', { class: 'btn primary', type: 'submit', text: t('admin.sign_in') });
    const form = h('form', { class: 'form', novalidate: 'novalidate' },
      field(t('admin.login'), login),
      field(t('admin.password'), password),
      field(t('admin.code'), code),
      err, go);
    form.addEventListener('submit', async function (e) {
      e.preventDefault();
      go.disabled = true;
      try {
        S.me = await api('POST', '/login', { login: login.value, password: password.value, code: code.value });
        password.value = '';
        code.value = '';
        renderShell();
      } catch (ex) {
        err.hidden = false;
        err.textContent = ex instanceof Failure && ex.status === 401 ? t('admin.err.login') : failText(ex);
        code.value = '';
      } finally {
        go.disabled = false;
      }
    });
    app.appendChild(h('div', { class: 'login' },
      h('div', { class: 'card' },
        h('div', { class: 'brand' }, h('div', { class: 'mark', text: 'N' }),
          h('div', null, h('h1', { text: t('admin.title') }), h('div', { class: 'muted small', text: t('admin.subtitle') }))),
        form,
        h('div', { class: 'muted small', text: t('admin.login_hint') }),
        langPicker())));
    login.focus();
  }

  function langPicker() {
    const s = select([['ar', 'العربية'], ['fr', 'Français'], ['en', 'English']], S.lang);
    s.setAttribute('aria-label', 'Language');
    s.addEventListener('change', async function () {
      store('nb-admin-lang', s.value);
      await loadWords(s.value);
      if (S.me) renderShell(); else renderLogin();
    });
    return s;
  }

  // ----------------------------------------------------------------- frame

  const VIEWS = [
    ['overview', 'admin.nav.overview'],
    ['shops', 'admin.nav.shops'],
    ['accounts', 'admin.nav.accounts'],
    ['billing', 'admin.nav.billing'],
    ['security', 'admin.nav.security'],
    ['log', 'admin.nav.log'],
    ['server', 'admin.nav.server'],
    ['admins', 'admin.nav.admins'],
    ['me', 'admin.nav.me'],
  ];

  function go(view, arg) {
    S.view = view;
    S.arg = arg === undefined ? null : arg;
    renderShell();
  }

  function renderShell() {
    const app = clear(document.getElementById('app'));
    const side = h('nav', { class: 'side', 'aria-label': t('admin.title') },
      h('div', { class: 'brand' }, h('div', { class: 'mark', text: 'N' }), h('h2', { text: t('admin.title') })),
      VIEWS.map(function (v) {
        const on = S.view === v[0] || (S.view === 'org' && v[0] === 'shops') || (S.view === 'account' && v[0] === 'accounts');
        return h('button', { class: 'nav' + (on ? ' on' : ''), type: 'button', onclick: function () { go(v[0]); } },
          h('span', { class: 'dot' }), t(v[1]));
      }),
      h('div', { class: 'grow' }));
    const content = h('div', { class: 'content' }, h('div', { class: 'muted', text: t('admin.loading') }));
    const top = h('header', { class: 'top' },
      h('div', { class: 'who' }, h('strong', { text: S.me ? S.me.name : '' }), h('span', { class: 'muted mono', text: S.me ? S.me.login : '' })),
      h('div', { class: 'row' }, langPicker(), button(t('admin.sign_out'), async function () {
        await act(function () { return api('POST', '/logout', {}); });
        S.me = null;
        renderLogin();
      })));
    app.appendChild(h('div', { class: 'shell' }, side, h('main', { class: 'main' }, top, content)));
    const views = {
      overview: viewOverview, shops: viewShops, org: viewOrg, accounts: viewAccounts, account: viewAccount,
      billing: viewBilling, security: viewSecurity, log: viewLog, server: viewServer, admins: viewAdmins, me: viewMe,
    };
    (views[S.view] || viewOverview)(content).catch(function (e) {
      if (!(e instanceof Failure && e.status === 401)) {
        clear(content).appendChild(h('div', { class: 'note bad', text: failText(e) }));
      }
    });
  }

  function heading(title, sub, tools) {
    return h('div', { class: 'head' },
      h('div', null, h('h1', { text: title }), sub ? h('div', { class: 'sub muted', text: sub }) : null),
      tools ? h('div', { class: 'row' }, tools) : null);
  }

  // -------------------------------------------------------------- overview

  async function viewOverview(el) {
    const o = await api('GET', '/overview');
    const events = Object.keys(o.events_today || {}).map(function (k) { return { kind: k, n: o.events_today[k] }; });
    clear(el).append(
      heading(t('admin.nav.overview'), o.billing ? t('admin.billing_on') : t('admin.billing_off')),
      h('div', { class: 'tiles' },
        tile(t('admin.k.shops'), count(o.orgs)),
        tile(t('admin.k.accounts'), count(o.accounts)),
        tile(t('admin.k.devices'), count(o.devices)),
        tile(t('admin.k.devices_today'), count(o.devices_today)),
        tile(t('admin.k.trial'), count(o.trial)),
        tile(t('admin.k.ending'), count(o.ending), o.ending ? 'warn' : null),
        tile(t('admin.k.ended'), count(o.ended), o.ended ? 'bad' : null),
        tile(t('admin.k.suspended'), count(o.suspended), o.suspended ? 'warn' : null),
        tile(t('admin.k.paid_month'), money(o.paid_month, 2, 'DZD')),
        tile(t('admin.k.blocked'), count(o.blocked))),
      card(t('admin.events_today'), table([
        { label: t('admin.col.kind'), cell: function (r) { return kindLabel(r.kind); } },
        { label: t('admin.col.count'), cell: function (r) { return h('span', { class: 'num', text: count(r.n) }); } },
      ], events, function () { go('security'); }), button(t('admin.see_all'), function () { go('security'); }, 'sm'), true));
  }

  function kindLabel(kind) {
    const key = 'admin.event.' + kind;
    return Object.prototype.hasOwnProperty.call(S.words, key) ? t(key) : kind;
  }

  // ----------------------------------------------------------------- shops

  async function viewShops(el) {
    const st = S.arg || { q: '', filter: '', page: 0 };
    const q = input({ type: 'search', value: st.q, placeholder: t('admin.search_shops') });
    const f = select([['', t('admin.f.all')], ['trial', t('admin.f.trial')], ['ending', t('admin.f.ending')],
      ['ended', t('admin.f.ended')], ['suspended', t('admin.f.suspended')]], st.filter);
    const list = h('div', null, h('div', { class: 'empty', text: t('admin.loading') }));
    const run = async function (page) {
      st.q = q.value; st.filter = f.value; st.page = page;
      S.arg = st;
      const params = new URLSearchParams({ q: st.q, filter: st.filter, page: String(page) });
      const r = await api('GET', '/orgs?' + params.toString());
      clear(list).append(
        table([
          { label: t('admin.col.shop'), cell: function (o) { return h('div', null, h('strong', { text: o.name }), o.suspended ? [' ', badge(t('admin.suspended'), 'bad')] : null); } },
          { label: t('admin.col.owners'), cell: function (o) { return h('span', { class: 'mono', text: o.owners || '—' }); } },
          { label: t('admin.col.devices'), cell: function (o) { return h('span', { class: 'num', text: count(o.devices) }); } },
          { label: t('admin.col.last_seen'), cell: function (o) { return ago(o.last_seen); } },
          { label: t('admin.col.subscription'), cell: function (o) { return h('div', { class: 'row' }, subStatus(o), o.plan ? h('span', { class: 'muted small', text: o.plan }) : null); } },
          { label: t('admin.col.ends'), cell: function (o) { return day(o.ends_at); } },
          { label: t('admin.col.size'), cell: function (o) { return h('span', { class: 'num', text: bytes(o.size) }); } },
        ], r.rows, function (o) { go('org', o.id); }),
        pager(r.total, r.page_size, page, run));
    };
    q.addEventListener('keydown', function (e) { if (e.key === 'Enter') run(0); });
    f.addEventListener('change', function () { run(0); });
    clear(el).append(
      heading(t('admin.nav.shops'), null),
      card(null, list, [q, f, button(t('admin.search'), function () { return run(0); })], true));
    await run(st.page || 0);
  }

  function pager(total, size, page, run) {
    const pages = Math.max(1, Math.ceil(total / size));
    return h('div', { class: 'pager' },
      h('span', { class: 'muted small', text: t('admin.total', { n: count(total) }) }),
      h('div', { class: 'row' },
        off(button(t('admin.prev'), function () { return run(page - 1); }, 'sm'), page <= 0),
        h('span', { class: 'num small', text: (page + 1) + ' / ' + pages }),
        off(button(t('admin.next'), function () { return run(page + 1); }, 'sm'), page + 1 >= pages)));
  }

  function off(b, disabled) { b.disabled = !!disabled; return b; }

  // ------------------------------------------------------------ one shop

  async function viewOrg(el) {
    const id = S.arg;
    const [o, plans] = await Promise.all([api('GET', '/orgs/' + encodeURIComponent(id)), api('GET', '/plans')]);
    const reload = function () { go('org', id); };

    const suspendBtn = o.suspended_at
      ? button(t('admin.resume'), function () { return act(function () { return api('POST', '/orgs/' + encodeURIComponent(id) + '/resume', {}); }, t('admin.done')).then(reload); }, 'primary')
      : button(t('admin.suspend'), async function () {
        const reason = await askText(t('admin.suspend'), t('admin.reason'));
        if (reason === null) return;
        await act(function () { return api('POST', '/orgs/' + encodeURIComponent(id) + '/suspend', { reason: reason }); }, t('admin.done'));
        reload();
      }, 'danger');
    const deleteBtn = button(t('admin.delete_shop'), async function () {
      const name = input({ placeholder: o.name });
      const r = await askCode(t('admin.delete_shop'), t('admin.delete_warning'),
        { el: field(t('admin.type_name'), name), value: function () { return name.value; } });
      if (!r) return;
      const ok = await act(function () { return api('POST', '/orgs/' + encodeURIComponent(id) + '/delete', { name: r.extra, code: r.code }); }, t('admin.done'));
      if (ok) go('shops');
    }, 'danger');

    const info = h('dl', { class: 'kv' },
      h('dt', { text: t('admin.col.id') }), h('dd', { class: 'mono', text: o.id }),
      h('dt', { text: t('admin.created') }), h('dd', { text: when(o.created_at) }),
      h('dt', { text: t('admin.col.size') }), h('dd', { class: 'num', text: bytes(o.size) }),
      h('dt', { text: t('admin.epoch') }), h('dd', { class: 'num', text: String(o.epoch) }),
      o.suspended_at ? [h('dt', { text: t('admin.suspended') }), h('dd', null, badge(when(o.suspended_at), 'bad'), ' ', o.suspend_reason)] : null);

    // Subscription and payments.
    const sub = o.subscription || {};
    const planSel = select([['', '—']].concat(plans.map(function (p) { return [p.code, p.name]; })), sub.plan || '');
    const statusSel = select([['trial', t('admin.sub.trial')], ['active', t('admin.sub.active')], ['cancelled', t('admin.sub.cancelled')]], sub.status || 'trial');
    const ends = input({ type: 'date', value: isoDay(sub.ends_at) });
    const note = input({ value: sub.note || '' });
    const subForm = h('div', { class: 'form cols' },
      field(t('admin.plan'), planSel), field(t('admin.status'), statusSel), field(t('admin.col.ends'), ends), field(t('admin.note'), note),
      button(t('admin.save'), async function () {
        await act(function () {
          return api('POST', '/orgs/' + encodeURIComponent(id) + '/subscription', {
            plan: planSel.value || null, status: statusSel.value, ends_at: fromIsoDay(ends.value), note: note.value,
          });
        }, t('admin.saved'));
        reload();
      }, 'primary'));
    const payPlan = select(plans.map(function (p) { return [p.code, p.name]; }), sub.plan || (plans[0] && plans[0].code));
    const amount = input({ inputmode: 'decimal', dir: 'ltr', placeholder: '0' });
    const months = input({ type: 'number', min: '1', max: '36', value: '1' });
    const method = select(['baridimob', 'ccp', 'cash', 'bank', 'card', 'other'].map(function (m) { return [m, t('admin.method.' + m)]; }), 'baridimob');
    const ref = input({ dir: 'ltr' });
    const fillAmount = function () {
      const p = plans.find(function (x) { return x.code === payPlan.value; });
      if (p && !amount.value) amount.value = money(p.price_minor * Math.max(1, Math.round(Number(months.value) / p.months)), 2, '').trim();
    };
    payPlan.addEventListener('change', function () { amount.value = ''; fillAmount(); });
    fillAmount();
    const payForm = plans.length ? h('div', { class: 'form cols' },
      field(t('admin.plan'), payPlan), field(t('admin.amount'), amount), field(t('admin.months'), months),
      field(t('admin.method'), method), field(t('admin.reference'), ref),
      button(t('admin.record_payment'), async function () {
        const minor = toMinor(amount.value);
        if (minor === null) { toast(t('admin.err.amount')); return; }
        await act(function () {
          return api('POST', '/orgs/' + encodeURIComponent(id) + '/payments', {
            plan: payPlan.value, amount_minor: minor, months: Number(months.value), method: method.value, reference: ref.value,
          });
        }, t('admin.saved'));
        reload();
      }, 'primary')) : h('div', { class: 'note', text: t('admin.no_plans') });

    const devices = table([
      { label: t('admin.col.device'), cell: function (d) { return h('div', null, h('strong', { class: 'mono', text: d.prefix }), ' ', d.name); } },
      { label: t('admin.created'), cell: function (d) { return day(d.created_at); } },
      { label: t('admin.col.last_seen'), cell: function (d) { return ago(d.last_seen); } },
      { label: t('admin.status'), cell: function (d) { return d.revoked_at ? badge(t('admin.revoked'), 'bad') : badge(t('admin.active'), 'ok'); } },
      { label: '', cls: 'actions', cell: function (d) {
        return d.revoked_at ? null : button(t('admin.revoke'), async function () {
          const r = await dialog(t('admin.revoke'), h('div', { text: t('admin.revoke_warning', { name: d.name }) }), [{ label: t('admin.revoke'), kind: 'danger', value: function () { return true; } }]);
          if (!r) return;
          await act(function () { return api('POST', '/devices/' + encodeURIComponent(d.id) + '/revoke', {}); }, t('admin.done'));
          reload();
        }, 'sm danger');
      } },
    ], o.devices);

    const members = table([
      { label: t('admin.col.name'), cell: function (m) { return m.name; } },
      { label: t('admin.col.phone'), cell: function (m) { return h('span', { class: 'mono', text: m.phone }); } },
      { label: t('admin.col.role'), cell: function (m) { return m.role; } },
    ], o.members, function (m) { go('account', m.id); });

    const payments = paymentsTable(o.payments, false);

    // Inside the shop (read only).
    const dataBox = h('div');
    const dataTabs = h('div', { class: 'tabs' });
    const dataSearch = input({ type: 'search', placeholder: t('admin.search') });
    let part = 'summary';
    const showData = async function () {
      clear(dataBox).append(h('div', { class: 'empty', text: t('admin.loading') }));
      const params = new URLSearchParams({ part: part, q: dataSearch.value });
      try {
        const r = await api('GET', '/orgs/' + encodeURIComponent(id) + '/data?' + params.toString());
        clear(dataBox).append(dataView(part, r));
      } catch (e) {
        clear(dataBox).append(h('div', { class: 'empty', text: failText(e) }));
      }
    };
    ['summary', 'customers', 'sales', 'products'].forEach(function (p) {
      dataTabs.appendChild(h('button', {
        class: 'tab' + (p === part ? ' on' : ''), type: 'button', text: t('admin.data.' + p),
        onclick: function (e) {
          part = p;
          dataTabs.querySelectorAll('.tab').forEach(function (b) { b.classList.remove('on'); });
          e.currentTarget.classList.add('on');
          showData();
        },
      }));
    });
    dataSearch.addEventListener('keydown', function (e) { if (e.key === 'Enter') showData(); });

    // Copies.
    const copies = h('div', null, h('div', { class: 'empty', text: t('admin.loading') }));
    const showCopies = async function () {
      const list = await api('GET', '/orgs/' + encodeURIComponent(id) + '/backups');
      clear(copies).append(table([
        { label: t('admin.col.copy'), cell: function (b) { return h('span', { class: 'mono', text: b.name }); } },
        { label: t('admin.col.size'), cell: function (b) { return h('span', { class: 'num', text: bytes(b.size) }); } },
        { label: '', cls: 'actions', cell: function (b) {
          return button(t('admin.restore'), async function () {
            const r = await askCode(t('admin.restore'), t('admin.restore_warning', { name: b.name }));
            if (!r) return;
            await act(function () { return api('POST', '/orgs/' + encodeURIComponent(id) + '/backups/restore', { name: b.name, code: r.code }); }, t('admin.done'));
            reload();
          }, 'sm danger');
        } },
      ], list));
    };

    clear(el).append(
      heading(o.name, o.id, [subStatus(o.subscription), suspendBtn, deleteBtn]),
      h('div', { class: 'grid2' },
        card(t('admin.shop_info'), info),
        card(t('admin.subscription'), subForm)),
      card(t('admin.payments'), [payForm, h('div', { class: 'tablewrap' }, payments)]),
      card(t('admin.devices'), devices, null, true),
      card(t('admin.members'), members, null, true),
      card(t('admin.inside'), [dataTabs, dataBox], [dataSearch, button(t('admin.search'), showData)], true),
      card(t('admin.copies'), copies, button(t('admin.backup_now'), async function () {
        await act(function () { return api('POST', '/orgs/' + encodeURIComponent(id) + '/backups/run', {}); }, t('admin.done'));
        await showCopies();
      }), true));
    await Promise.all([showData(), showCopies()]);
  }

  function dataView(part, r) {
    const cur = r.currency; const exp = r.exp; const d = r.data;
    const m = function (v) { return h('span', { class: 'num', text: money(v || 0, exp, cur) }); };
    if (part === 'summary') {
      return h('div', { class: 'body' }, h('div', { class: 'tiles' },
        tile(t('admin.data.customers'), count(d.customers)),
        tile(t('admin.data.products'), count(d.products)),
        tile(t('admin.data.sales'), count(d.sales)),
        tile(t('admin.data.branches'), count(d.branches)),
        tile(t('admin.data.users'), count(d.users)),
        tile(t('admin.data.sales_30'), money(d.sales_30_total, exp, cur)),
        tile(t('admin.data.debts'), money(d.debts_total, exp, cur))));
    }
    const cols = {
      customers: [
        { label: t('admin.col.code'), cell: function (x) { return h('span', { class: 'mono', text: x.code }); } },
        { label: t('admin.col.name'), cell: function (x) { return x.name; } },
        { label: t('admin.col.phone'), cell: function (x) { return h('span', { class: 'mono', text: x.phone || '—' }); } },
        { label: t('admin.col.city'), cell: function (x) { return x.city || '—'; } },
        { label: t('admin.col.balance'), cell: function (x) { return m(x.balance_minor); } },
      ],
      sales: [
        { label: t('admin.col.number'), cell: function (x) { return h('span', { class: 'mono', text: x.number }); } },
        { label: t('admin.col.date'), cell: function (x) { return when(x.created_at); } },
        { label: t('admin.col.customer'), cell: function (x) { return x.customer || '—'; } },
        { label: t('admin.col.total'), cell: function (x) { return m(x.total_minor); } },
        { label: t('admin.col.credit'), cell: function (x) { return x.credit_minor ? m(x.credit_minor) : '—'; } },
      ],
      products: [
        { label: t('admin.col.sku'), cell: function (x) { return h('span', { class: 'mono', text: x.sku }); } },
        { label: t('admin.col.name'), cell: function (x) { return x.name; } },
        { label: t('admin.col.price'), cell: function (x) { return m(x.price_minor); } },
        { label: t('admin.status'), cell: function (x) { return x.active ? badge(t('admin.active'), 'ok') : badge(t('admin.inactive')); } },
      ],
    };
    return table(cols[part], d.rows || []);
  }

  function paymentsTable(rows, withShop) {
    const cols = [
      { label: t('admin.col.date'), cell: function (p) { return when(p.at); } },
      withShop ? { label: t('admin.col.shop'), cell: function (p) { return p.org_name || p.org_id; } } : null,
      { label: t('admin.plan'), cell: function (p) { return p.plan; } },
      { label: t('admin.amount'), cell: function (p) { return h('span', { class: 'num', text: money(p.amount_minor, 2, p.currency) }); } },
      { label: t('admin.months'), cell: function (p) { return h('span', { class: 'num', text: String(p.months) }); } },
      { label: t('admin.method'), cell: function (p) { return t('admin.method.' + p.method); } },
      { label: t('admin.reference'), cell: function (p) { return h('span', { class: 'mono', text: p.reference || '—' }); } },
      { label: t('admin.col.by'), cell: function (p) { return h('span', { class: 'mono', text: p.admin }); } },
    ].filter(Boolean);
    return table(cols, rows || [], withShop ? function (p) { go('org', p.org_id); } : null);
  }

  // ------------------------------------------------------------- accounts

  async function viewAccounts(el) {
    const st = S.arg || { q: '', page: 0 };
    const q = input({ type: 'search', value: st.q, placeholder: t('admin.search_accounts') });
    const list = h('div');
    const run = async function (page) {
      st.q = q.value; st.page = page; S.arg = st;
      const r = await api('GET', '/accounts?' + new URLSearchParams({ q: st.q, page: String(page) }).toString());
      clear(list).append(table([
        { label: t('admin.col.name'), cell: function (a) { return h('strong', { text: a.name }); } },
        { label: t('admin.col.phone'), cell: function (a) { return h('span', { class: 'mono', text: a.phone }); } },
        { label: t('admin.col.shops'), cell: function (a) { return h('span', { class: 'num', text: count(a.shops) }); } },
        { label: t('admin.created'), cell: function (a) { return day(a.created_at); } },
        { label: t('admin.status'), cell: function (a) { return accountBadge(a.disabled, a.locked_until); } },
      ], r.rows, function (a) { go('account', a.id); }), pager(r.total, r.page_size, page, run));
    };
    q.addEventListener('keydown', function (e) { if (e.key === 'Enter') run(0); });
    clear(el).append(heading(t('admin.nav.accounts')), card(null, list, [q, button(t('admin.search'), function () { return run(0); })], true));
    await run(st.page || 0);
  }

  function accountBadge(disabled, locked) {
    if (disabled) return badge(t('admin.disabled'), 'bad');
    if (locked && locked > Date.now()) return badge(t('admin.locked'), 'warn');
    return badge(t('admin.active'), 'ok');
  }

  async function viewAccount(el) {
    const id = S.arg;
    const a = await api('GET', '/accounts/' + encodeURIComponent(id));
    const reload = function () { go('account', id); };
    const run = function (action, needsCode, warning) {
      return async function () {
        let body = {};
        if (needsCode) {
          const r = await askCode(t('admin.acc.' + action), warning);
          if (!r) return;
          body = { code: r.code };
        }
        let out = null;
        const ok = await act(async function () {
          out = await api('POST', '/accounts/' + encodeURIComponent(id) + '/' + action, body);
        }, action === 'reset' ? null : t('admin.done'));
        if (ok && out && out.code) {
          await dialog(t('admin.acc.reset'), [
            h('div', { text: t('admin.reset_shown') }),
            h('div', { class: 'secret', text: out.code }),
            h('div', { class: 'muted small', text: t('admin.reset_how') }),
          ], []);
        }
        reload();
      };
    };
    clear(el).append(
      heading(a.name, a.phone, [accountBadge(a.disabled_at, a.locked_until)]),
      h('div', { class: 'grid2' },
        card(t('admin.account_info'), h('dl', { class: 'kv' },
          h('dt', { text: t('admin.col.phone') }), h('dd', { class: 'mono', text: a.phone }),
          h('dt', { text: t('admin.created') }), h('dd', { text: when(a.created_at) }),
          h('dt', { text: t('admin.sessions') }), h('dd', { class: 'num', text: count(a.sessions) }),
          a.reset_until ? [h('dt', { text: t('admin.reset_pending') }), h('dd', { text: when(a.reset_until) })] : null)),
        card(t('admin.actions'), h('div', { class: 'row' },
          button(t('admin.acc.unlock'), run('unlock', false)),
          button(t('admin.acc.end_sessions'), run('end_sessions', false)),
          button(t('admin.acc.reset'), run('reset', true, t('admin.reset_warning'))),
          a.disabled_at ? button(t('admin.acc.enable'), run('enable', false), 'primary')
            : button(t('admin.acc.disable'), run('disable', true, t('admin.disable_warning')), 'danger')))),
      card(t('admin.col.shops'), table([
        { label: t('admin.col.shop'), cell: function (s) { return h('strong', { text: s.name }); } },
        { label: t('admin.col.role'), cell: function (s) { return s.role; } },
      ], a.shops, function (s) { go('org', s.id); }), null, true));
  }

  // -------------------------------------------------------------- billing

  async function viewBilling(el) {
    const [plans, settings, payments] = await Promise.all([api('GET', '/plans'), api('GET', '/settings'), api('GET', '/payments')]);
    const code = input({ dir: 'ltr', placeholder: 'pro' });
    const name = input();
    const price = input({ inputmode: 'decimal', dir: 'ltr' });
    const currency = input({ dir: 'ltr', value: 'DZD', maxlength: '3' });
    const months = input({ type: 'number', min: '1', max: '36', value: '1' });
    const maxDev = input({ type: 'number', min: '1', placeholder: '∞' });
    const maxBr = input({ type: 'number', min: '1', placeholder: '∞' });
    const active = h('input', { type: 'checkbox', checked: true });
    const fill = function (p) {
      code.value = p.code; name.value = p.name; price.value = money(p.price_minor, 2, '').trim();
      currency.value = p.currency; months.value = String(p.months);
      maxDev.value = p.max_devices ? String(p.max_devices) : ''; maxBr.value = p.max_branches ? String(p.max_branches) : '';
      active.checked = !!p.active;
      code.focus();
    };
    const planForm = h('div', { class: 'form cols' },
      field(t('admin.plan_code'), code), field(t('admin.col.name'), name), field(t('admin.price'), price),
      field(t('admin.currency'), currency), field(t('admin.months'), months),
      field(t('admin.max_devices'), maxDev), field(t('admin.max_branches'), maxBr),
      h('label', { class: 'row' }, active, t('admin.active')),
      button(t('admin.save'), async function () {
        const minor = toMinor(price.value);
        if (minor === null) { toast(t('admin.err.amount')); return; }
        const ok = await act(function () {
          return api('POST', '/plans', {
            code: code.value.trim(), name: name.value, price_minor: minor, currency: currency.value.trim().toUpperCase(),
            months: Number(months.value), max_devices: maxDev.value ? Number(maxDev.value) : null,
            max_branches: maxBr.value ? Number(maxBr.value) : null, active: active.checked,
          });
        }, t('admin.saved'));
        if (ok) go('billing');
      }, 'primary'));
    const trial = input({ type: 'number', min: '0', max: '365', value: String(settings.trial_days) });
    const grace = input({ type: 'number', min: '0', max: '90', value: String(settings.grace_days) });
    clear(el).append(
      heading(t('admin.nav.billing'), settings.billing ? t('admin.billing_on') : t('admin.billing_off')),
      card(t('admin.plans'), [
        table([
          { label: t('admin.plan_code'), cell: function (p) { return h('span', { class: 'mono', text: p.code }); } },
          { label: t('admin.col.name'), cell: function (p) { return h('strong', { text: p.name }); } },
          { label: t('admin.price'), cell: function (p) { return h('span', { class: 'num', text: money(p.price_minor, 2, p.currency) }); } },
          { label: t('admin.months'), cell: function (p) { return h('span', { class: 'num', text: String(p.months) }); } },
          { label: t('admin.max_devices'), cell: function (p) { return p.max_devices ? String(p.max_devices) : '∞'; } },
          { label: t('admin.max_branches'), cell: function (p) { return p.max_branches ? String(p.max_branches) : '∞'; } },
          { label: t('admin.status'), cell: function (p) { return p.active ? badge(t('admin.active'), 'ok') : badge(t('admin.inactive')); } },
        ], plans, fill),
        h('div', { class: 'body' }, planForm)], null, true),
      card(t('admin.trial_grace'), h('div', { class: 'form cols' },
        field(t('admin.trial_days'), trial), field(t('admin.grace_days'), grace),
        button(t('admin.save'), function () {
          return act(function () { return api('POST', '/settings', { trial_days: Number(trial.value), grace_days: Number(grace.value) }); }, t('admin.saved'));
        }, 'primary'))),
      card(t('admin.payments'), paymentsTable(payments, true), null, true));
  }

  // ------------------------------------------------------------- security

  async function viewSecurity(el) {
    const st = S.arg || { kind: '', ip: '' };
    const r = await api('GET', '/security?' + new URLSearchParams(st).toString());
    const kinds = ['', 'account.login_failed', 'account.locked', 'account.recover_failed', 'admin.login_failed', 'admin.locked',
      'admin.code_refused', 'admin.session_moved', 'admin.forged_request', 'admin.address_refused', 'rate_limited', 'db.refused'];
    const kind = select(kinds.map(function (k) { return [k, k ? kindLabel(k) : t('admin.f.all')]; }), st.kind);
    const ip = input({ dir: 'ltr', value: st.ip, placeholder: 'IP' });
    const apply = function () { go('security', { kind: kind.value, ip: ip.value.trim() }); };
    kind.addEventListener('change', apply);
    ip.addEventListener('keydown', function (e) { if (e.key === 'Enter') apply(); });
    const blockIp = async function (addr) {
      const hours = select([['24', t('admin.block.day')], ['168', t('admin.block.week')], ['', t('admin.block.forever')]], '24');
      const reason = input();
      const target = input({ dir: 'ltr', value: addr || '' });
      const res = await dialog(t('admin.block'), [field('IP', target), field(t('admin.block.for'), hours), field(t('admin.reason'), reason)],
        [{ label: t('admin.block'), kind: 'danger', value: function () { return true; } }]);
      if (!res) return;
      await act(function () {
        return api('POST', '/blocks', { ip: target.value.trim(), reason: reason.value, hours: hours.value ? Number(hours.value) : null });
      }, t('admin.done'));
      go('security', st);
    };
    clear(el).append(
      heading(t('admin.nav.security'), t('admin.security_sub'), [button(t('admin.block'), function () { return blockIp(''); }, 'danger')]),
      h('div', { class: 'grid2' },
        card(t('admin.top_ips'), table([
          { label: 'IP', cell: function (x) { return h('span', { class: 'mono', text: x.ip }); } },
          { label: t('admin.col.count'), cell: function (x) { return h('span', { class: 'num', text: count(x.count) }); } },
          { label: t('admin.col.last'), cell: function (x) { return ago(x.last); } },
          { label: '', cls: 'actions', cell: function (x) { return button(t('admin.block_short'), function () { return blockIp(x.ip); }, 'sm danger'); } },
        ], r.top, function (x) { go('security', { kind: '', ip: x.ip }); }), null, true),
        card(t('admin.blocked'), table([
          { label: 'IP', cell: function (x) { return h('span', { class: 'mono', text: x.ip }); } },
          { label: t('admin.reason'), cell: function (x) { return x.reason || '—'; } },
          { label: t('admin.until'), cell: function (x) { return x.until ? when(x.until) : t('admin.block.forever'); } },
          { label: '', cls: 'actions', cell: function (x) {
            return button(t('admin.unblock'), async function () {
              await act(function () { return api('POST', '/blocks/remove', { ip: x.ip }); }, t('admin.done'));
              go('security', st);
            }, 'sm');
          } },
        ], r.blocked), null, true)),
      card(t('admin.events'), table([
        { label: t('admin.col.date'), cell: function (x) { return when(x.at); } },
        { label: t('admin.col.kind'), cell: function (x) { return kindLabel(x.kind); } },
        { label: 'IP', cell: function (x) { return h('span', { class: 'mono', text: x.ip }); } },
        { label: t('admin.col.subject'), cell: function (x) { return h('span', { class: 'mono', text: x.subject || '—' }); } },
        { label: t('admin.col.detail'), cell: function (x) { return h('span', { class: 'small muted', text: x.detail || '' }); } },
      ], r.events), [kind, ip], true));
  }

  // ------------------------------------------------------------------ log

  async function viewLog(el) {
    const r = await api('GET', '/log');
    clear(el).append(
      heading(t('admin.nav.log'), t('admin.log_sub')),
      r.broken_at === null
        ? h('div', { class: 'note ok', text: t('admin.chain_ok') })
        : h('div', { class: 'note bad', text: t('admin.chain_broken', { n: r.broken_at }) }),
      card(null, table([
        { label: '#', cell: function (x) { return h('span', { class: 'num muted', text: String(x.seq) }); } },
        { label: t('admin.col.date'), cell: function (x) { return when(x.at); } },
        { label: t('admin.col.by'), cell: function (x) { return h('span', { class: 'mono', text: x.admin }); } },
        { label: 'IP', cell: function (x) { return h('span', { class: 'mono', text: x.ip }); } },
        { label: t('admin.col.action'), cell: function (x) { return h('span', { class: 'mono', text: x.action }); } },
        { label: t('admin.col.target'), cell: function (x) { return h('span', { class: 'mono small', text: x.target }); } },
        { label: t('admin.col.detail'), cell: function (x) { return h('span', { class: 'small muted', text: x.detail }); } },
      ], r.rows), null, true));
  }

  // --------------------------------------------------------------- server

  async function viewServer(el) {
    const s = await api('GET', '/server');
    const up = Math.max(0, s.now - s.started_at);
    const used = s.disk_total && s.disk_free !== null ? s.disk_total - s.disk_free : null;
    const low = s.disk_total && s.disk_free !== null && s.disk_free < s.disk_total * 0.1;
    clear(el).append(
      heading(t('admin.nav.server'), 'nb-server ' + s.version),
      h('div', { class: 'tiles' },
        tile(t('admin.srv.uptime'), t('admin.srv.uptime_v', { d: Math.floor(up / 86400000), h: Math.floor(up / 3600000) % 24 })),
        tile(t('admin.srv.disk_free'), bytes(s.disk_free), low ? 'bad' : null),
        tile(t('admin.srv.disk_used'), bytes(used)),
        tile(t('admin.srv.shops_data'), bytes(s.size_tenants)),
        tile(t('admin.srv.backups'), bytes(s.size_backups)),
        tile(t('admin.srv.deleted'), bytes(s.size_deleted)),
        tile(t('admin.srv.open_shops'), count(s.open_shops)),
        tile(t('admin.srv.live'), count(s.live_connections))),
      card(t('admin.srv.settings'), h('dl', { class: 'kv' },
        h('dt', { text: t('admin.srv.started') }), h('dd', { text: when(s.started_at) }),
        h('dt', { text: t('admin.srv.billing') }), h('dd', null, s.billing ? badge(t('admin.on'), 'ok') : badge(t('admin.off'))),
        h('dt', { text: t('admin.srv.signup') }), h('dd', null, s.open_signup ? badge(t('admin.on'), 'warn') : badge(t('admin.off'))),
        h('dt', { text: t('admin.srv.admin_networks') }), h('dd', null, s.admin_networks ? badge(t('admin.on'), 'ok') : badge(t('admin.srv.local_only'), 'warn'))),
        null));
  }

  // --------------------------------------------------------------- admins

  async function viewAdmins(el) {
    const list = await api('GET', '/admins');
    const login = input({ dir: 'ltr', autocapitalize: 'none' });
    const name = input();
    const password = input({ type: 'password', autocomplete: 'new-password', dir: 'ltr' });
    clear(el).append(
      heading(t('admin.nav.admins'), t('admin.admins_sub')),
      card(null, table([
        { label: t('admin.login'), cell: function (a) { return h('span', { class: 'mono', text: a.login }); } },
        { label: t('admin.col.name'), cell: function (a) { return a.name; } },
        { label: t('admin.created'), cell: function (a) { return day(a.created_at); } },
        { label: t('admin.status'), cell: function (a) { return accountBadge(a.disabled, a.locked_until); } },
        { label: '', cls: 'actions', cell: function (a) {
          return a.disabled || (S.me && a.login === S.me.login) ? null : button(t('admin.acc.disable'), async function () {
            const r = await askCode(t('admin.acc.disable'), t('admin.disable_admin_warning', { name: a.login }));
            if (!r) return;
            await act(function () { return api('POST', '/admins/' + encodeURIComponent(a.id) + '/disable', { code: r.code }); }, t('admin.done'));
            go('admins');
          }, 'sm danger');
        } },
      ], list), null, true),
      card(t('admin.add_admin'), h('div', { class: 'form cols' },
        field(t('admin.login'), login), field(t('admin.col.name'), name), field(t('admin.password_new'), password),
        button(t('admin.add_admin'), async function () {
          const r = await askCode(t('admin.add_admin'), t('admin.add_admin_warning'));
          if (!r) return;
          let made = null;
          const ok = await act(async function () {
            made = await api('POST', '/admins', { login: login.value, name: name.value, password: password.value, code: r.code });
          });
          password.value = '';
          if (ok && made) {
            await dialog(t('admin.scan_title'), [
              h('div', { text: t('admin.scan_help') }),
              h('div', { class: 'qr' }, h('img', { src: made.qr, alt: made.uri })),
              h('div', { class: 'secret', text: made.secret }),
            ], []);
            go('admins');
          }
        }, 'primary'))));
  }

  // ---------------------------------------------------------------- me

  async function viewMe(el) {
    const old = input({ type: 'password', autocomplete: 'current-password', dir: 'ltr' });
    const fresh = input({ type: 'password', autocomplete: 'new-password', dir: 'ltr' });
    const again = input({ type: 'password', autocomplete: 'new-password', dir: 'ltr' });
    const code = codeInput();
    clear(el).append(
      heading(t('admin.nav.me'), S.me ? S.me.login : ''),
      card(t('admin.change_password'), h('div', { class: 'form' },
        field(t('admin.password_old'), old), field(t('admin.password_new'), fresh), field(t('admin.password_again'), again),
        field(t('admin.code'), code),
        h('div', { class: 'muted small', text: t('admin.password_rule') }),
        h('div', null, button(t('admin.save'), async function () {
          if (fresh.value !== again.value) { toast(t('admin.err.password_again')); return; }
          const ok = await act(function () { return api('POST', '/password', { old: old.value, new: fresh.value, code: code.value }); }, t('admin.saved'));
          if (ok) { old.value = ''; fresh.value = ''; again.value = ''; }
          code.value = '';
        }, 'primary')))));
  }

  // ---------------------------------------------------------------- start

  async function loadWords(lang) {
    const r = await fetch('/admin/api/strings?lang=' + encodeURIComponent(lang), { credentials: 'same-origin' });
    const w = await r.json();
    S.lang = w.lang; S.dir = w.dir; S.words = w.words;
    document.documentElement.lang = w.lang;
    document.documentElement.dir = w.dir;
    document.title = t('admin.title') + ' · NSH Business';
  }

  async function start() {
    const wanted = store('nb-admin-lang') || (navigator.language || 'ar').slice(0, 2);
    try { await loadWords(['ar', 'fr', 'en'].includes(wanted) ? wanted : 'ar'); } catch (e) { /* keys shown */ }
    try {
      S.me = await api('GET', '/me');
      renderShell();
    } catch (e) {
      if (!(e instanceof Failure && e.status === 401)) renderLogin(failText(e));
    }
  }

  start();
})();
