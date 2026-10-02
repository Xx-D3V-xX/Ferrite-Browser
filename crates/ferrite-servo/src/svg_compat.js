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

  // One pass over `svgs`: every read first, then every write. Interleaving them
  // forces a style recalculation per element, which is what makes a naive
  // version of this crawl on a page with many icons.
  function fixBatch(svgs) {
    var plan = [];
    var i, j, k;
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
