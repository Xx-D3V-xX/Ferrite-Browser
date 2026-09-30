// Ferrite page operations - the one script every Servo-backed DOM action runs.
//
// This file is the BODY of a function. `ferrite-engine-servo` wraps it as
//
//   (function () { "use strict"; var __op = "<name>"; var __args = <json>; <this file> })()
//
// so nothing here leaks into the page's global scope, and the final `return`
// hands back a JSON string (ASCII only: see `out`) that Rust parses. Every op
// answers `{ok: true, ...}` or `{ok: false, error: <code>, detail?: <text>}`;
// nothing throws past this file.
//
// Everything a page can influence is treated as untrusted data: every string
// is length-bounded and stripped of control / invisible / bidi characters
// (`clean`), passwords are never read, and Rust re-bounds the result
// (`PageDigest::sanitized`) because this script runs inside a page that could
// shadow the built-ins it uses.
//
// Refs: `data-ferrite-ref="N"` is stamped on every element handed to the model.
// A re-digest reuses an existing stamp and gives only new elements max+1, so a
// ref stays valid for as long as the node does; navigation drops them all.

var REF = 'data-ferrite-ref';
var T0 = Date.now();
var VIEW_W = window.innerWidth || document.documentElement.clientWidth || 0;
var VIEW_H = window.innerHeight || document.documentElement.clientHeight || 0;

var MAX_ELEMENTS = 150;
var MAX_TEXT = 6000;
var MAX_VISITED = 25000;
var TIME_BUDGET_MS = 1000;

// -- output ----------------------------------------------------------------

// ASCII-only JSON: the Rust side reads it back through `Debug`-formatted text
// (`execute_js`), where non-ASCII would need escapes JSON does not have.
function out(o) {
  return JSON.stringify(o).replace(/[\u007f-\uffff]/g, function (c) {
    return '\\u' + ('0000' + c.charCodeAt(0).toString(16)).slice(-4);
  });
}

var SURROGATES = /[\ud800-\udbff][\udc00-\udfff]|[\ud800-\udfff]/g;
var INVISIBLES = /[\u00ad\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u2069\ufeff]/g;
var CONTROLS = /[\u0000-\u001f\u007f-\u009f]/g;

// Whitespace-collapsed, control/invisible-free, well-formed, bounded.
function clean(s, max) {
  if (s === null || s === undefined) return '';
  s = String(s);
  if (s.length > max * 4 + 64) s = s.slice(0, max * 4 + 64);
  s = s.replace(SURROGATES, function (m) { return m.length === 2 ? m : ''; })
    .replace(INVISIBLES, '')
    .replace(CONTROLS, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (s.length > max) {
    s = s.slice(0, max - 1).replace(/[\ud800-\udbff]$/, '') + '\u2026';
  }
  return s;
}

// Like `clean` but keeps line structure (for read_text of tables/lists/code).
function cleanBlock(s, max) {
  if (s === null || s === undefined) return '';
  s = String(s);
  if (s.length > max * 2 + 64) s = s.slice(0, max * 2 + 64);
  s = s.replace(SURROGATES, function (m) { return m.length === 2 ? m : ''; })
    .replace(INVISIBLES, '')
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, ' ')
    .replace(/\r\n?/g, '\n')
    .replace(/[ \t\u00a0]+/g, ' ')
    .replace(/ ?\n ?/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  if (s.length > max) {
    s = s.slice(0, max - 1).replace(/[\ud800-\udbff]$/, '') + '\u2026';
  }
  return s;
}

function attr(el, name) {
  var v = el.getAttribute(name);
  return v === null || v === undefined ? '' : v;
}

function lc(s) { return String(s === null || s === undefined ? '' : s).toLowerCase(); }

function styleOf(el) {
  try { return window.getComputedStyle(el); } catch (e) { return null; }
}

// -- refs ------------------------------------------------------------------

var maxRef = 0;
var usedRefs = {};
var refsScanned = false;

function scanRefs() {
  if (refsScanned) return;
  refsScanned = true;
  var nodes = document.querySelectorAll('[' + REF + ']');
  for (var i = 0; i < nodes.length; i++) {
    var n = parseInt(nodes[i].getAttribute(REF), 10);
    if (n > maxRef) maxRef = n;
  }
}

// The ref of `el`: its existing stamp if that is unique in this run, else the
// next number. (A cloned node carries its original's stamp - the clone gets a
// fresh one.)
function refOf(el) {
  scanRefs();
  var cur = parseInt(el.getAttribute(REF), 10);
  if (cur > 0 && !usedRefs[cur]) {
    usedRefs[cur] = true;
    return cur;
  }
  maxRef += 1;
  el.setAttribute(REF, String(maxRef));
  usedRefs[maxRef] = true;
  return maxRef;
}

// -- element lookup (light DOM first, then open shadow roots) --------------

function gatherShadowRoots(root, roots, depth) {
  if (depth > 8) return;
  var all = root.querySelectorAll('*');
  var n = Math.min(all.length, MAX_VISITED);
  for (var i = 0; i < n; i++) {
    var sr = all[i].shadowRoot;
    if (sr) {
      roots.push(sr);
      gatherShadowRoots(sr, roots, depth + 1);
    }
  }
}

// undefined = not a valid selector; null = no such element; else the element.
function find(sel) {
  var el;
  try { el = document.querySelector(sel); } catch (e) { return undefined; }
  if (el) return el;
  var roots = [];
  gatherShadowRoots(document, roots, 0);
  for (var i = 0; i < roots.length; i++) {
    try { el = roots[i].querySelector(sel); } catch (e2) { el = null; }
    if (el) return el;
  }
  return null;
}

function findAll(sel, limit) {
  var found = [];
  var all;
  try { all = document.querySelectorAll(sel); } catch (e) { return undefined; }
  for (var i = 0; i < all.length && found.length < limit; i++) found.push(all[i]);
  if (found.length < limit) {
    var roots = [];
    gatherShadowRoots(document, roots, 0);
    for (var r = 0; r < roots.length && found.length < limit; r++) {
      var inner;
      try { inner = roots[r].querySelectorAll(sel); } catch (e2) { continue; }
      for (var j = 0; j < inner.length && found.length < limit; j++) found.push(inner[j]);
    }
  }
  return found;
}

// -- element facts ---------------------------------------------------------

var TEXT_INPUT_BLOCKED = { checkbox: 1, radio: 1, submit: 1, button: 1, reset: 1, image: 1, file: 1, hidden: 1, range: 1, color: 1 };
var BUTTON_INPUTS = { submit: 1, button: 1, reset: 1, image: 1, file: 1 };
var EDITABLE_ROLES = { textbox: 1, searchbox: 1, combobox: 1, spinbutton: 1, listbox: 1 };

function tagOf(el) { return String(el.tagName || '').toLowerCase(); }
function typeOf(el) { return tagOf(el) === 'input' ? lc(el.type || attr(el, 'type') || 'text') : ''; }

function isContentEditable(el) {
  if (el.isContentEditable === true) return true;
  var ce = el.getAttribute && el.getAttribute('contenteditable');
  return ce !== null && ce !== undefined && lc(ce) !== 'false';
}

function roleOf(el) {
  var explicit = lc(attr(el, 'role')).split(/\s+/)[0];
  if (explicit && explicit !== 'presentation' && explicit !== 'none' && explicit !== 'generic') return explicit;
  var tag = tagOf(el);
  var type = typeOf(el);
  if (tag === 'a') return el.hasAttribute('href') ? 'link' : null;
  if (tag === 'button' || tag === 'summary') return 'button';
  if (tag === 'select') return (el.multiple || el.size > 1) ? 'listbox' : 'combobox';
  if (tag === 'textarea') return 'textbox';
  if (tag === 'input') {
    if (type === 'checkbox') return 'checkbox';
    if (type === 'radio') return 'radio';
    if (BUTTON_INPUTS[type]) return 'button';
    if (type === 'range') return 'slider';
    if (type === 'search') return 'searchbox';
    if (type === 'number') return 'spinbutton';
    return 'textbox';
  }
  if (tag === 'option') return 'option';
  if (isContentEditable(el)) return 'textbox';
  return null;
}

function isSensitive(el) {
  var tag = tagOf(el);
  if (tag !== 'input' && tag !== 'textarea') return false;
  if (typeOf(el) === 'password') return true;
  if (/(^|\s)(current-password|new-password|one-time-code|cc-number|cc-csc|cc-exp|cc-exp-month|cc-exp-year)(\s|$)/.test(lc(attr(el, 'autocomplete')))) return true;
  return /passw|passwd|cvv|cvc|card.?num|\bssn\b|social.?sec|\botp\b/i.test(attr(el, 'name') + ' ' + attr(el, 'id'));
}

function isDisabled(el) {
  if (el.disabled === true) return true;
  if (lc(attr(el, 'aria-disabled')) === 'true') return true;
  var fs = el.closest ? el.closest('fieldset[disabled]') : null;
  if (fs) {
    var legend = el.closest('legend');
    if (!(legend && legend.parentElement === fs)) return true;
  }
  return false;
}

function labelText(label, max) {
  // The label's own text, without the text of controls nested inside it.
  var parts = [];
  var total = 0;
  (function walk(node, depth) {
    if (depth > 12 || total > max * 2) return;
    for (var c = node.firstChild; c; c = c.nextSibling) {
      if (c.nodeType === 3) {
        parts.push(c.nodeValue);
        total += c.nodeValue.length;
      } else if (c.nodeType === 1) {
        var t = tagOf(c);
        if (t === 'input' || t === 'select' || t === 'textarea' || t === 'button' || t === 'script' || t === 'style') continue;
        walk(c, depth + 1);
      }
    }
  })(label, 0);
  return clean(parts.join(' '), max);
}

function associatedLabel(el) {
  var tag = tagOf(el);
  if (tag !== 'input' && tag !== 'select' && tag !== 'textarea' && tag !== 'meter' && tag !== 'progress') return '';
  var texts = [];
  var labels = el.labels;
  if (labels && labels.length) {
    for (var i = 0; i < labels.length && i < 3; i++) texts.push(labelText(labels[i], 200));
  } else {
    var wrap = el.closest ? el.closest('label') : null;
    if (wrap) texts.push(labelText(wrap, 200));
  }
  return clean(texts.join(' '), 200);
}

// Accessible name: aria-label > aria-labelledby > associated <label> >
// alt/title > visible text > (button-like) value > placeholder > name.
function nameOf(el, role) {
  var tag = tagOf(el);
  var s = clean(attr(el, 'aria-label'), 200);
  if (s) return s;
  var ids = attr(el, 'aria-labelledby');
  if (ids) {
    var parts = [];
    var list = ids.split(/\s+/);
    for (var i = 0; i < list.length && i < 6; i++) {
      var n = list[i] ? document.getElementById(list[i]) : null;
      if (n) parts.push(clean(n.textContent, 200));
    }
    s = clean(parts.join(' '), 200);
    if (s) return s;
  }
  s = associatedLabel(el);
  if (s) return s;
  s = clean(attr(el, 'alt'), 200) || clean(attr(el, 'title'), 200);
  if (s) return s;
  var nativeField = tag === 'input' || tag === 'textarea' || tag === 'select';
  if (!nativeField && !EDITABLE_ROLES[role]) {
    s = clean(el.textContent, 200);
    if (s) return s;
    var inner = el.querySelector ? el.querySelector('img[alt], svg title, [aria-label]') : null;
    if (inner) {
      s = clean(inner.getAttribute('alt') || inner.getAttribute('aria-label') || inner.textContent, 200);
      if (s) return s;
    }
  }
  if (tag === 'input' && BUTTON_INPUTS[typeOf(el)]) {
    s = clean(el.value, 200);
    if (s) return s;
  }
  s = clean(attr(el, 'placeholder') || attr(el, 'aria-placeholder') || attr(el, 'data-placeholder'), 200);
  if (s) return s;
  return clean(attr(el, 'name'), 200);
}

function selectedLabel(sel) {
  var picked = [];
  var opts = sel.options || [];
  for (var i = 0; i < opts.length; i++) {
    if (opts[i].selected) picked.push(clean(opts[i].label || opts[i].text || opts[i].value, 100));
  }
  return picked.join(', ');
}

function valueOf(el, role) {
  if (isSensitive(el)) return null;
  var tag = tagOf(el);
  if (tag === 'select') return selectedLabel(el);
  if (tag === 'input') {
    return TEXT_INPUT_BLOCKED[typeOf(el)] ? null : clean(el.value, 300);
  }
  if (tag === 'textarea') return clean(el.value, 300);
  if (isContentEditable(el)) return clean(el.textContent, 300);
  if (role === 'slider' || role === 'spinbutton') {
    var v = attr(el, 'aria-valuenow');
    return v ? clean(v, 40) : null;
  }
  return null;
}

// null = the element has no checked state.
function readChecked(el, role) {
  var tag = tagOf(el);
  var type = typeOf(el);
  if (tag === 'input' && (type === 'checkbox' || type === 'radio')) return !!el.checked;
  if (role === 'checkbox' || role === 'radio' || role === 'switch' || role === 'menuitemcheckbox' || role === 'menuitemradio') {
    var ac = lc(attr(el, 'aria-checked'));
    if (ac === 'true') return true;
    if (ac === 'mixed') return null;
    return false;
  }
  return null;
}

function readSelected(el, role) {
  if (tagOf(el) === 'option') return !!el.selected;
  if (role === 'option' || role === 'tab' || role === 'treeitem') return lc(attr(el, 'aria-selected')) === 'true';
  return null;
}

function readExpanded(el) {
  var ae = lc(attr(el, 'aria-expanded'));
  if (ae === 'true') return true;
  if (ae === 'false') return false;
  if (tagOf(el) === 'summary' && el.parentElement && tagOf(el.parentElement) === 'details') return !!el.parentElement.open;
  return null;
}

function formIndexOf(el) {
  var f = el.form || (el.closest ? el.closest('form') : null);
  if (!f) return null;
  var i = Array.prototype.indexOf.call(document.forms, f);
  return i >= 0 ? i : null;
}

function optionsOf(el) {
  if (tagOf(el) !== 'select') return [];
  var res = [];
  var opts = el.options || [];
  for (var i = 0; i < opts.length && i < 50; i++) {
    res.push(clean(opts[i].label || opts[i].text || opts[i].value, 100));
  }
  return res;
}

// -- geometry --------------------------------------------------------------

function sizedRect(el) {
  var r = el.getBoundingClientRect();
  return (r.width > 0 && r.height > 0) ? r : null;
}

// The rect the user sees for `el`. A zero-size native checkbox/radio (a common
// custom-styled control) is represented by its visible <label>.
function visibleRect(el, style) {
  if (style && style.visibility === 'hidden') return null;
  var r = sizedRect(el);
  if (r) return r;
  var type = typeOf(el);
  if (tagOf(el) === 'input' && (type === 'checkbox' || type === 'radio') && el.labels) {
    for (var i = 0; i < el.labels.length; i++) {
      var lr = sizedRect(el.labels[i]);
      if (lr) return lr;
    }
  }
  return null;
}

function inViewport(r) {
  if (!VIEW_H || !VIEW_W) return true;
  return r.bottom > 0 && r.top < VIEW_H && r.right > 0 && r.left < VIEW_W;
}

// Off the top/left edge of the document itself: unreachable by scrolling
// (skip-links, sr-only text) and only noise to the model.
function unreachable(r) {
  var sx = window.pageXOffset || 0;
  var sy = window.pageYOffset || 0;
  return (r.bottom + sy) <= 0 || (r.right + sx) <= 0;
}

function distanceFromViewport(r) {
  if (inViewport(r)) return 0;
  if (r.top >= VIEW_H) return r.top - VIEW_H;
  if (r.bottom <= 0) return -r.bottom;
  return 1e6;
}

function scrollerOf() {
  var se = document.scrollingElement || document.documentElement;
  if (se.scrollHeight - se.clientHeight > 1) return se;
  if (typeof document.elementFromPoint === 'function' && VIEW_W && VIEW_H) {
    var n = document.elementFromPoint(VIEW_W / 2, VIEW_H / 2);
    for (; n && n !== document.body && n !== document.documentElement; n = n.parentElement) {
      if (n.scrollHeight - n.clientHeight > 1) {
        var st = styleOf(n);
        var oy = st ? st.overflowY : '';
        if (oy === 'auto' || oy === 'scroll' || oy === 'overlay') return n;
      }
    }
  }
  return se;
}

function scrollState() {
  var se = document.scrollingElement || document.documentElement;
  var el = scrollerOf();
  var isDoc = el === se;
  var y = isDoc ? (window.pageYOffset || se.scrollTop || 0) : (el.scrollTop || 0);
  var viewport = isDoc ? VIEW_H : (el.clientHeight || 0);
  return {
    y: Math.round(y),
    max_y: Math.max(0, Math.round((el.scrollHeight || 0) - (el.clientHeight || 0))),
    viewport_height: Math.round(viewport)
  };
}

// Brings `el` fully into view if any of it is outside, centred.
function reveal(el) {
  var r = el.getBoundingClientRect();
  var inside = r.top >= 0 && r.bottom <= VIEW_H && r.left >= 0 && r.right <= VIEW_W;
  if (VIEW_H && VIEW_W && !inside) {
    try {
      el.scrollIntoView({ block: 'center', inline: 'nearest' });
    } catch (e) {
      try { el.scrollIntoView(true); } catch (e2) { /* nothing more to try */ }
    }
  }
}

// -- page walk: text + interactive candidates in a single pass -------------

var SKIP_TAGS = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEMPLATE: 1, HEAD: 1, META: 1, LINK: 1, TITLE: 1, IFRAME: 1, OBJECT: 1, EMBED: 1, CANVAS: 1 };
var NO_TEXT_TAGS = { SELECT: 1, TEXTAREA: 1, OPTION: 1, OPTGROUP: 1 };
var CLICKABLE_TAGS = { DIV: 1, SPAN: 1, LI: 1, IMG: 1, SVG: 1, TD: 1, TR: 1, ARTICLE: 1, SECTION: 1, H1: 1, H2: 1, H3: 1, H4: 1, P: 1, I: 1, B: 1, FIGURE: 1 };
var CANDIDATE_SELECTOR = [
  'a[href]', 'button', 'input:not([type=hidden])', 'select', 'textarea', 'summary',
  '[role=button]', '[role=link]', '[role=checkbox]', '[role=radio]', '[role=tab]',
  '[role=menuitem]', '[role=menuitemcheckbox]', '[role=menuitemradio]', '[role=switch]',
  '[role=combobox]', '[role=textbox]', '[role=searchbox]', '[role=option]', '[role=slider]',
  '[contenteditable]:not([contenteditable=false])', '[onclick]'
].join(',');

// Walks the visible document once. Collects visible text (up to `textMax`
// characters) and, when `wantCandidates`, the interactive elements with their
// rects. Prunes display:none subtrees, ignores visibility:hidden text, pierces
// open shadow roots, and stops at a node/time budget (`truncated`).
function walkPage(textMax, wantCandidates) {
  var res = { text: '', cands: [], truncated: false };
  var parts = [];
  var total = 0;
  var visited = 0;

  function addText(t) {
    if (total >= textMax || !t) return;
    parts.push(t);
    total += t.length;
  }

  function visitChildren(parent, vis, parentPointer, inCand, depth) {
    for (var c = parent.firstChild; c; c = c.nextSibling) {
      if (res.truncated) return;
      if (c.nodeType === 3) {
        if (vis && total < textMax) addText(c.nodeValue);
        continue;
      }
      if (c.nodeType !== 1) continue;
      visitElement(c, parentPointer, inCand, depth);
    }
  }

  function visitElement(el, parentPointer, inCand, depth) {
    if (depth > 400) return;
    if (++visited > MAX_VISITED || (visited % 64 === 0 && Date.now() - T0 > TIME_BUDGET_MS)) {
      res.truncated = true;
      return;
    }
    var tag = el.tagName.toUpperCase();
    if (SKIP_TAGS[tag]) return;
    var style = styleOf(el);
    if (style && style.display === 'none') return;
    var vis = !(style && style.visibility === 'hidden');
    var pointer = !!(style && style.cursor === 'pointer');
    var block = !!style && style.display !== 'inline' && style.display !== 'contents';

    var isCand = false;
    if (wantCandidates && res.cands.length < 1200 && vis) {
      var matched = false;
      try { matched = el.matches(CANDIDATE_SELECTOR); } catch (e) { matched = false; }
      var role = null;
      var kind = '';
      if (matched) {
        role = roleOf(el);
        // Spec: [onclick] only counts with a pointer cursor.
        if (role === null && el.hasAttribute('onclick')) role = pointer ? 'button' : null;
        kind = 'native';
      } else if (pointer && !parentPointer && !inCand && CLICKABLE_TAGS[tag]) {
        // cursor:pointer on the outermost element of a clickable region: the
        // shape of a JS-driven "button" that has no semantic markup.
        role = 'clickable';
        kind = 'heuristic';
      }
      if (role) {
        var rect = visibleRect(el, style);
        if (rect && !unreachable(rect)) {
          if (kind === 'heuristic' && !clean(el.textContent, 40) && !attr(el, 'aria-label') && !attr(el, 'title') && !el.querySelector('img[alt]')) {
            role = null; // an empty clickable box is not something to name
          }
          if (role) {
            res.cands.push({ el: el, role: role, rect: rect, order: res.cands.length });
            isCand = true;
          }
        }
      }
    }

    if (block) addText(' ');
    if (!NO_TEXT_TAGS[tag]) {
      var sr = el.shadowRoot;
      if (sr) visitChildren(sr, vis, pointer, inCand || isCand, depth + 1);
      visitChildren(el, vis, pointer, inCand || isCand, depth + 1);
    }
    if (block) addText(' ');
  }

  try {
    visitChildren(document.documentElement || document, true, false, false, 0);
  } catch (e) {
    res.truncated = true;
  }
  res.text = parts.join('');
  return res;
}

// -- ops -------------------------------------------------------------------

var OPS = {};

function lookup(sel) {
  var el = find(sel);
  if (el === undefined) return { fail: { ok: false, error: 'bad_selector' } };
  if (!el) return { fail: { ok: false, error: 'not_found' } };
  return { el: el };
}

OPS.digest = function () {
  var walk = walkPage(MAX_TEXT, true);
  var cands = walk.cands;
  var truncated = walk.truncated;
  if (cands.length > MAX_ELEMENTS) {
    truncated = true;
    var order = cands.slice().sort(function (a, b) {
      var d = distanceFromViewport(a.rect) - distanceFromViewport(b.rect);
      return d !== 0 ? d : a.order - b.order;
    }).slice(0, MAX_ELEMENTS);
    order.sort(function (a, b) { return a.order - b.order; });
    cands = order;
  }
  var elements = [];
  for (var i = 0; i < cands.length; i++) {
    var c = cands[i];
    var el = c.el;
    var tag = tagOf(el);
    var type = typeOf(el);
    var sensitive = isSensitive(el);
    var inputType = null;
    if (tag === 'input') inputType = (type === 'text' && c.role === 'textbox') ? null : type;
    else if (tag === 'textarea') inputType = 'textarea';
    var ph = clean(attr(el, 'placeholder') || attr(el, 'aria-placeholder'), 100);
    var href = null;
    if (tag === 'a' && el.href) href = clean(String(el.href), 500);
    elements.push({
      ref_id: refOf(el),
      role: c.role,
      label: nameOf(el, c.role),
      input_type: inputType,
      value: sensitive ? null : valueOf(el, c.role),
      placeholder: ph || null,
      href: href,
      checked: readChecked(el, c.role),
      selected: readSelected(el, c.role),
      expanded: readExpanded(el),
      disabled: isDisabled(el),
      in_viewport: inViewport(c.rect),
      sensitive: sensitive,
      form: formIndexOf(el),
      options: optionsOf(el)
    });
  }
  return {
    url: clean(location.href, 1000),
    title: clean(document.title, 200),
    text: clean(walk.text, MAX_TEXT),
    scroll: scrollState(),
    elements: elements,
    elements_truncated: truncated
  };
};

OPS.query = function (a) {
  var found = findAll(a.sel, 100);
  if (found === undefined) return { ok: false, error: 'bad_selector' };
  var items = [];
  for (var i = 0; i < found.length; i++) {
    var el = found[i];
    var text = clean(el.textContent, 200);
    items.push({
      selector: '@' + refOf(el),
      role: clean(attr(el, 'role') || tagOf(el), 40),
      text: text ? text : null
    });
  }
  return { ok: true, items: items };
};

OPS.read_text = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  var el = l.el;
  var tag = tagOf(el);
  var text;
  if (isSensitive(el)) {
    text = '';
  } else if (tag === 'input' || tag === 'textarea') {
    text = el.value;
  } else if (tag === 'select') {
    text = selectedLabel(el);
  } else {
    text = (typeof el.innerText === 'string' && el.innerText) ? el.innerText : el.textContent;
  }
  return { ok: true, text: cleanBlock(text, 7000) };
};

// Dispatches a synthetic event; page handlers may throw, we never do.
function fire(el, type, Ctor, init) {
  try {
    el.dispatchEvent(new Ctor(type, init));
  } catch (e) { /* the page's handler failed; the action still happened */ }
}

function pointerInit(el, buttons) {
  var r = el.getBoundingClientRect();
  var x = r.left + r.width / 2;
  var y = r.top + r.height / 2;
  return {
    bubbles: true, cancelable: true, composed: true, view: window,
    clientX: x, clientY: y, screenX: x, screenY: y, button: 0, buttons: buttons
  };
}

function pointerEvents(el, names, buttons) {
  var init = pointerInit(el, buttons);
  for (var i = 0; i < names.length; i++) {
    var n = names[i];
    if (n.indexOf('pointer') === 0) {
      if (typeof PointerEvent === 'function') {
        var pi = {};
        for (var k in init) pi[k] = init[k];
        pi.pointerId = 1;
        pi.pointerType = 'mouse';
        pi.isPrimary = true;
        fire(el, n, PointerEvent, pi);
      }
    } else {
      fire(el, n, MouseEvent, init);
    }
  }
}

// What a real click does, in order - so handlers listening for pointer/mouse
// events (not just `click`) and focus-dependent widgets behave - ending in the
// element's own click(), which also performs its activation behaviour.
function realClick(el) {
  reveal(el);
  var link = el.closest ? el.closest('a[target]') : null;
  if (link && !/^(_self|_top|_parent)?$/i.test(attr(link, 'target'))) {
    // This embedder has no second window: keep the navigation in this tab.
    link.setAttribute('target', '_self');
  }
  if (tagOf(el) === 'option') {
    var owner = el.closest ? el.closest('select') : null;
    if (owner) {
      el.selected = true;
      fire(owner, 'input', Event, { bubbles: true });
      fire(owner, 'change', Event, { bubbles: true });
      return;
    }
  }
  pointerEvents(el, ['pointerover', 'mouseover', 'pointermove', 'mousemove', 'pointerdown', 'mousedown'], 1);
  try { if (typeof el.focus === 'function') el.focus({ preventScroll: true }); } catch (e) { /* not focusable */ }
  pointerEvents(el, ['pointerup', 'mouseup'], 0);
  el.click();
}

OPS.click = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  if (isDisabled(l.el)) return { ok: false, error: 'disabled' };
  realClick(l.el);
  return { ok: true };
};

OPS.hover = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  var el = l.el;
  reveal(el);
  var chain = [];
  for (var n = el; n && n.nodeType === 1; n = n.parentElement) chain.push(n);
  pointerEvents(el, ['pointerover', 'mouseover'], 0);
  for (var i = chain.length - 1; i >= 0; i--) {
    pointerEvents(chain[i], ['pointerenter', 'mouseenter'], 0);
  }
  pointerEvents(el, ['pointermove', 'mousemove'], 0);
  return { ok: true };
};

OPS.scroll_to = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  try {
    l.el.scrollIntoView({ block: 'center', inline: 'nearest' });
  } catch (e) {
    try { l.el.scrollIntoView(true); } catch (e2) { /* nothing more to try */ }
  }
  return { ok: true, scroll: scrollState() };
};

function nativeSetValue(el, value) {
  var proto = tagOf(el) === 'textarea' ? window.HTMLTextAreaElement.prototype : window.HTMLInputElement.prototype;
  var desc = Object.getOwnPropertyDescriptor(proto, 'value');
  // The prototype setter, so frameworks that track the value (React) see it.
  if (desc && desc.set) desc.set.call(el, value); else el.value = value;
}

function inputEvents(el, data) {
  var init = { bubbles: true, cancelable: false, composed: true, inputType: 'insertText', data: data };
  if (typeof InputEvent === 'function') fire(el, 'input', InputEvent, init);
  else fire(el, 'input', Event, { bubbles: true });
  fire(el, 'change', Event, { bubbles: true });
}

function chooseOption(el, wanted) {
  var opts = el.options || [];
  var want = String(wanted);
  var wantLc = lc(want).trim();
  var idx = -1;
  var i;
  for (i = 0; i < opts.length && idx < 0; i++) if (opts[i].value === want) idx = i;
  for (i = 0; i < opts.length && idx < 0; i++) if (lc(opts[i].label || opts[i].text).trim() === wantLc) idx = i;
  for (i = 0; i < opts.length && idx < 0; i++) if (wantLc && lc(opts[i].label || opts[i].text).indexOf(wantLc) >= 0) idx = i;
  if (idx < 0) {
    // The labels are what the model sees in the digest, so they are what the
    // error offers back.
    return {
      ok: false,
      error: 'no_option',
      detail: 'no option matches "' + clean(want, 80) + '"; the options are: ' + optionsOf(el).slice(0, 30).join(' | ')
    };
  }
  if (opts[idx].disabled) return { ok: false, error: 'disabled' };
  el.selectedIndex = idx;
  opts[idx].selected = true;
  fire(el, 'input', Event, { bubbles: true });
  fire(el, 'change', Event, { bubbles: true });
  return { ok: true };
}

function truthy(v) { return /^(true|yes|on|1|checked|check)$/i.test(String(v).trim()); }

// Sets `el` to `value` however that kind of control takes a value.
function setValue(el, value) {
  var tag = tagOf(el);
  var type = typeOf(el);
  if (isDisabled(el)) return { ok: false, error: 'disabled' };
  if (tag === 'select') return chooseOption(el, value);
  if (tag === 'input' && (type === 'checkbox' || type === 'radio')) {
    return setChecked(el, truthy(value));
  }
  if (tag === 'input' || tag === 'textarea') {
    if (el.readOnly) return { ok: false, error: 'not_editable', detail: 'read-only' };
    reveal(el);
    try { el.focus({ preventScroll: true }); } catch (e) { /* not focusable */ }
    nativeSetValue(el, String(value));
    inputEvents(el, String(value));
    return { ok: true };
  }
  if (isContentEditable(el)) {
    reveal(el);
    try { el.focus({ preventScroll: true }); } catch (e2) { /* not focusable */ }
    var done = false;
    try {
      document.execCommand('selectAll', false, null);
      done = document.execCommand('insertText', false, String(value));
    } catch (e3) { done = false; }
    if (!done) {
      el.textContent = String(value);
      inputEvents(el, String(value));
    }
    return { ok: true };
  }
  return { ok: false, error: 'not_editable' };
}

OPS.type_text = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  var t = tagOf(l.el);
  var type = typeOf(l.el);
  if (t === 'select' || (t === 'input' && (type === 'checkbox' || type === 'radio'))) {
    // type_text into a non-text control is a mistake worth naming, not
    // something to reinterpret silently (fill_form does the reinterpretation).
    return { ok: false, error: 'not_editable', detail: t + (type ? '[' + type + ']' : '') + ' \u2014 use select_option / set_checked' };
  }
  return setValue(l.el, a.text);
};

OPS.fill_form = function (a) {
  for (var i = 0; i < a.fields.length; i++) {
    var l = lookup(a.fields[i][0]);
    if (l.fail) { l.fail.index = i; return l.fail; }
    var r = setValue(l.el, a.fields[i][1]);
    if (!r.ok) { r.index = i; return r; }
  }
  return { ok: true };
};

OPS.select_option = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  if (tagOf(l.el) !== 'select') return { ok: false, error: 'not_editable', detail: tagOf(l.el) + ' is not a <select>' };
  if (isDisabled(l.el)) return { ok: false, error: 'disabled' };
  reveal(l.el);
  return chooseOption(l.el, a.value);
};

function setChecked(el, want) {
  var role = roleOf(el);
  var cur = readChecked(el, role);
  if (cur === null) return { ok: false, error: 'not_checkable' };
  if (cur === want) return { ok: true, changed: false };
  if (isDisabled(el)) return { ok: false, error: 'disabled' };
  if (tagOf(el) === 'input' && typeOf(el) === 'radio' && !want) {
    // Clicking a checked radio changes nothing; say so instead of pretending.
    return { ok: false, error: 'state_unchanged', detail: 'a radio button cannot be unchecked; check another one in its group' };
  }
  realClick(el);
  var native = tagOf(el) === 'input';
  if (native && !!el.checked !== want) {
    return { ok: false, error: 'state_unchanged', detail: 'the page reverted the change' };
  }
  return { ok: true, changed: true };
}

OPS.set_checked = function (a) {
  var l = lookup(a.sel);
  if (l.fail) return l.fail;
  return setChecked(l.el, !!a.checked);
};

// -- keys --

var KEY_INFO = {
  Enter: [13, 'Enter'], Escape: [27, 'Escape'], Tab: [9, 'Tab'], Backspace: [8, 'Backspace'],
  Delete: [46, 'Delete'], ArrowUp: [38, 'ArrowUp'], ArrowDown: [40, 'ArrowDown'],
  ArrowLeft: [37, 'ArrowLeft'], ArrowRight: [39, 'ArrowRight'], Home: [36, 'Home'], End: [35, 'End'],
  PageUp: [33, 'PageUp'], PageDown: [34, 'PageDown'], ' ': [32, 'Space']
};
var KEY_ALIASES = {
  return: 'Enter', esc: 'Escape', space: ' ', spacebar: ' ', del: 'Delete', up: 'ArrowUp', down: 'ArrowDown',
  left: 'ArrowLeft', right: 'ArrowRight', pgup: 'PageUp', pgdn: 'PageDown', pageup: 'PageUp', pagedown: 'PageDown',
  enter: 'Enter', escape: 'Escape', tab: 'Tab', backspace: 'Backspace', delete: 'Delete', home: 'Home', end: 'End',
  arrowup: 'ArrowUp', arrowdown: 'ArrowDown', arrowleft: 'ArrowLeft', arrowright: 'ArrowRight'
};

function parseKey(spec) {
  var m = { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false };
  var rest = String(spec);
  var mod;
  while ((mod = /^(ctrl|control|shift|alt|option|meta|cmd|command)\+(.+)$/i.exec(rest))) {
    var name = mod[1].toLowerCase();
    if (name === 'ctrl' || name === 'control') m.ctrlKey = true;
    else if (name === 'shift') m.shiftKey = true;
    else if (name === 'alt' || name === 'option') m.altKey = true;
    else m.metaKey = true;
    rest = mod[2];
  }
  var key = KEY_ALIASES[lc(rest)] || rest;
  var info = KEY_INFO[key];
  var code = info ? info[1] : (key.length === 1 ? (/[a-z]/i.test(key) ? 'Key' + key.toUpperCase() : /[0-9]/.test(key) ? 'Digit' + key : '') : key);
  var kc = info ? info[0] : (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
  m.key = key;
  m.code = code;
  m.keyCode = kc;
  return m;
}

function keyInit(m) {
  return {
    key: m.key, code: m.code, keyCode: m.keyCode, which: m.keyCode,
    ctrlKey: m.ctrlKey, shiftKey: m.shiftKey, altKey: m.altKey, metaKey: m.metaKey,
    bubbles: true, cancelable: true, composed: true, view: window
  };
}

var FOCUSABLE = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[tabindex],[contenteditable]:not([contenteditable=false])';

function moveFocus(from, backwards) {
  var all = document.querySelectorAll(FOCUSABLE);
  var list = [];
  for (var i = 0; i < all.length && list.length < 2000; i++) {
    var e = all[i];
    if (isDisabled(e)) continue;
    var ti = e.getAttribute('tabindex');
    if (ti !== null && parseInt(ti, 10) < 0) continue;
    if (!sizedRect(e)) continue;
    list.push(e);
  }
  if (!list.length) return null;
  var at = list.indexOf(from);
  var next = at < 0 ? (backwards ? list[list.length - 1] : list[0]) : list[(at + (backwards ? list.length - 1 : 1)) % list.length];
  try { next.focus(); } catch (e2) { /* not focusable */ }
  return next;
}

function insertAtCaret(el, text) {
  var tag = tagOf(el);
  if (tag === 'input' || tag === 'textarea') {
    if (el.readOnly || isDisabled(el)) return;
    var start = typeof el.selectionStart === 'number' ? el.selectionStart : el.value.length;
    var end = typeof el.selectionEnd === 'number' ? el.selectionEnd : el.value.length;
    var v = String(el.value);
    nativeSetValue(el, v.slice(0, start) + text + v.slice(end));
    try { el.setSelectionRange(start + text.length, start + text.length); } catch (e) { /* type has no selection */ }
    if (typeof InputEvent === 'function') fire(el, 'input', InputEvent, { bubbles: true, composed: true, inputType: 'insertText', data: text });
    else fire(el, 'input', Event, { bubbles: true });
  } else if (isContentEditable(el)) {
    try { document.execCommand('insertText', false, text); } catch (e2) { el.textContent = (el.textContent || '') + text; }
  }
}

function deleteAtCaret(el, forward) {
  var tag = tagOf(el);
  if (tag !== 'input' && tag !== 'textarea') return;
  if (el.readOnly || isDisabled(el)) return;
  var v = String(el.value);
  var start = typeof el.selectionStart === 'number' ? el.selectionStart : v.length;
  var end = typeof el.selectionEnd === 'number' ? el.selectionEnd : v.length;
  if (start === end) {
    if (forward) end = Math.min(v.length, end + 1); else start = Math.max(0, start - 1);
  }
  if (start === end) return;
  nativeSetValue(el, v.slice(0, start) + v.slice(end));
  try { el.setSelectionRange(start, start); } catch (e) { /* type has no selection */ }
  if (typeof InputEvent === 'function') fire(el, 'input', InputEvent, { bubbles: true, composed: true, inputType: forward ? 'deleteContentForward' : 'deleteContentBackward' });
  else fire(el, 'input', Event, { bubbles: true });
}

function submitFormOf(form) {
  if (typeof form.requestSubmit === 'function') {
    form.requestSubmit();
    return;
  }
  var btn = form.querySelector('[type=submit], button:not([type])');
  if (btn) { btn.click(); return; }
  form.submit();
}

// The default action a trusted key press would have had (synthetic key events
// have none), applied after the page's own handlers saw keydown.
function keyDefault(target, m) {
  var tag = tagOf(target);
  var type = typeOf(target);
  var key = m.key;
  var textual = (tag === 'input' && !TEXT_INPUT_BLOCKED[type]) || tag === 'textarea' || isContentEditable(target);
  if (m.ctrlKey || m.metaKey) {
    if (lc(key) === 'a' && (tag === 'input' || tag === 'textarea') && typeof target.select === 'function') target.select();
    return;
  }
  if (key === 'Enter') {
    if (tag === 'textarea') { insertAtCaret(target, '\n'); return; }
    if (tag === 'input' && !TEXT_INPUT_BLOCKED[type]) {
      var form = target.form;
      if (form) submitFormOf(form);
      return;
    }
    if (tag === 'button' || tag === 'a' || tag === 'summary' || tag === 'select') { if (tag !== 'select') target.click(); return; }
    var r = lc(attr(target, 'role'));
    if (r === 'button' || r === 'link' || r === 'menuitem' || r === 'tab' || r === 'option') target.click();
    return;
  }
  if (key === ' ') {
    if (textual) { insertAtCaret(target, ' '); return; }
    var r2 = lc(attr(target, 'role'));
    if (tag === 'button' || tag === 'summary' || (tag === 'input' && (type === 'checkbox' || type === 'radio' || BUTTON_INPUTS[type])) ||
        r2 === 'button' || r2 === 'checkbox' || r2 === 'radio' || r2 === 'switch' || r2 === 'tab') target.click();
    return;
  }
  if (key === 'Tab') { moveFocus(target, m.shiftKey); return; }
  if (key === 'Backspace') { deleteAtCaret(target, false); return; }
  if (key === 'Delete') { deleteAtCaret(target, true); return; }
  if ((key === 'ArrowDown' || key === 'ArrowUp') && tag === 'select') {
    var i = target.selectedIndex + (key === 'ArrowDown' ? 1 : -1);
    if (i >= 0 && i < target.options.length) {
      target.selectedIndex = i;
      fire(target, 'input', Event, { bubbles: true });
      fire(target, 'change', Event, { bubbles: true });
    }
    return;
  }
  if (key.length === 1 && textual) insertAtCaret(target, key);
}

OPS.press_key = function (a) {
  var target;
  if (a.sel) {
    var l = lookup(a.sel);
    if (l.fail) return l.fail;
    target = l.el;
    reveal(target);
    try { target.focus({ preventScroll: true }); } catch (e) { /* not focusable */ }
  } else {
    target = document.activeElement || document.body;
  }
  var m = parseKey(a.key);
  if (!m.key) return { ok: false, error: 'bad_key', detail: 'empty key' };
  var init = keyInit(m);
  var notPrevented = target.dispatchEvent(new KeyboardEvent('keydown', init));
  if (notPrevented && (m.key.length === 1 || m.key === 'Enter')) {
    try { target.dispatchEvent(new KeyboardEvent('keypress', init)); } catch (e2) { /* page handler */ }
  }
  if (notPrevented) keyDefault(target, m);
  try { target.dispatchEvent(new KeyboardEvent('keyup', init)); } catch (e3) { /* page handler */ }
  return { ok: true };
};

OPS.submit_form = function (a) {
  var form = null;
  if (a.sel) {
    var l = lookup(a.sel);
    if (l.fail) return l.fail;
    form = tagOf(l.el) === 'form' ? l.el : (l.el.form || (l.el.closest ? l.el.closest('form') : null));
  } else {
    var active = document.activeElement;
    form = active && (active.form || (active.closest ? active.closest('form') : null));
    if (!form) {
      for (var i = 0; i < document.forms.length && !form; i++) {
        if (sizedRect(document.forms[i])) form = document.forms[i];
      }
    }
  }
  if (!form) return { ok: false, error: 'no_form' };
  if (!form.noValidate && typeof form.checkValidity === 'function' && !form.checkValidity()) {
    var bad = [];
    var els = form.elements;
    for (var j = 0; j < els.length && bad.length < 5; j++) {
      if (els[j].willValidate && els[j].checkValidity && !els[j].checkValidity()) {
        bad.push(nameOf(els[j], roleOf(els[j]) || '') || els[j].name || tagOf(els[j]));
      }
    }
    return { ok: false, error: 'invalid_form', detail: bad.join(', ') };
  }
  submitFormOf(form);
  return { ok: true };
};

function foldChars(s) {
  var o = '';
  for (var i = 0; i < s.length; i++) {
    var c = s.charAt(i).toLowerCase();
    o += c.length === 1 ? c : s.charAt(i);
  }
  return o;
}

OPS.find_text = function (a) {
  var needle = clean(a.text, 200);
  if (!needle) return { ok: true, count: 0, snippets: [] };
  var walk = walkPage(400000, false);
  var hay = clean(walk.text, 400000);
  var hayF = foldChars(hay);
  var needF = foldChars(needle);
  var count = 0;
  var snippets = [];
  var ctx = 60;
  var from = 0;
  var at;
  while ((at = hayF.indexOf(needF, from)) >= 0) {
    count += 1;
    if (snippets.length < 5) {
      var s0 = Math.max(0, at - ctx);
      var s1 = Math.min(hay.length, at + needF.length + ctx);
      snippets.push(
        (s0 > 0 ? '\u2026' : '') + hay.slice(s0, at) + '[' + hay.slice(at, at + needF.length) + ']' +
        hay.slice(at + needF.length, s1) + (s1 < hay.length ? '\u2026' : '')
      );
    }
    from = at + needF.length;
  }
  return { ok: true, count: count, snippets: snippets.map(function (s) { return clean(s, 260); }), truncated: walk.truncated };
};

OPS.extract_links = function (a) {
  var scope = document;
  if (a.sel) {
    var l = lookup(a.sel);
    if (l.fail) return l.fail;
    scope = l.el;
  }
  var anchors = [];
  if (scope !== document && tagOf(scope) === 'a' && scope.hasAttribute('href')) anchors.push(scope);
  var inner = scope.querySelectorAll('a[href]');
  for (var i = 0; i < inner.length && anchors.length < 600; i++) anchors.push(inner[i]);
  var links = [];
  var seen = {};
  for (var k = 0; k < anchors.length && links.length < 300; k++) {
    var el = anchors[k];
    var st = styleOf(el);
    if (st && (st.display === 'none' || st.visibility === 'hidden')) continue;
    if (!sizedRect(el)) continue;
    var href = clean(String(el.href), 500);
    var text = nameOf(el, 'link');
    var key = href + '\n' + text;
    if (seen[key]) continue;
    seen[key] = true;
    links.push({ text: text, href: href });
  }
  return { ok: true, links: links };
};

var result;
try {
  var op = OPS[__op];
  result = op ? op(__args) : { ok: false, error: 'unknown_op', detail: String(__op) };
} catch (e) {
  result = { ok: false, error: 'script_error', detail: clean(e && e.message ? e.message : e, 300) };
}
return out(result);
