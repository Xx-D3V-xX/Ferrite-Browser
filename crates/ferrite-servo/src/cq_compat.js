/* Ferrite compatibility script: CSS container queries (`@container`, `cqw` and friends).
 *
 * Servo 0.6's style engine drops every `@container` rule while parsing (it reads the rule
 * only when built for Firefox), so a layout written with container queries falls back to
 * its base styles. This reads the page's own style sheets, finds the container rules and
 * the container-relative units, and rewrites them into ordinary rules that the engine does
 * understand, switched on and off by measuring the containers:
 *
 *   .card { container-type: inline-size; container-name: card }
 *   @container card (min-width: 30em) { .title { font-size: 2cqw } }
 *
 * becomes, in a style element this script adds at the end of the head,
 *
 *   :where([data-cq~="1"]) .title { font-size: calc(2 * var(--cq-w)) }
 *
 * and `data-cq="1"` is put on a `.card` while its content box is at least 30em wide (the
 * attribute is the only thing the page can see change). `--cq-w` is 1% of the container's
 * width, set on the container, so a descendant inherits its nearest container's size.
 *
 * Supported: container-type (size, inline-size), container-name, the `container`
 * shorthand; conditions with `min-/max-` features, range syntax (`width >= 400px`,
 * `400px <= width < 800px`), `and`, `or`, `not`, parentheses; width, height, inline-size,
 * block-size, aspect-ratio, orientation; px, em, rem, vw, vh, cm, mm, in, pt; nesting in
 * `@media`, `@supports`, `@layer`; cqw, cqh, cqi, cqb, cqmin, cqmax in any declaration.
 *
 * Differences from the real thing, none of which a typical layout meets:
 *  - a rule applies to the descendants of *any* container whose condition holds, not the
 *    nearest one that matches (nested containers of the same name are the visible case);
 *  - `style(...)` and `scroll-state(...)` queries are not evaluated (their rules are left
 *    out);
 *  - style sheets are read from `<style>` elements and from `<link>` elements the page may
 *    fetch (same origin, or served with CORS); shadow roots and `@import`ed sheets are not;
 *  - the first measurement is a frame after the first paint.
 */
(function () {
  'use strict';
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  if (typeof ResizeObserver !== 'function') return;
  // The engine's own support, if it ever has it: a container rule that survives parsing.
  try {
    var probe = document.createElement('style');
    probe.textContent = '@container (min-width: 1px){ .__ferrite_cq_probe{ color: red } }';
    (document.head || document.documentElement).appendChild(probe);
    var has = probe.sheet && probe.sheet.cssRules && probe.sheet.cssRules.length > 0;
    probe.remove();
    if (has) return;
  } catch (e) { /* assume the engine has none */ }

  // ---------------------------------------------------------------- parsing

  function stripComments(text) { return text.replace(/\/\*[\s\S]*?\*\//g, ''); }

  // Splits a block of CSS into top-level items: { at: 'name', prelude, body } for
  // `@name prelude { body }`, { at: 'name', prelude, body: null } for `@name prelude;`, and
  // { selector, body } for `selector { declarations }`.
  function splitItems(text) {
    var items = [], i = 0, n = text.length;
    while (i < n) {
      while (i < n && /\s/.test(text[i])) i++;
      if (i >= n) break;
      var start = i, depthParen = 0, quote = null;
      // Read the prelude up to `{` or `;` (outside strings and parentheses).
      while (i < n) {
        var c = text[i];
        if (quote) { if (c === '\\') i++; else if (c === quote) quote = null; }
        else if (c === '"' || c === "'") quote = c;
        else if (c === '(') depthParen++;
        else if (c === ')') depthParen = Math.max(0, depthParen - 1);
        else if (depthParen === 0 && (c === '{' || c === ';' || c === '}')) break;
        i++;
      }
      var prelude = text.slice(start, i).trim();
      if (i >= n || text[i] === ';') { if (prelude) items.push(makeItem(prelude, null)); i++; continue; }
      if (text[i] === '}') { i++; continue; }
      // A block: find its matching brace.
      var bodyStart = ++i, depth = 1; quote = null;
      while (i < n && depth > 0) {
        var d = text[i];
        if (quote) { if (d === '\\') i++; else if (d === quote) quote = null; }
        else if (d === '"' || d === "'") quote = d;
        else if (d === '{') depth++;
        else if (d === '}') depth--;
        i++;
      }
      items.push(makeItem(prelude, text.slice(bodyStart, depth === 0 ? i - 1 : i)));
    }
    return items;
  }
  function makeItem(prelude, body) {
    if (prelude[0] === '@') {
      var m = /^@([a-zA-Z-]+)\s*([\s\S]*)$/.exec(prelude);
      return { at: m ? m[1].toLowerCase() : '', prelude: m ? m[2].trim() : '', body: body };
    }
    return { selector: prelude, body: body };
  }

  function splitTop(text, separator) {
    var parts = [], depth = 0, quote = null, last = 0;
    for (var i = 0; i < text.length; i++) {
      var c = text[i];
      if (quote) { if (c === '\\') i++; else if (c === quote) quote = null; continue; }
      if (c === '"' || c === "'") quote = c;
      else if (c === '(' || c === '[') depth++;
      else if (c === ')' || c === ']') depth--;
      else if (c === separator && depth === 0) { parts.push(text.slice(last, i)); last = i + 1; }
    }
    parts.push(text.slice(last));
    return parts;
  }

  // ------------------------------------------------------- container discovery

  // A declaration block's container-type / container-name / container shorthand.
  function containerFrom(decls) {
    var type = null, name = null;
    splitTop(decls, ';').forEach(function (d) {
      var colon = d.indexOf(':');
      if (colon < 0) return;
      var prop = d.slice(0, colon).trim().toLowerCase(), value = d.slice(colon + 1).replace(/!important/i, '').trim();
      if (prop === 'container-type') type = value;
      else if (prop === 'container-name') name = value;
      else if (prop === 'container') {
        var parts = value.split('/');
        name = parts[0].trim();
        type = parts.length > 1 ? parts[1].trim() : 'normal';
      }
    });
    if (!type || type === 'normal') {
      // A named container with no type still contains nothing: only a type makes one.
      return null;
    }
    return { type: type.toLowerCase(), names: name && name !== 'none' ? name.split(/\s+/) : [] };
  }

  // ------------------------------------------------------- condition handling

  // A parsed condition is a tree: { op: 'and'|'or'|'not', items }, or a leaf
  // { feature, compare: [[op, value], ...] } / { feature } (a boolean feature) / { never }.
  function parseCondition(text) {
    var pos = 0, s = text.trim();
    function skip() { while (pos < s.length && /\s/.test(s[pos])) pos++; }
    function ident() { var m = /^[a-zA-Z-]+/.exec(s.slice(pos)); if (!m) return null; pos += m[0].length; return m[0].toLowerCase(); }
    function group() {
      // `(` ... `)`, returns the text inside.
      var depth = 0, start = pos;
      if (s[pos] !== '(') return null;
      for (; pos < s.length; pos++) {
        if (s[pos] === '(') depth++;
        else if (s[pos] === ')') { depth--; if (depth === 0) { pos++; return s.slice(start + 1, pos - 1); } }
      }
      return null;
    }
    function term() {
      skip();
      var save = pos, word = ident();
      if (word === 'not') { var inner = term(); return inner && { op: 'not', items: [inner] }; }
      if (word === 'style' || word === 'scroll-state') { group(); return { never: true }; }
      pos = save;
      var inside = group();
      if (inside === null) return null;
      var inner = inside.trim();
      // A parenthesised condition or a feature?
      if (/^\(/.test(inner) || /^not\b/i.test(inner)) return parseCondition(inner);
      return feature(inner);
    }
    function chain() {
      var first = term();
      if (!first) return null;
      var items = [first], op = null;
      for (;;) {
        skip();
        var save = pos, w = ident();
        if (w === 'and' || w === 'or') {
          if (op && op !== w) return null;
          op = w;
          var next = term();
          if (!next) return null;
          items.push(next);
        } else { pos = save; break; }
      }
      return op ? { op: op, items: items } : first;
    }
    var tree = chain();
    skip();
    return pos >= s.length ? tree : null;
  }

  var COMPARE = /(<=|>=|<|>|=)/;
  function feature(text) {
    // `min-width: 400px`, `orientation: portrait`, `width >= 400px`, `400px <= width < 800px`.
    var colon = text.indexOf(':');
    if (colon > 0 && !COMPARE.test(text.slice(0, colon))) {
      var name = text.slice(0, colon).trim().toLowerCase(), value = text.slice(colon + 1).trim();
      var m = /^(min|max)-(.+)$/.exec(name);
      if (m) return { feature: canonical(m[2]), compare: [[m[1] === 'min' ? '>=' : '<=', value]] };
      return { feature: canonical(name), equals: value };
    }
    var parts = text.split(COMPARE).map(function (p) { return p.trim(); });
    if (parts.length === 3) {
      // `width >= 400px` or `400px <= width`
      if (/^[a-z-]+$/i.test(parts[0])) return { feature: canonical(parts[0]), compare: [[parts[1], parts[2]]] };
      return { feature: canonical(parts[2]), compare: [[flip(parts[1]), parts[0]]] };
    }
    if (parts.length === 5) {
      // `400px <= width < 800px`
      return { feature: canonical(parts[2]), compare: [[flip(parts[1]), parts[0]], [parts[3], parts[4]]] };
    }
    if (parts.length === 1 && /^[a-z-]+$/i.test(parts[0])) return { feature: canonical(parts[0]) };
    return { never: true };
  }
  function flip(op) { return op === '<' ? '>' : op === '<=' ? '>=' : op === '>' ? '<' : op === '>=' ? '<=' : '='; }
  function canonical(name) {
    name = name.toLowerCase();
    return name === 'inline-size' ? 'width' : name === 'block-size' ? 'height' : name;
  }

  // The lengths a condition is measured in.
  function toPx(value, el) {
    var m = /^(-?[\d.]+)\s*([a-z%]*)$/i.exec(String(value).trim());
    if (!m) return NaN;
    var v = parseFloat(m[1]), unit = m[2].toLowerCase();
    switch (unit) {
      case '': return v === 0 ? 0 : NaN;
      case 'px': return v;
      case 'em': return v * parseFloat(getComputedStyle(el).fontSize);
      case 'rem': return v * parseFloat(getComputedStyle(document.documentElement).fontSize);
      case 'vw': return v * window.innerWidth / 100;
      case 'vh': return v * window.innerHeight / 100;
      case 'vmin': return v * Math.min(window.innerWidth, window.innerHeight) / 100;
      case 'vmax': return v * Math.max(window.innerWidth, window.innerHeight) / 100;
      case 'cm': return v * 96 / 2.54;
      case 'mm': return v * 96 / 25.4;
      case 'in': return v * 96;
      case 'pt': return v * 96 / 72;
      case 'pc': return v * 16;
      default: return NaN;
    }
  }
  function ratioOf(value) {
    var m = /^([\d.]+)\s*(?:\/\s*([\d.]+))?$/.exec(String(value).trim());
    return m ? parseFloat(m[1]) / (m[2] ? parseFloat(m[2]) : 1) : NaN;
  }
  function compare(a, op, b) {
    switch (op) { case '<': return a < b; case '<=': return a <= b; case '>': return a > b; case '>=': return a >= b; default: return a === b; }
  }

  // Whether `tree` holds for a container of `type` with a content box of w x h.
  function holds(tree, el, type, w, h) {
    if (!tree || tree.never) return false;
    if (tree.op === 'not') return !holds(tree.items[0], el, type, w, h);
    if (tree.op === 'and') return tree.items.every(function (t) { return holds(t, el, type, w, h); });
    if (tree.op === 'or') return tree.items.some(function (t) { return holds(t, el, type, w, h); });
    var f = tree.feature;
    if (f === 'height' && type === 'inline-size') return false; // not queryable on this container
    if (f === 'width' || f === 'height') {
      var size = f === 'width' ? w : h;
      if (!tree.compare) return size > 0;
      return tree.compare.every(function (c) { var px = toPx(c[1], el); return !isNaN(px) && compare(size, c[0], px); });
    }
    if (f === 'aspect-ratio') {
      if (type === 'inline-size') return false;
      if (!tree.compare) return h > 0;
      return tree.compare.every(function (c) { var r = ratioOf(c[1]); return !isNaN(r) && compare(w / (h || 1), c[0], r); });
    }
    if (f === 'orientation') {
      if (type === 'inline-size') return false;
      return tree.equals === (h >= w ? 'portrait' : 'landscape');
    }
    return false;
  }

  // ------------------------------------------------------------ rewriting

  var CQ_UNIT = /(-?[\d.]+)(cqw|cqh|cqi|cqb|cqmin|cqmax)\b/gi;
  function replaceUnits(text) {
    return text.replace(CQ_UNIT, function (all, num, unit) {
      var v = { cqw: '--cq-w', cqi: '--cq-w', cqh: '--cq-h', cqb: '--cq-h', cqmin: '--cq-min', cqmax: '--cq-max' }[unit.toLowerCase()];
      return 'calc(' + num + ' * var(' + v + ', 1vw))';
    });
  }
  var rules = [];     // { id, names: [..]|[], tree } for each container rule found
  var containerSelectors = []; // { selector, type, names }
  var unitsUsed = false;

  function prefixed(selectorList, id) {
    return splitTop(selectorList, ',').map(function (sel) {
      sel = sel.trim();
      if (!sel) return '';
      // `&` (nesting) and `:scope` have nothing to be relative to here: left alone.
      return ':where([data-cq~="' + id + '"]) ' + sel;
    }).filter(Boolean).join(', ');
  }

  // Rewrites the inside of a container rule: style rules get the container prefix,
  // nested at-rules keep their wrappers.
  function rewriteInside(text, id) {
    var out = '';
    splitItems(text).forEach(function (item) {
      if (item.selector !== undefined) {
        out += prefixed(item.selector, id) + '{' + replaceUnits(item.body) + '}\n';
      } else if (item.body !== null && (item.at === 'media' || item.at === 'supports' || item.at === 'layer' || item.at === 'container')) {
        if (item.at === 'container') out += convertContainer(item, id);
        else out += '@' + item.at + ' ' + item.prelude + '{' + rewriteInside(item.body, id) + '}\n';
      }
    });
    return out;
  }

  function convertContainer(item, outerId) {
    var prelude = item.prelude, names = [];
    var nameMatch = /^([a-zA-Z_][\w-]*)\s+(?=[(not])/.exec(prelude);
    if (nameMatch && !/^(not|and|or|style|scroll-state)$/i.test(nameMatch[1])) {
      names = [nameMatch[1]];
      prelude = prelude.slice(nameMatch[0].length);
    }
    var tree = parseCondition(prelude);
    if (!tree) return '';
    var id = rules.length + 1;
    rules.push({ id: id, names: names, tree: tree });
    return rewriteInside(item.body, id);
  }

  // Turns one sheet's text into the rules to add; also records containers.
  function transform(text) {
    var out = '';
    splitItems(stripComments(text)).forEach(function (item) { out += transformItem(item); });
    return out;
  }
  function transformItem(item) {
    if (item.selector !== undefined) {
      var found = containerFrom(item.body);
      if (found) containerSelectors.push({ selector: item.selector, type: found.type, names: found.names });
      if (CQ_UNIT.test(item.body)) {
        CQ_UNIT.lastIndex = 0;
        unitsUsed = true;
        return item.selector + '{' + replaceUnits(item.body) + '}\n';
      }
      CQ_UNIT.lastIndex = 0;
      return '';
    }
    if (item.body === null) return '';
    if (item.at === 'container') return convertContainer(item, 0);
    if (item.at === 'media' || item.at === 'supports' || item.at === 'layer' || item.at === 'scope' || item.at === 'starting-style') {
      var inner = '';
      splitItems(item.body).forEach(function (child) { inner += transformItem(child); });
      return inner ? '@' + item.at + ' ' + item.prelude + '{' + inner + '}\n' : '';
    }
    return '';
  }

  // -------------------------------------------------------- reading the sheets

  var seenSheets = {};
  function gatherSources() {
    var jobs = [];
    [].forEach.call(document.querySelectorAll('style'), function (el) {
      if (el.id === 'ferrite-cq-style' || el.hasAttribute('data-ferrite-cq')) return;
      var key = el;
      if (seenSheets[key.__ferriteCq]) return;
      el.__ferriteCq = el.__ferriteCq || ('s' + Math.random());
      seenSheets[el.__ferriteCq] = true;
      jobs.push(Promise.resolve(el.textContent));
    });
    [].forEach.call(document.querySelectorAll('link[rel~="stylesheet"][href]'), function (el) {
      var href = el.href;
      if (!href || seenSheets[href]) return;
      seenSheets[href] = true;
      jobs.push(fetch(href, { credentials: 'same-origin' }).then(function (r) { return r.ok ? r.text() : ''; }).catch(function () { return ''; }));
    });
    return jobs;
  }

  var generated = '';
  var styleEl = null;
  function ingest(texts) {
    var added = false;
    texts.forEach(function (text) {
      if (!text || (text.indexOf('@container') < 0 && text.indexOf('container') < 0 && !/cq(w|h|i|b|min|max)\b/i.test(text))) return;
      var css = transform(text);
      if (css) { generated += css; added = true; }
    });
    if (added) {
      if (!styleEl) {
        styleEl = document.createElement('style');
        styleEl.id = 'ferrite-cq-style';
        styleEl.setAttribute('data-ferrite-cq', '');
      }
      styleEl.textContent = generated;
      if (!styleEl.parentNode) (document.head || document.documentElement).appendChild(styleEl);
    }
    return added;
  }

  // ------------------------------------------------------------- measuring

  var inlineContainers = new Map();
  var observer = new ResizeObserver(function () { schedule(); });
  var observed = new WeakSet();
  var scheduled = false;
  function schedule() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(function () { scheduled = false; update(); });
  }

  function contentBox(el) {
    var cs = getComputedStyle(el), r = el.getBoundingClientRect();
    var pl = parseFloat(cs.paddingLeft) || 0, pr = parseFloat(cs.paddingRight) || 0;
    var pt = parseFloat(cs.paddingTop) || 0, pb = parseFloat(cs.paddingBottom) || 0;
    var bl = parseFloat(cs.borderLeftWidth) || 0, br = parseFloat(cs.borderRightWidth) || 0;
    var bt = parseFloat(cs.borderTopWidth) || 0, bb = parseFloat(cs.borderBottomWidth) || 0;
    return { w: Math.max(0, r.width - pl - pr - bl - br), h: Math.max(0, r.height - pt - pb - bt - bb) };
  }

  function update() {
    var seen = new Set();
    containerSelectors.forEach(function (c) {
      var list;
      try { list = document.querySelectorAll(c.selector); } catch (e) { return; }
      [].forEach.call(list, function (el) {
        var entry = el.__ferriteCqInfo || (el.__ferriteCqInfo = { types: {}, names: {} });
        entry.types[c.selector] = c.type;
        c.names.forEach(function (n) { entry.names[n] = true; });
        seen.add(el);
        if (!observed.has(el)) { observed.add(el); observer.observe(el); }
      });
    });
    // Containers declared in an element's own `style` attribute. The engine drops a
    // property it does not know the first time the attribute is rewritten (this script
    // rewrites it when it sets `--cq-w`), so each is remembered when first seen.
    // (The engine does not match `[style*=...]`, so every element with a style attribute is looked at.)
    [].forEach.call(document.querySelectorAll('[style]'), function (el) {
      if (inlineContainers.has(el)) return;
      var text = el.getAttribute('style') || '';
      if (text.indexOf('container') < 0) return;
      var found = containerFrom(text);
      if (found) inlineContainers.set(el, found);
    });
    inlineContainers.forEach(function (found, el) {
      if (!el.isConnected) { inlineContainers.delete(el); return; }
      var entry = el.__ferriteCqInfo || (el.__ferriteCqInfo = { types: {}, names: {} });
      entry.types['@style'] = found.type;
      found.names.forEach(function (n) { entry.names[n] = true; });
      seen.add(el);
      if (!observed.has(el)) { observed.add(el); observer.observe(el); }
    });
    seen.forEach(function (el) {
      var info = el.__ferriteCqInfo;
      var type = Object.keys(info.types).some(function (k) { return info.types[k] === 'size'; }) ? 'size' : 'inline-size';
      var box = contentBox(el);
      var tokens = [];
      rules.forEach(function (rule) {
        if (rule.names.length && !rule.names.some(function (n) { return info.names[n]; })) return;
        if (holds(rule.tree, el, type, box.w, box.h)) tokens.push(String(rule.id));
      });
      var value = tokens.join(' ');
      if (el.getAttribute('data-cq') !== value) {
        if (value) el.setAttribute('data-cq', value); else el.removeAttribute('data-cq');
      }
      el.style.setProperty('--cq-w', (box.w / 100) + 'px');
      el.style.setProperty('--cq-h', (box.h / 100) + 'px');
      el.style.setProperty('--cq-min', (Math.min(box.w, box.h) / 100) + 'px');
      el.style.setProperty('--cq-max', (Math.max(box.w, box.h) / 100) + 'px');
    });
  }

  // ----------------------------------------------------------------- driving

  function scan() {
    var jobs = gatherSources();
    if (!jobs.length) return;
    Promise.all(jobs).then(function (texts) { if (ingest(texts)) schedule(); });
  }

  function start() {
    scan();
    // New style or link elements, new nodes that may be containers, class changes.
    new MutationObserver(function (records) {
      var needScan = false;
      records.forEach(function (r) {
        [].forEach.call(r.addedNodes, function (n) {
          if (n.nodeType === 1 && (n.tagName === 'STYLE' || n.tagName === 'LINK' || (n.querySelector && n.querySelector('style, link[rel~="stylesheet"]')))) needScan = true;
        });
        if (r.type === 'characterData' && r.target.parentNode && r.target.parentNode.tagName === 'STYLE') {
          // A changed style element is read again as a new sheet.
          if (r.target.parentNode.id !== 'ferrite-cq-style') { r.target.parentNode.__ferriteCq = null; needScan = true; }
        }
      });
      if (needScan) scan();
      if (rules.length || containerSelectors.length) schedule();
    }).observe(document.documentElement, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['class', 'id', 'hidden'] });
    window.addEventListener('resize', schedule);
    window.addEventListener('load', function () { scan(); schedule(); });
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
  else start();
})();
