#!/usr/bin/env node
// scripts/page-script-check/run.js - runs the real Servo page script
// (crates/ferrite-engine-servo/src/page_ops.js) against real HTML under jsdom.
//
// Why this exists: the page script is JavaScript inside a Rust crate, and the
// real Servo engine cannot be built in most environments, so nothing in
// `cargo test` executes it. `cargo test` only proves it *parses* (`node
// --check`) and that its select-matching logic works against a stub. This
// harness is the strongest check available without a browser: it builds the
// exact wrapper the Rust side builds, runs each operation against HTML
// fixtures, and asserts on the JSON the Rust side would deserialize.
//
// Usage (network needed once, for jsdom):
//   cd scripts/page-script-check && npm install && node run.js
//
// Optional: `FERRITE_PAGE_SCRIPT_DUMP=/tmp/dump cargo test -p ferrite-engine-servo
// every_assembled_script_parses_under_node` then `node run.js --check-dump
// /tmp/dump` verifies this harness wraps the script exactly like the Rust
// `build_script` does (so a green run here is a claim about the shipped string).
//
// jsdom has no layout engine. The harness stubs one: an element's rect is
// `data-rect="left,top,width,height"` if present, else a 200x18 box stacked 20px
// per element in document order; the viewport is 800x600. That is enough to
// exercise visibility, viewport and off-screen logic deterministically.
'use strict';

const fs = require('fs');
const path = require('path');
const assert = require('assert');
const { JSDOM } = require('jsdom');

const SCRIPT_PATH = path.join(__dirname, '..', '..', 'crates', 'ferrite-engine-servo', 'src', 'page_ops.js');
const PAGE_OPS = fs.readFileSync(SCRIPT_PATH, 'utf8');

function jsonLiteral(v) {
  return JSON.stringify(v).replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
}

// Must stay byte-identical to `script::build_script` in the Rust crate.
function wrap(op, args) {
  return '(function () { "use strict"; var __op = ' + jsonLiteral(op) + '; var __args = ' + jsonLiteral(args) + ';\n' +
    PAGE_OPS + '\n})()';
}

const REF = 'data-ferrite-ref';

function page(html, opts) {
  opts = opts || {};
  const dom = new JSDOM('<!doctype html><html><head><title>' + (opts.title || 'Test page') + '</title></head><body>' + html + '</body></html>', {
    url: opts.url || 'https://shop.example/checkout?x=1',
    runScripts: 'outside-only',
    pretendToBeVisual: true,
  });
  const w = dom.window;
  Object.defineProperty(w, 'innerWidth', { value: 800, configurable: true });
  Object.defineProperty(w, 'innerHeight', { value: 600, configurable: true });
  let scrollTop = opts.scrollTop || 0;
  Object.defineProperty(w, 'pageYOffset', { get: () => scrollTop, configurable: true });
  const order = new Map();
  w.Element.prototype.getBoundingClientRect = function () {
    const explicit = this.getAttribute && this.getAttribute('data-rect');
    let left, top, width, height;
    if (explicit) {
      [left, top, width, height] = explicit.split(',').map(Number);
    } else {
      if (!order.size || !order.has(this)) {
        order.clear();
        let i = 0;
        for (const el of this.ownerDocument.querySelectorAll('*')) order.set(el, i++);
        // shadow content
        for (const host of this.ownerDocument.querySelectorAll('*')) {
          if (host.shadowRoot) for (const el of host.shadowRoot.querySelectorAll('*')) order.set(el, i++);
        }
      }
      left = 0; top = (order.get(this) || 0) * 20 - scrollTop; width = 200; height = 18;
    }
    return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top };
  };
  w.Element.prototype.scrollIntoView = function () { this.__scrolledIntoView = (this.__scrolledIntoView || 0) + 1; };
  Object.defineProperty(w.Element.prototype, 'scrollHeight', { get() { return this === w.document.documentElement ? (opts.scrollHeight || 600) : 0; }, configurable: true });
  Object.defineProperty(w.Element.prototype, 'clientHeight', { get() { return this === w.document.documentElement ? 600 : 0; }, configurable: true });
  const run = (op, args) => {
    const raw = w.eval(wrap(op, args || {}));
    // The Rust side reads this through Debug-formatted text: it must be ASCII.
    assert.ok(/^[\x20-\x7e]*$/.test(raw), 'script output must be printable ASCII, got: ' + raw.slice(0, 80));
    return JSON.parse(raw);
  };
  return { dom, w, doc: w.document, run, setScroll: (y) => { scrollTop = y; } };
}

const tests = [];
function test(name, fn) { tests.push({ name, fn }); }

const byLabel = (d, label) => d.elements.find((e) => e.label === label);
const sel = (n) => '[' + REF + '="' + n + '"]';

// -- fixtures --------------------------------------------------------------

const CHECKOUT = `
<h1>Checkout</h1>
<p>Order total: <b>42.00</b> dollars. Free returns.</p>
<script>var secret = "SCRIPT_TEXT_MUST_NOT_APPEAR";</script>
<style>.x { color: red } /* STYLE_TEXT_MUST_NOT_APPEAR */</style>
<div style="display:none">HIDDEN_BLOCK_TEXT</div>
<div style="visibility:hidden">INVISIBLE_TEXT</div>
<a href="/cart">Back to cart</a>
<a href="https://other.example/help" title="Help centre">?</a>
<a>anchor without href</a>
<form id="f1" action="/pay">
  <label for="email">Email address</label><input id="email" type="email" placeholder="you@example.com" value="ada@example.com" required>
  <label>Card holder <input name="holder" type="text" value="Ada"></label>
  <input aria-label="Password" type="password" value="hunter2" name="pw">
  <input type="text" name="cc" autocomplete="cc-number" value="4111111111111111" aria-label="Card number">
  <label for="country">Country</label>
  <select id="country" name="country">
    <option value="us">United States</option>
    <option value="ca" selected>Canada</option>
    <option value="mx">Mexico</option>
  </select>
  <label><input type="checkbox" id="terms"> I accept the terms</label>
  <label><input type="checkbox" id="news" checked> Newsletter</label>
  <input type="radio" name="ship" id="ship1" checked aria-label="Standard shipping">
  <input type="radio" name="ship" id="ship2" aria-label="Express shipping">
  <textarea aria-label="Notes">Leave at door</textarea>
  <input type="hidden" name="csrf" value="SECRET_TOKEN">
  <button type="submit" id="pay">Pay now</button>
  <button type="button" disabled>Disabled thing</button>
  <input type="submit" value="Save draft">
</form>
<div role="button" aria-label="Menu" aria-expanded="false">\u2261</div>
<div onclick="go()" style="cursor:pointer">Clickable via onclick</div>
<div onclick="go()">Not pointer onclick</div>
<div contenteditable="true">Editable note</div>
<details><summary>More info</summary>Hidden details</details>
<button style="display:none">Hidden button</button>
<button style="visibility:hidden">Invisible button</button>
`;

// -- digest ----------------------------------------------------------------

test('digest: page identity, visible text, no script/style/hidden text', () => {
  const p = page(CHECKOUT);
  const d = p.run('digest');
  assert.strictEqual(d.url, 'https://shop.example/checkout?x=1');
  assert.strictEqual(d.title, 'Test page');
  assert.ok(d.text.includes('Order total: 42.00 dollars. Free returns.'), d.text);
  for (const bad of ['SCRIPT_TEXT_MUST_NOT_APPEAR', 'STYLE_TEXT_MUST_NOT_APPEAR', 'HIDDEN_BLOCK_TEXT', 'INVISIBLE_TEXT', 'SECRET_TOKEN']) {
    assert.ok(!d.text.includes(bad), bad + ' leaked into page text: ' + d.text);
  }
  assert.ok(!/\s{2,}/.test(d.text), 'whitespace must be collapsed');
});

test('digest: element roles, accessible names and states', () => {
  const p = page(CHECKOUT);
  const d = p.run('digest');
  const el = (label) => { const e = byLabel(d, label); assert.ok(e, 'no element labelled ' + JSON.stringify(label) + ' in ' + JSON.stringify(d.elements.map((x) => x.label))); return e; };

  const back = el('Back to cart');
  assert.strictEqual(back.role, 'link');
  assert.strictEqual(back.href, 'https://shop.example/cart', 'href must be absolute');
  assert.strictEqual(el('Help centre').role, 'link', 'title is used before the "?" text');

  const email = el('Email address');
  assert.strictEqual(email.role, 'textbox');
  assert.strictEqual(email.input_type, 'email');
  assert.strictEqual(email.value, 'ada@example.com');
  assert.strictEqual(email.placeholder, 'you@example.com');
  assert.strictEqual(email.form, 0);

  assert.strictEqual(el('Card holder').role, 'textbox', 'wrapping <label> text names the field');

  const pw = el('Password');
  assert.strictEqual(pw.sensitive, true);
  assert.strictEqual(pw.value, null, 'a password value must never be read');
  assert.strictEqual(pw.input_type, 'password');
  const cc = el('Card number');
  assert.strictEqual(cc.sensitive, true, 'autocomplete=cc-number is sensitive');
  assert.strictEqual(cc.value, null);
  assert.ok(!JSON.stringify(d).includes('hunter2'), 'password leaked');
  assert.ok(!JSON.stringify(d).includes('4111111111111111'), 'card number leaked');
  assert.ok(!JSON.stringify(d).includes('SECRET_TOKEN'), 'hidden input leaked');

  const country = el('Country');
  assert.strictEqual(country.role, 'combobox');
  assert.deepStrictEqual(country.options, ['United States', 'Canada', 'Mexico'], 'options are the visible labels');
  assert.strictEqual(country.value, 'Canada', 'the value is the selected option label');

  const terms = el('I accept the terms');
  assert.strictEqual(terms.role, 'checkbox');
  assert.strictEqual(terms.checked, false);
  assert.strictEqual(el('Newsletter').checked, true);
  assert.strictEqual(el('Standard shipping').role, 'radio');
  assert.strictEqual(el('Standard shipping').checked, true);
  assert.strictEqual(el('Express shipping').checked, false);

  assert.strictEqual(el('Notes').role, 'textbox');
  assert.strictEqual(el('Notes').input_type, 'textarea');
  assert.strictEqual(el('Notes').value, 'Leave at door');

  assert.strictEqual(el('Pay now').role, 'button');
  const dis = el('Disabled thing');
  assert.strictEqual(dis.disabled, true);
  assert.strictEqual(el('Save draft').role, 'button', 'submit input is named by its value');

  const menu = el('Menu');
  assert.strictEqual(menu.role, 'button');
  assert.strictEqual(menu.expanded, false);
  assert.strictEqual(el('Clickable via onclick').role, 'button');
  assert.ok(!byLabel(d, 'Not pointer onclick'), '[onclick] without a pointer cursor is not listed');
  assert.ok(!byLabel(d, 'anchor without href'));
  const note = d.elements.find((e) => e.value === 'Editable note');
  assert.ok(note && note.role === 'textbox', 'contenteditable is a textbox whose value is its content');
  assert.strictEqual(el('More info').role, 'button');
  assert.strictEqual(el('More info').expanded, false);
  assert.ok(!byLabel(d, 'Hidden button') && !byLabel(d, 'Invisible button'), 'hidden elements are not listed');
  assert.ok(d.elements.every((e) => !('csrf' === e.label)), 'type=hidden is not listed');
});

// The Rust crate deserializes THIS output in a unit test (script.rs,
// `a_real_page_script_digest_deserializes_and_renders`), so the two sides are
// checked against the same bytes. `node run.js --update-fixture` regenerates it.
const FIXTURE = path.join(__dirname, '..', '..', 'crates', 'ferrite-engine-servo', 'tests', 'fixtures', 'checkout_digest.json');
test('digest: the checkout fixture the Rust contract test reads is current', () => {
  const text = JSON.stringify(page(CHECKOUT).run('digest'), null, 2) + '\n';
  if (process.argv.includes('--update-fixture')) {
    fs.mkdirSync(path.dirname(FIXTURE), { recursive: true });
    fs.writeFileSync(FIXTURE, text);
  }
  assert.strictEqual(fs.readFileSync(FIXTURE, 'utf8'), text, 'stale fixture: run `node run.js --update-fixture`');
});

test('digest: output matches the fields Rust deserializes (PageDigest / DigestElement)', () => {
  const d = page(CHECKOUT).run('digest');
  assert.deepStrictEqual(Object.keys(d).sort(), ['elements', 'elements_truncated', 'scroll', 'text', 'title', 'url']);
  assert.deepStrictEqual(Object.keys(d.scroll).sort(), ['max_y', 'viewport_height', 'y']);
  const want = ['checked', 'disabled', 'expanded', 'form', 'href', 'in_viewport', 'input_type', 'label', 'options', 'placeholder', 'ref_id', 'role', 'selected', 'sensitive', 'value'];
  for (const e of d.elements) assert.deepStrictEqual(Object.keys(e).sort(), want, JSON.stringify(e));
  assert.ok(d.elements.every((e) => Number.isInteger(e.ref_id) && e.ref_id > 0));
  assert.strictEqual(new Set(d.elements.map((e) => e.ref_id)).size, d.elements.length, 'refs are unique');
});

test('refs: stamped on the live DOM, stable across re-digest, new elements get max+1', () => {
  const p = page('<button id="a">A</button><button id="b">B</button>');
  const d1 = p.run('digest');
  const a = byLabel(d1, 'A');
  const b = byLabel(d1, 'B');
  assert.strictEqual(p.doc.getElementById('a').getAttribute(REF), String(a.ref_id));
  assert.strictEqual(p.doc.querySelector(sel(b.ref_id)).id, 'b');
  const d2 = p.run('digest');
  assert.strictEqual(byLabel(d2, 'A').ref_id, a.ref_id, 'ref must be reused');
  assert.strictEqual(byLabel(d2, 'B').ref_id, b.ref_id);
  // A new element is inserted BEFORE the others in document order.
  p.doc.body.insertAdjacentHTML('afterbegin', '<button id="c">C</button>');
  const d3 = p.run('digest');
  assert.strictEqual(byLabel(d3, 'A').ref_id, a.ref_id);
  assert.strictEqual(byLabel(d3, 'C').ref_id, Math.max(a.ref_id, b.ref_id) + 1, 'only the new element takes max+1');
});

test('refs: a cloned node (duplicate stamp) gets a fresh ref', () => {
  const p = page('<button id="a">A</button>');
  const d1 = p.run('digest');
  const clone = p.doc.getElementById('a').cloneNode(true);
  clone.id = 'a2';
  p.doc.body.appendChild(clone);
  const d2 = p.run('digest');
  const refs = d2.elements.map((e) => e.ref_id);
  assert.strictEqual(new Set(refs).size, 2, 'duplicate stamps must be resolved: ' + refs);
  assert.strictEqual(p.doc.getElementById('a').getAttribute(REF), String(d1.elements[0].ref_id), 'the original keeps its ref');
});

test('refs: an unknown ref is not_found (Rust turns this into "read_page again")', () => {
  const p = page('<button>A</button>');
  p.run('digest');
  assert.deepStrictEqual(p.run('click', { sel: sel(999) }), { ok: false, error: 'not_found' });
  assert.deepStrictEqual(p.run('click', { sel: 'div[' }), { ok: false, error: 'bad_selector' });
});

test('digest: in_viewport, off-screen elements, and near-viewport priority when over the cap', () => {
  let html = '';
  for (let i = 0; i < 300; i++) html += `<button data-rect="0,${i * 30},100,20">B${i}</button>`;
  const p = page(html, { scrollHeight: 9000 });
  const d = p.run('digest');
  assert.strictEqual(d.elements.length, 150);
  assert.strictEqual(d.elements_truncated, true);
  const inView = d.elements.filter((e) => e.in_viewport);
  assert.strictEqual(inView.length, 20, 'the 20 buttons within 0..600px are in the viewport');
  assert.ok(byLabel(d, 'B0') && byLabel(d, 'B19'), 'in-viewport elements always survive the cap');
  assert.ok(byLabel(d, 'B149'), 'then the nearest off-screen ones');
  assert.ok(!byLabel(d, 'B299'), 'the farthest are dropped');
  const labels = d.elements.map((e) => e.label);
  assert.deepStrictEqual(labels, labels.slice().sort((a, b) => Number(a.slice(1)) - Number(b.slice(1))), 'document order is kept');
  assert.strictEqual(d.scroll.max_y, 9000 - 600);
  assert.strictEqual(d.scroll.viewport_height, 600);
});

test('digest: after scrolling, the newly visible elements are the ones listed', () => {
  let html = '';
  for (let i = 0; i < 400; i++) html += `<button data-rect="0,${i * 30},100,20">B${i}</button>`;
  const p = page(html, { scrollHeight: 12000 });
  p.setScroll(9000); // elements are stubbed absolute in the document via data-rect; shift by scroll
  p.w.Element.prototype.getBoundingClientRect = function () {
    const [l, t, w, h] = this.getAttribute('data-rect').split(',').map(Number);
    return { left: l, top: t - 9000, width: w, height: h, right: l + w, bottom: t - 9000 + h };
  };
  const d = p.run('digest');
  const visible = d.elements.filter((e) => e.in_viewport).map((e) => e.label);
  assert.ok(visible.includes('B300') && visible.includes('B310'), visible.join());
  assert.ok(!byLabel(d, 'B0'), 'elements far above the viewport are not listed');
  assert.strictEqual(d.scroll.y, 9000);
});

test('digest: unicode survives, control/invisible/bidi characters and lone surrogates are stripped', () => {
  const p = page('<p id="t"></p><button id="b">x</button>');
  p.doc.getElementById('t').textContent = 'Caf\u00e9 \u2713 \ud83d\ude00 a\u200bb \u202eEVIL\u202c \u0007bell \ud800 lone';
  p.doc.getElementById('b').setAttribute('aria-label', 'Sp\u00e9cial \ud83d\ude00 \udc00');
  const raw = p.w.eval(wrap('digest', {}));
  assert.ok(/^[\x20-\x7e]*$/.test(raw), 'ASCII-only');
  const d = JSON.parse(raw); // would throw on a lone surrogate escape only if Rust-style strict; check manually below
  assert.ok(d.text.includes('Caf\u00e9 \u2713 \ud83d\ude00'), d.text);
  assert.ok(!/[\u200b\u202e\u202c\u0007]/.test(d.text), 'invisible/bidi/control characters removed');
  assert.ok(d.text.includes('ab'), 'zero-width space removed without leaving a gap');
  assert.ok(!/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(d.text + d.elements[0].label), 'no lone surrogates');
  assert.ok(!/\\ud[89ab][0-9a-f]{2}(?!\\ud[c-f])/i.test(raw.replace(/\\ud[89ab][0-9a-f]{2}\\ud[c-f][0-9a-f]{2}/gi, '')), 'no unpaired surrogate escapes for serde_json to choke on');
});

test('digest: text is bounded and elements carry bounded strings', () => {
  const p = page('<p>' + 'word '.repeat(5000) + '</p><button>' + 'L'.repeat(1000) + '</button><input placeholder="' + 'P'.repeat(500) + '">');
  const d = p.run('digest');
  assert.ok(d.text.length <= 6000, d.text.length);
  assert.ok(d.elements[0].label.length <= 200);
  assert.ok(d.elements[1].placeholder.length <= 100);
});

test('digest: open shadow roots are pierced (text and elements), and refs resolve inside them', () => {
  const p = page('<div id="host"></div><button id="light">Light</button>');
  const root = p.doc.getElementById('host').attachShadow({ mode: 'open' });
  root.innerHTML = '<p>Shadow paragraph</p><button id="sh">Inside shadow</button>';
  const d = p.run('digest');
  assert.ok(d.text.includes('Shadow paragraph'), d.text);
  const inner = byLabel(d, 'Inside shadow');
  assert.ok(inner, 'shadow button listed');
  let clicked = 0;
  root.getElementById('sh').addEventListener('click', () => { clicked++; });
  assert.strictEqual(p.run('click', { sel: sel(inner.ref_id) }).ok, true);
  assert.strictEqual(clicked, 1, 'a ref inside a shadow root must resolve and click');
});

test('digest: cursor:pointer regions without semantic markup are found once, as "clickable"', () => {
  const p = page('<div id="card" style="cursor:pointer"><span>Add to cart</span><span style="cursor:pointer">inner</span></div><div style="cursor:pointer"></div>');
  const d = p.run('digest');
  const clickable = d.elements.filter((e) => e.role === 'clickable');
  assert.strictEqual(clickable.length, 1, JSON.stringify(d.elements));
  assert.ok(clickable[0].label.startsWith('Add to cart'), clickable[0].label);
});

test('digest: a zero-size custom checkbox is represented by its visible label', () => {
  const p = page('<label data-rect="0,0,120,20"><input type="checkbox" id="c" data-rect="0,0,0,0" style="opacity:0"> Remember me</label>');
  const d = p.run('digest');
  const c = byLabel(d, 'Remember me');
  assert.ok(c, JSON.stringify(d.elements));
  assert.strictEqual(c.role, 'checkbox');
  assert.strictEqual(c.in_viewport, true);
});

test('digest: a page whose script throws still returns a structured error, never an exception', () => {
  const p = page('<button>x</button>');
  const r = p.run('no_such_op');
  assert.strictEqual(r.ok, false);
  assert.strictEqual(r.error, 'unknown_op');
});

// -- actions ---------------------------------------------------------------

function record(el, types) {
  const seen = [];
  for (const t of types) el.addEventListener(t, () => seen.push(t), true);
  return seen;
}

test('click: full pointer/mouse sequence, focus, then click; refs and CSS selectors both work', () => {
  const p = page('<button id="b">Go</button>');
  const d = p.run('digest');
  const b = p.doc.getElementById('b');
  const seen = record(b, ['mouseover', 'mousemove', 'mousedown', 'focus', 'mouseup', 'click']);
  assert.deepStrictEqual(p.run('click', { sel: sel(d.elements[0].ref_id) }), { ok: true });
  assert.deepStrictEqual(seen, ['mouseover', 'mousemove', 'mousedown', 'focus', 'mouseup', 'click']);
  assert.strictEqual(p.doc.activeElement, b);
  assert.deepStrictEqual(p.run('click', { sel: '#b' }), { ok: true });
});

test('click: scrolls an off-screen element into view first; disabled elements are refused', () => {
  const p = page('<button id="far" data-rect="0,5000,100,20">Far</button><button id="d" disabled>D</button>');
  p.run('click', { sel: '#far' });
  assert.strictEqual(p.doc.getElementById('far').__scrolledIntoView, 1);
  assert.deepStrictEqual(p.run('click', { sel: '#d' }), { ok: false, error: 'disabled' });
});

test('click: target=_blank links stay in this tab; clicking an <option> selects it', () => {
  const p = page('<a id="l" href="/x" target="_blank">x</a><select id="s"><option value="1">One</option><option value="2">Two</option></select>');
  p.doc.getElementById('l').addEventListener('click', (e) => e.preventDefault());
  p.run('click', { sel: '#l' });
  assert.strictEqual(p.doc.getElementById('l').getAttribute('target'), '_self');
  const changes = record(p.doc.getElementById('s'), ['change']);
  p.run('click', { sel: 'option[value="2"]' });
  assert.strictEqual(p.doc.getElementById('s').value, '2');
  assert.deepStrictEqual(changes, ['change']);
});

test('type_text: sets the value through the native setter and fires input then change', () => {
  const p = page('<input id="i" value="old"><textarea id="t"></textarea>');
  const i = p.doc.getElementById('i');
  const seen = record(i, ['input', 'change']);
  assert.deepStrictEqual(p.run('type_text', { sel: '#i', text: 'new value' }), { ok: true });
  assert.strictEqual(i.value, 'new value', 'replaces, does not append');
  assert.deepStrictEqual(seen, ['input', 'change']);
  assert.deepStrictEqual(p.run('type_text', { sel: '#t', text: 'multi\nline' }), { ok: true });
  assert.strictEqual(p.doc.getElementById('t').value, 'multi\nline');
  // React tracks the value through the prototype setter: the own-property
  // tracker must not be what is bypassed.
  let tracked = 0;
  const desc = Object.getOwnPropertyDescriptor(p.w.HTMLInputElement.prototype, 'value');
  Object.defineProperty(p.w.HTMLInputElement.prototype, 'value', { get: desc.get, set(v) { tracked++; desc.set.call(this, v); }, configurable: true });
  p.run('type_text', { sel: '#i', text: 'x' });
  assert.strictEqual(tracked, 1, 'the prototype setter must be used');
});

test('type_text: refuses controls that are not text (naming the right action) and read-only fields', () => {
  const p = page('<input type="checkbox" id="c"><select id="s"><option>a</option></select><input id="r" readonly value="x"><div id="d">not editable</div>');
  for (const id of ['c', 's']) {
    const r = p.run('type_text', { sel: '#' + id, text: 'x' });
    assert.strictEqual(r.error, 'not_editable', id);
  }
  assert.strictEqual(p.run('type_text', { sel: '#r', text: 'x' }).error, 'not_editable');
  assert.strictEqual(p.run('type_text', { sel: '#d', text: 'x' }).error, 'not_editable');
  assert.strictEqual(p.doc.getElementById('r').value, 'x');
});

test('type_text: contenteditable falls back to textContent when execCommand is unavailable', () => {
  const p = page('<div id="e" contenteditable="true">old</div>');
  const seen = record(p.doc.getElementById('e'), ['input']);
  assert.deepStrictEqual(p.run('type_text', { sel: '#e', text: 'new' }), { ok: true });
  assert.strictEqual(p.doc.getElementById('e').textContent, 'new');
  assert.ok(seen.length >= 1);
});

test('select_option: value or label, case-insensitive, events, and a helpful error', () => {
  const p = page('<select id="s"><option value="us">United States</option><option value="ca">Canada</option><option value="mx" disabled>Mexico</option></select>');
  const s = p.doc.getElementById('s');
  const seen = record(s, ['input', 'change']);
  for (const [wanted, value] of [['ca', 'ca'], ['United States', 'us'], ['  cANADA ', 'ca'], ['united', 'us']]) {
    assert.deepStrictEqual(p.run('select_option', { sel: '#s', value: wanted }), { ok: true }, wanted);
    assert.strictEqual(s.value, value, wanted);
  }
  assert.strictEqual(seen.length, 8, 'input+change per selection');
  const before = s.value;
  const r = p.run('select_option', { sel: '#s', value: 'Atlantis' });
  assert.strictEqual(r.error, 'no_option');
  assert.ok(r.detail.includes('Atlantis') && r.detail.includes('United States | Canada | Mexico'), r.detail);
  assert.strictEqual(s.value, before, 'nothing changes on a miss');
  assert.strictEqual(p.run('select_option', { sel: '#s', value: 'Mexico' }).error, 'disabled');
  assert.strictEqual(p.run('select_option', { sel: 'body', value: 'x' }).error, 'not_editable');
});

test('fill_form: text, select-by-label and checkbox in one call; reports the failing index', () => {
  const p = page(CHECKOUT);
  const d = p.run('digest');
  const ref = (label) => sel(byLabel(d, label).ref_id);
  const r = p.run('fill_form', {
    fields: [[ref('Email address'), 'grace@example.com'], [ref('Country'), 'mexico'], [ref('I accept the terms'), 'true'], [ref('Newsletter'), 'no']],
  });
  assert.deepStrictEqual(r, { ok: true });
  assert.strictEqual(p.doc.getElementById('email').value, 'grace@example.com');
  assert.strictEqual(p.doc.getElementById('country').value, 'mx');
  assert.strictEqual(p.doc.getElementById('terms').checked, true);
  assert.strictEqual(p.doc.getElementById('news').checked, false);
  const bad = p.run('fill_form', { fields: [['#email', 'a@b.c'], ['@404', 'x'], ['#terms', 'x']] });
  assert.strictEqual(bad.ok, false);
  assert.strictEqual(bad.index, 1, 'the failing field is identified');
  assert.strictEqual(bad.error, 'bad_selector', '"@404" is not CSS: the Rust side normalizes refs before calling');
});

test('set_checked: idempotent (clicks only when the state differs); radios cannot be unchecked', () => {
  const p = page('<input type="checkbox" id="a"><input type="checkbox" id="b" checked><input type="radio" name="r" id="r1" checked><input type="radio" name="r" id="r2"><div role="checkbox" aria-checked="false" id="aria" tabindex="0"></div><button id="btn">x</button>');
  let clicks = 0;
  for (const id of ['a', 'b', 'r2', 'aria']) p.doc.getElementById(id).addEventListener('click', () => { clicks++; });
  assert.deepStrictEqual(p.run('set_checked', { sel: '#a', checked: true }), { ok: true, changed: true });
  assert.strictEqual(p.doc.getElementById('a').checked, true);
  assert.strictEqual(clicks, 1);
  assert.deepStrictEqual(p.run('set_checked', { sel: '#a', checked: true }), { ok: true, changed: false });
  assert.strictEqual(clicks, 1, 'no click when already in the wanted state');
  assert.deepStrictEqual(p.run('set_checked', { sel: '#b', checked: false }), { ok: true, changed: true });
  assert.strictEqual(p.doc.getElementById('b').checked, false);
  assert.deepStrictEqual(p.run('set_checked', { sel: '#r2', checked: true }), { ok: true, changed: true });
  assert.strictEqual(p.doc.getElementById('r1').checked, false);
  const un = p.run('set_checked', { sel: '#r2', checked: false });
  assert.strictEqual(un.error, 'state_unchanged');
  assert.strictEqual(p.run('set_checked', { sel: '#btn', checked: true }).error, 'not_checkable');
  p.run('set_checked', { sel: '#aria', checked: true });
  assert.strictEqual(clicks, 4, 'ARIA checkbox: clicked once (state is the page\'s to update)');
});

test('press_key: Enter submits the enclosing form; Tab moves focus; characters and Backspace edit', () => {
  const p = page('<form id="f"><input id="a" name="a"><input id="b" name="b"><button type="submit">Go</button></form><button id="after">after</button>');
  const submits = [];
  p.doc.getElementById('f').addEventListener('submit', (e) => { e.preventDefault(); submits.push('submit'); });
  const a = p.doc.getElementById('a');
  const keys = [];
  a.addEventListener('keydown', (e) => keys.push(e.key + ':' + e.keyCode));
  assert.deepStrictEqual(p.run('press_key', { sel: '#a', key: 'Enter' }), { ok: true });
  assert.deepStrictEqual(submits, ['submit']);
  assert.deepStrictEqual(keys, ['Enter:13']);
  // Tab from #a goes to #b
  p.run('press_key', { sel: '#a', key: 'Tab' });
  assert.strictEqual(p.doc.activeElement.id, 'b');
  // typing characters into the focused element (no selector)
  p.run('press_key', { key: 'h' });
  p.run('press_key', { key: 'i' });
  assert.strictEqual(p.doc.getElementById('b').value, 'hi');
  p.run('press_key', { key: 'Backspace' });
  assert.strictEqual(p.doc.getElementById('b').value, 'h');
  // shift+Tab goes back
  p.run('press_key', { key: 'Shift+Tab' });
  assert.strictEqual(p.doc.activeElement.id, 'a');
  assert.strictEqual(p.run('press_key', { sel: '#a', key: '' }).error, 'bad_key');
});

test('press_key: Enter/Space activate buttons; ArrowDown moves a <select>; a prevented keydown suppresses the default', () => {
  const p = page('<button id="b">B</button><select id="s"><option>1</option><option>2</option></select><input id="i">');
  let clicked = 0;
  p.doc.getElementById('b').addEventListener('click', () => clicked++);
  p.run('press_key', { sel: '#b', key: 'Enter' });
  p.run('press_key', { sel: '#b', key: 'Space' });
  assert.strictEqual(clicked, 2);
  p.run('press_key', { sel: '#s', key: 'ArrowDown' });
  assert.strictEqual(p.doc.getElementById('s').selectedIndex, 1);
  p.doc.getElementById('i').addEventListener('keydown', (e) => e.preventDefault());
  p.run('press_key', { sel: '#i', key: 'x' });
  assert.strictEqual(p.doc.getElementById('i').value, '', 'a page that prevents keydown gets no text');
});

test('submit_form: requestSubmit runs validation; an invalid form is reported, a valid one submits', () => {
  const p = page('<form id="f"><label for="e">Email</label><input id="e" type="email" required><button>Go</button></form>');
  const submits = [];
  p.doc.getElementById('f').addEventListener('submit', (ev) => { ev.preventDefault(); submits.push(1); });
  const bad = p.run('submit_form', {});
  assert.strictEqual(bad.error, 'invalid_form');
  assert.ok(bad.detail.includes('Email'), bad.detail);
  assert.strictEqual(submits.length, 0);
  p.run('type_text', { sel: '#e', text: 'a@b.co' });
  assert.deepStrictEqual(p.run('submit_form', { sel: '#e' }), { ok: true });
  assert.strictEqual(submits.length, 1);
  assert.strictEqual(page('<p>none</p>').run('submit_form', {}).error, 'no_form');
});

test('hover and scroll_to', () => {
  const p = page('<div id="outer"><button id="b" data-rect="0,3000,100,20">Far</button></div>');
  const seen = record(p.doc.getElementById('b'), ['mouseover', 'mouseenter', 'mousemove']);
  assert.deepStrictEqual(p.run('hover', { sel: '#b' }), { ok: true });
  assert.deepStrictEqual(seen, ['mouseover', 'mouseenter', 'mousemove']);
  const r = p.run('scroll_to', { sel: '#b' });
  assert.strictEqual(r.ok, true);
  assert.ok(p.doc.getElementById('b').__scrolledIntoView >= 1);
  assert.deepStrictEqual(Object.keys(r.scroll).sort(), ['max_y', 'viewport_height', 'y']);
});

test('find_text: case-insensitive count and [bracketed] snippets over the whole page, not just the digest', () => {
  const p = page('<p>Refund policy: refunds within 30 days. REFUND requests need a receipt.</p><div style="display:none">refund hidden</div><p>' + 'filler '.repeat(3000) + ' the last Refund</p>');
  const r = p.run('find_text', { text: 'refund' });
  assert.strictEqual(r.ok, true);
  assert.strictEqual(r.count, 4, 'Refund, refunds, REFUND, and the one past the 6000-char digest cut-off; hidden text not counted');
  assert.strictEqual(r.snippets.length, 4);
  assert.ok(r.snippets[0].startsWith('[Refund] policy'), r.snippets[0]);
  assert.ok(r.snippets[3].includes('[Refund]'));
  const none = p.run('find_text', { text: 'zzz' });
  assert.strictEqual(none.count, 0);
  assert.deepStrictEqual(none.snippets, []);
  assert.strictEqual(p.run('find_text', { text: '   ' }).count, 0);
});

test('extract_links: absolute, visible, deduplicated, scoped by selector', () => {
  const p = page('<nav id="n"><a href="/a">A</a><a href="/a">A</a><a href="/b">B</a><a href="/h" style="display:none">H</a></nav><a href="https://x.example/c">C</a>');
  const all = p.run('extract_links', {});
  assert.deepStrictEqual(all.links, [
    { text: 'A', href: 'https://shop.example/a' },
    { text: 'B', href: 'https://shop.example/b' },
    { text: 'C', href: 'https://x.example/c' },
  ]);
  assert.strictEqual(p.run('extract_links', { sel: '#n' }).links.length, 2);
  assert.strictEqual(p.run('extract_links', { sel: '#nope' }).error, 'not_found');
});

test('query stamps refs and returns them as selectors; read_text of inputs returns the value, never a password', () => {
  const p = page('<ul><li class="i">One</li><li class="i">Two</li></ul><input id="t" value="typed"><input id="pw" type="password" value="hunter2">');
  const q = p.run('query', { sel: '.i' });
  assert.strictEqual(q.items.length, 2);
  assert.ok(/^@\d+$/.test(q.items[0].selector), q.items[0].selector);
  assert.strictEqual(q.items[1].text, 'Two');
  const again = p.run('query', { sel: '.i' });
  assert.deepStrictEqual(again.items.map((i) => i.selector), q.items.map((i) => i.selector), 'stable refs');
  const n = q.items[0].selector.slice(1);
  assert.strictEqual(p.run('read_text', { sel: sel(n) }).text, 'One');
  assert.strictEqual(p.run('read_text', { sel: '#t' }).text, 'typed');
  assert.strictEqual(p.run('read_text', { sel: '#pw' }).text, '');
  assert.strictEqual(p.run('read_text', { sel: '#nope' }).error, 'not_found');
  assert.strictEqual(p.run('query', { sel: 'li[' }).error, 'bad_selector');
});

test('a hostile page cannot smuggle a password through a lookalike field or attribute tricks', () => {
  const p = page('<input type="text" name="password" value="looks-like-a-secret"><input type="text" name="user_passwd" value="p2"><input type="tel" id="otp" autocomplete="one-time-code" value="123456">');
  const d = p.run('digest');
  assert.ok(d.elements.every((e) => e.sensitive && e.value === null), JSON.stringify(d.elements));
});

// -- runner ----------------------------------------------------------------

function checkDump(dir) {
  // The Rust test wrote one file per op; each must be exactly what `wrap` builds.
  let checked = 0;
  for (const f of fs.readdirSync(dir).filter((n) => n.endsWith('.js'))) {
    const text = fs.readFileSync(path.join(dir, f), 'utf8');
    const op = f.replace(/\.js$/, '');
    const head = '(function () { "use strict"; var __op = ' + jsonLiteral(op) + '; var __args = ';
    assert.ok(text.startsWith(head), f + ': wrapper head differs from run.js wrap()');
    assert.ok(text.endsWith('\n' + PAGE_OPS + '\n})()'), f + ': wrapper tail/body differs from run.js wrap()');
    checked++;
  }
  assert.ok(checked > 0, 'no dumped scripts found in ' + dir);
  console.log('ok   dump: ' + checked + ' Rust-assembled scripts match this harness wrapper');
}

let failed = 0;
const dumpIdx = process.argv.indexOf('--check-dump');
if (dumpIdx >= 0) {
  try { checkDump(process.argv[dumpIdx + 1]); } catch (e) { failed++; console.log('FAIL dump: ' + e.message); }
}
const only = process.argv.find((a) => a.startsWith('--only='));
for (const t of tests) {
  if (only && !t.name.includes(only.slice(7))) continue;
  try {
    t.fn();
    console.log('ok   ' + t.name);
  } catch (e) {
    failed++;
    console.log('FAIL ' + t.name + '\n     ' + String(e && e.stack ? e.stack : e).split('\n').slice(0, 6).join('\n     '));
  }
}
console.log('\n' + (tests.length - failed) + ' passed, ' + failed + ' failed');
process.exit(failed ? 1 : 0);
