/* Ferrite compatibility script: inline SVG colours.
 *
 * Servo paints an inline <svg> by serializing the element and rasterizing that
 * markup on its own, so only styles written *on the SVG's own elements* reach
 * the painter. Anything that comes from a stylesheet or is inherited from the
 * page (`color`, `fill: currentColor` through an ancestor, `.icon path { fill }`,
 * a custom property) is invisible to it, and the icon paints black. On a dark
 * theme that is black on near-black: the icons simply vanish.
 *
 * This copies each SVG's resolved styles onto its own elements as inline style,
 * the one place the painter reads them. It changes no meaning: it writes the
 * value the page already computed. It touches only <svg> subtrees, only inline
 * `style`, and nothing else on the page. Idempotent; safe to run twice.
 *
 * One structural step comes first: an icon drawn through `<use href="#id">`
 * whose target lives elsewhere in the page (a sprite sheet) is replaced by an
 * inline copy of that target. The painter never applies the `<use>`'s own
 * colour to the referenced content, so the copy is what lets the icon take the
 * colour the page gave it. References to other files are left alone.
 */
(function () {
  'use strict';
  if (window.__ferriteSvgCompat) return;
  window.__ferriteSvgCompat = true;

  var SHAPES = 'svg,g,path,circle,ellipse,rect,line,polyline,polygon,text,tspan,use,symbol,defs,clipPath,mask,linearGradient,radialGradient,stop';
  // Presentation properties the painter honours that stylesheets commonly set,
  // with their initial values: a property still at its initial value is left
  // alone (writing it would change nothing and cost style work).
  var INITIAL = {
    'stroke': 'none', 'stroke-width': '1px', 'stroke-linecap': 'butt',
    'stroke-linejoin': 'miter', 'fill-opacity': '1', 'stroke-opacity': '1',
    'opacity': '1', 'fill-rule': 'nonzero', 'clip-rule': 'nonzero',
    'stop-opacity': '1'
  };
  var PROPS = ['fill', 'stroke', 'stroke-width', 'stroke-linecap', 'stroke-linejoin',
               'fill-opacity', 'stroke-opacity', 'opacity', 'fill-rule', 'clip-rule',
               'stop-color', 'stop-opacity'];
  // How many SVGs one pass handles. A page can hold hundreds of icons; doing
  // them in slices keeps the page responsive while they are fixed.
  var CHUNK = 40;
  var queued = false;
  var pending = [];

  var SVG_NS = 'http://www.w3.org/2000/svg';
  var XLINK_NS = 'http://www.w3.org/1999/xlink';
  var COPY_FROM_USE = ['class', 'style', 'fill', 'stroke', 'stroke-width', 'opacity',
                       'fill-opacity', 'stroke-opacity', 'color'];

  function copyAttrs(from, to, names) {
    for (var i = 0; i < names.length; i++) {
      if (from.hasAttribute(names[i])) to.setAttribute(names[i], from.getAttribute(names[i]));
    }
  }

  // Replaces each `<use href="#id">` in `svg` whose target is outside `svg` with
  // an inline copy of the target.
  function expandUses(svg) {
    var uses = Array.prototype.slice.call(svg.querySelectorAll('use'));
    for (var i = 0; i < uses.length; i++) {
      var use = uses[i];
      var ref = use.getAttribute('href') || use.getAttributeNS(XLINK_NS, 'href') || '';
      if (ref.charAt(0) !== '#' || ref.length < 2) continue;
      var target = document.getElementById(ref.slice(1));
      if (!target || svg.contains(target) || target.contains(use) || !use.parentNode) continue;
      var holder, kids, k;
      if (target.tagName.toLowerCase() === 'symbol') {
        // A symbol becomes a nested <svg> with the symbol's viewBox, sized by the
        // <use> (100% when the <use> gives no size, as the specification says).
        holder = document.createElementNS(SVG_NS, 'svg');
        copyAttrs(target, holder, ['viewBox', 'preserveAspectRatio']);
        copyAttrs(use, holder, ['x', 'y', 'width', 'height']);
        if (!use.hasAttribute('width')) holder.setAttribute('width', '100%');
        if (!use.hasAttribute('height')) holder.setAttribute('height', '100%');
        kids = target.childNodes;
        for (k = 0; k < kids.length; k++) holder.appendChild(kids[k].cloneNode(true));
      } else {
        holder = document.createElementNS(SVG_NS, 'g');
        var tx = use.getAttribute('x') || '0', ty = use.getAttribute('y') || '0';
        var tf = (use.getAttribute('transform') || '') + ' translate(' + tx + ' ' + ty + ')';
        holder.setAttribute('transform', tf.trim());
        holder.appendChild(target.cloneNode(true));
      }
      copyAttrs(use, holder, COPY_FROM_USE);
      use.parentNode.replaceChild(holder, use);
    }
  }

  // One pass over `svgs`: every read first, then every write. Interleaving them
  // forces a style recalculation per element, which is what makes a naive
  // version of this crawl on a page with many icons.
  function fixBatch(svgs) {
    var plan = [];
    var i, j, k;
    for (i = 0; i < svgs.length; i++) {
      try { expandUses(svgs[i]); } catch (e) { /* leave this icon as the page made it */ }
    }
    for (i = 0; i < svgs.length; i++) {
      var svg = svgs[i];
      var nodes = Array.prototype.slice.call(svg.querySelectorAll(SHAPES));
      nodes.push(svg);
      for (j = 0; j < nodes.length; j++) {
        var el = nodes[j];
        var cs;
        try { cs = window.getComputedStyle(el); } catch (e) { continue; }
        if (!cs) continue;
        var values = [];
        for (k = 0; k < PROPS.length; k++) {
          var p = PROPS[k];
          var v = cs.getPropertyValue(p);
          if (v && (p === 'fill' || v !== INITIAL[p])) values.push(p, v);
        }
        // `color` only matters on the root: it is what `currentColor` inherits.
        if (el === svg) values.push('color', cs.getPropertyValue('color'));
        plan.push(el, values);
      }
    }
    for (i = 0; i < plan.length; i += 2) {
      var target = plan[i], vals = plan[i + 1];
      for (k = 0; k < vals.length; k += 2) {
        if (vals[k + 1] && target.style.getPropertyValue(vals[k]) !== vals[k + 1]) {
          target.style.setProperty(vals[k], vals[k + 1]);
        }
      }
    }
    for (i = 0; i < svgs.length; i++) {
      svgs[i].__ferriteSvgFixed = 1;
      // The painter re-reads the markup when the root's attributes change, and
      // not when only a child's does; this is that change.
      svgs[i].setAttribute('data-svg-compat', '1');
    }
  }

  function flush() {
    queued = false;
    var batch = [];
    while (pending.length && batch.length < CHUNK) {
      var svg = pending.shift();
      if (svg && svg.__ferriteSvgFixed !== 1 && svg.isConnected !== false) batch.push(svg);
    }
    if (batch.length) fixBatch(batch);
    if (pending.length) enqueue();
  }

  function enqueue() {
    if (queued) return;
    queued = true;
    // After the page's own styles have applied for this change.
    (window.requestAnimationFrame || window.setTimeout)(flush, 0);
  }

  function schedule(svgs) {
    for (var i = 0; i < svgs.length; i++) pending.push(svgs[i]);
    if (pending.length) enqueue();
  }

  function rootSvgsIn(node) {
    var out = [];
    if (!node || node.nodeType !== 1) return out;
    if (node.tagName && node.tagName.toLowerCase() === 'svg') out.push(node);
    var found = node.querySelectorAll ? node.querySelectorAll('svg') : [];
    for (var i = 0; i < found.length; i++) out.push(found[i]);
    return out;
  }

  function scan() { schedule(document.querySelectorAll('svg')); }

  try {
    new MutationObserver(function (records) {
      for (var i = 0; i < records.length; i++) {
        var added = records[i].addedNodes;
        for (var j = 0; j < added.length; j++) schedule(rootSvgsIn(added[j]));
      }
    }).observe(document, { childList: true, subtree: true });
  } catch (e) { /* no MutationObserver: the load scan below still runs */ }

  document.addEventListener('DOMContentLoaded', scan);
  window.addEventListener('load', scan);
  if (document.readyState !== 'loading') scan();
})();

/* SVG geometry: getTotalLength(), getPointAtLength() and getBBox().
 *
 * The engine's SVG elements have none of them (its IDL has them commented out), and
 * pages call them: Google Meet measures an SVG path while it starts a call and stopped
 * on "getTotalLength is not a function"; d3 and most charting libraries call getBBox.
 * Each shape is turned into path data, the path into polylines (curves and arcs in
 * fine steps), and lengths, points and boxes are read from those. Transforms are not
 * applied (the values are in the element's own user space, as getTotalLength's are;
 * a container's box is the union of its shapes' boxes). Skipped where the engine has
 * the real methods.
 */
(function () {
  'use strict';
  if (typeof SVGGeometryElement !== 'function' || typeof SVGGeometryElement.prototype.getTotalLength === 'function') return;

  var CURVE_STEPS = 32;

  function attr(el, name) {
    var v = parseFloat(el.getAttribute(name));
    return isFinite(v) ? v : 0;
  }

  // The path data of a basic shape, as the SVG spec defines its equivalent path.
  function pathDataOf(el) {
    var tag = el.localName;
    if (tag === 'path') return el.getAttribute('d') || '';
    if (tag === 'line') return 'M' + attr(el, 'x1') + ' ' + attr(el, 'y1') + 'L' + attr(el, 'x2') + ' ' + attr(el, 'y2');
    if (tag === 'polyline' || tag === 'polygon') {
      var nums = (el.getAttribute('points') || '').match(/[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/g) || [];
      if (nums.length < 2) return '';
      var d = 'M' + nums[0] + ' ' + nums[1];
      for (var i = 2; i + 1 < nums.length; i += 2) d += 'L' + nums[i] + ' ' + nums[i + 1];
      return tag === 'polygon' ? d + 'Z' : d;
    }
    if (tag === 'circle' || tag === 'ellipse') {
      var cx = attr(el, 'cx'), cy = attr(el, 'cy');
      var rx = tag === 'circle' ? attr(el, 'r') : attr(el, 'rx');
      var ry = tag === 'circle' ? rx : attr(el, 'ry');
      if (rx <= 0 || ry <= 0) return '';
      return 'M' + (cx + rx) + ' ' + cy + 'A' + rx + ' ' + ry + ' 0 0 1 ' + cx + ' ' + (cy + ry) +
        'A' + rx + ' ' + ry + ' 0 0 1 ' + (cx - rx) + ' ' + cy +
        'A' + rx + ' ' + ry + ' 0 0 1 ' + cx + ' ' + (cy - ry) +
        'A' + rx + ' ' + ry + ' 0 0 1 ' + (cx + rx) + ' ' + cy + 'Z';
    }
    if (tag === 'rect') {
      var x = attr(el, 'x'), y = attr(el, 'y'), w = attr(el, 'width'), h = attr(el, 'height');
      if (w <= 0 || h <= 0) return '';
      var hasRx = el.hasAttribute('rx'), hasRy = el.hasAttribute('ry');
      var r1 = hasRx ? attr(el, 'rx') : attr(el, 'ry'), r2 = hasRy ? attr(el, 'ry') : r1;
      r1 = Math.min(Math.max(r1, 0), w / 2); r2 = Math.min(Math.max(r2, 0), h / 2);
      if (r1 === 0 || r2 === 0) return 'M' + x + ' ' + y + 'H' + (x + w) + 'V' + (y + h) + 'H' + x + 'Z';
      var a = 'A' + r1 + ' ' + r2 + ' 0 0 1 ';
      return 'M' + (x + r1) + ' ' + y + 'H' + (x + w - r1) + a + (x + w) + ' ' + (y + r2) +
        'V' + (y + h - r2) + a + (x + w - r1) + ' ' + (y + h) + 'H' + (x + r1) + a + x + ' ' + (y + h - r2) +
        'V' + (y + r2) + a + (x + r1) + ' ' + y + 'Z';
    }
    return '';
  }

  // Path data to a list of polylines (one per subpath), each [x0, y0, x1, y1, ...].
  function flatten(d) {
    var i = 0, n = d.length;
    var NUM = /[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/y;
    function skip() { while (i < n && /[\s,]/.test(d[i])) i++; }
    function number() {
      skip(); NUM.lastIndex = i;
      var m = NUM.exec(d);
      if (!m) throw new Error('number');
      i = NUM.lastIndex; return parseFloat(m[0]);
    }
    function flag() {
      skip();
      var c = d[i];
      if (c !== '0' && c !== '1') throw new Error('flag');
      i++; return c === '1' ? 1 : 0;
    }
    function moreNumbers() { skip(); return i < n && /[-+.\d]/.test(d[i]); }

    var lines = [], line = null;
    var x = 0, y = 0, sx = 0, sy = 0, cx = 0, cy = 0, qx = 0, qy = 0, last = '';
    function to(px, py) { line.push(px, py); x = px; y = py; }
    function start(px, py) { line = [px, py]; lines.push(line); x = sx = px; y = sy = py; }
    function cubic(x1, y1, x2, y2, ex, ey) {
      var x0 = x, y0 = y;
      for (var k = 1; k <= CURVE_STEPS; k++) {
        var t = k / CURVE_STEPS, u = 1 - t;
        to(u * u * u * x0 + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t * t * t * ex,
           u * u * u * y0 + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t * t * t * ey);
      }
    }
    function quad(x1, y1, ex, ey) {
      var x0 = x, y0 = y;
      for (var k = 1; k <= CURVE_STEPS; k++) {
        var t = k / CURVE_STEPS, u = 1 - t;
        to(u * u * x0 + 2 * u * t * x1 + t * t * ex, u * u * y0 + 2 * u * t * y1 + t * t * ey);
      }
    }
    // Endpoint to centre parameterization (SVG implementation notes, F.6.5).
    function arc(rx, ry, angle, large, sweep, ex, ey) {
      rx = Math.abs(rx); ry = Math.abs(ry);
      if ((ex === x && ey === y)) return;
      if (rx === 0 || ry === 0) { to(ex, ey); return; }
      var phi = angle * Math.PI / 180, cos = Math.cos(phi), sin = Math.sin(phi);
      var dx = (x - ex) / 2, dy = (y - ey) / 2;
      var x1p = cos * dx + sin * dy, y1p = -sin * dx + cos * dy;
      var lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
      if (lambda > 1) { var s = Math.sqrt(lambda); rx *= s; ry *= s; }
      var num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
      var den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
      var coef = (large === sweep ? -1 : 1) * Math.sqrt(Math.max(0, num / den));
      var cxp = coef * rx * y1p / ry, cyp = -coef * ry * x1p / rx;
      var ccx = cos * cxp - sin * cyp + (x + ex) / 2, ccy = sin * cxp + cos * cyp + (y + ey) / 2;
      function ang(ux, uy, vx, vy) {
        var a = Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
        return a;
      }
      var t1 = ang(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry);
      var dt = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
      if (!sweep && dt > 0) dt -= 2 * Math.PI;
      if (sweep && dt < 0) dt += 2 * Math.PI;
      var steps = Math.max(4, Math.ceil(Math.abs(dt) / (Math.PI / 64)));
      for (var k = 1; k <= steps; k++) {
        var t = t1 + dt * k / steps;
        if (k === steps) { to(ex, ey); break; }
        to(ccx + rx * Math.cos(t) * cos - ry * Math.sin(t) * sin,
           ccy + rx * Math.cos(t) * sin + ry * Math.sin(t) * cos);
      }
    }

    try {
      while (true) {
        skip();
        if (i >= n) break;
        var cmd = d[i];
        if (!/[MmLlHhVvCcSsQqTtAaZz]/.test(cmd)) break;
        i++;
        var rel = cmd === cmd.toLowerCase(), C = cmd.toUpperCase();
        if (C === 'Z') {
          if (line) { to(sx, sy); line = null; }
          last = 'Z';
          continue;
        }
        var first = true;
        do {
          var ox = rel ? x : 0, oy = rel ? y : 0;
          if (!line && C !== 'M') start(x, y);
          if (C === 'M') {
            var mx = number() + ox, my = number() + oy;
            if (first) start(mx, my); else to(mx, my);
          } else if (C === 'L') {
            to(number() + ox, number() + oy);
          } else if (C === 'H') {
            to(number() + ox, y);
          } else if (C === 'V') {
            to(x, number() + oy);
          } else if (C === 'C') {
            var c1x = number() + ox, c1y = number() + oy, c2x = number() + ox, c2y = number() + oy;
            var ex = number() + ox, ey = number() + oy;
            cubic(c1x, c1y, c2x, c2y, ex, ey); cx = c2x; cy = c2y;
          } else if (C === 'S') {
            var r1x = (last === 'C' || last === 'S') ? 2 * x - cx : x, r1y = (last === 'C' || last === 'S') ? 2 * y - cy : y;
            var s2x = number() + ox, s2y = number() + oy, sex = number() + ox, sey = number() + oy;
            cubic(r1x, r1y, s2x, s2y, sex, sey); cx = s2x; cy = s2y;
          } else if (C === 'Q') {
            var q1x = number() + ox, q1y = number() + oy, qex = number() + ox, qey = number() + oy;
            quad(q1x, q1y, qex, qey); qx = q1x; qy = q1y;
          } else if (C === 'T') {
            var t1x = (last === 'Q' || last === 'T') ? 2 * x - qx : x, t1y = (last === 'Q' || last === 'T') ? 2 * y - qy : y;
            var tex = number() + ox, tey = number() + oy;
            quad(t1x, t1y, tex, tey); qx = t1x; qy = t1y;
          } else if (C === 'A') {
            var arx = number(), ary = number(), rot = number(), lg = flag(), sw = flag();
            arc(arx, ary, rot, lg, sw, number() + ox, number() + oy);
          }
          last = C === 'M' ? 'L' : C;
          if (C === 'M') C = 'L';
          first = false;
        } while (moreNumbers());
      }
    } catch (e) { /* stop at the first error, keeping what came before, as renderers do */ }
    return lines;
  }

  function geometry(el) { return flatten(pathDataOf(el)); }

  function totalLength(lines) {
    var len = 0;
    for (var l = 0; l < lines.length; l++) {
      var p = lines[l];
      for (var k = 2; k + 1 < p.length; k += 2) len += Math.hypot(p[k] - p[k - 2], p[k + 1] - p[k - 1]);
    }
    return len;
  }

  function point(x, y) {
    return typeof DOMPoint === 'function' ? new DOMPoint(x, y) : { x: x, y: y, z: 0, w: 1 };
  }

  function rect(x, y, w, h) {
    return typeof DOMRect === 'function' ? new DOMRect(x, y, w, h) : { x: x, y: y, width: w, height: h };
  }

  function define(proto, name, fn) {
    if (typeof proto[name] === 'function') return;
    try { Object.defineProperty(proto, name, { value: fn, writable: true, configurable: true, enumerable: true }); } catch (e) { /* frozen */ }
  }

  var G = SVGGeometryElement.prototype;
  define(G, 'getTotalLength', function getTotalLength() { return totalLength(geometry(this)); });
  define(G, 'getPointAtLength', function getPointAtLength(distance) {
    var lines = geometry(this);
    distance = Number(distance);
    if (!isFinite(distance) || distance < 0) distance = 0;
    var walked = 0, lastX = 0, lastY = 0, any = false;
    for (var l = 0; l < lines.length; l++) {
      var p = lines[l];
      if (p.length >= 2 && !any) { lastX = p[0]; lastY = p[1]; any = true; }
      if (walked >= distance && p.length >= 2 && l > 0) return point(p[0], p[1]);
      for (var k = 2; k + 1 < p.length; k += 2) {
        var seg = Math.hypot(p[k] - p[k - 2], p[k + 1] - p[k - 1]);
        if (walked + seg >= distance && seg > 0) {
          var t = (distance - walked) / seg;
          return point(p[k - 2] + (p[k] - p[k - 2]) * t, p[k - 1] + (p[k + 1] - p[k - 1]) * t);
        }
        walked += seg; lastX = p[k]; lastY = p[k + 1];
      }
    }
    return point(lastX, lastY);
  });

  function boxOf(el, box) {
    if (el instanceof SVGGeometryElement) {
      var lines = geometry(el);
      for (var l = 0; l < lines.length; l++) {
        var p = lines[l];
        for (var k = 0; k + 1 < p.length; k += 2) {
          if (!box) box = { x0: p[k], y0: p[k + 1], x1: p[k], y1: p[k + 1] };
          box.x0 = Math.min(box.x0, p[k]); box.y0 = Math.min(box.y0, p[k + 1]);
          box.x1 = Math.max(box.x1, p[k]); box.y1 = Math.max(box.y1, p[k + 1]);
        }
      }
      return box;
    }
    for (var c = el.firstElementChild; c; c = c.nextElementSibling) {
      if (c.localName === 'defs' || c.localName === 'clipPath' || c.localName === 'mask' || c.localName === 'symbol') continue;
      box = boxOf(c, box);
    }
    return box;
  }

  if (typeof SVGGraphicsElement === 'function') {
    define(SVGGraphicsElement.prototype, 'getBBox', function getBBox() {
      var box = boxOf(this, null);
      return box ? rect(box.x0, box.y0, box.x1 - box.x0, box.y1 - box.y0) : rect(0, 0, 0, 0);
    });
  }
})();
