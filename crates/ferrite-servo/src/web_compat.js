/* Ferrite compatibility script: web interfaces Servo does not define.
 *
 * Google's sign-in bundle reads `className` on any element and, to tell an
 * SVG's animated string from a plain one, tests `x instanceof SVGAnimatedString`.
 * Servo has no such interface, so the test throws "SVGAnimatedString is not
 * defined" and the sign-in page's script stops. In Servo `className` on an SVG
 * element is already a plain string, so the right answer to that test is
 * "no, it is not one": a constructor nothing is an instance of. It adds no
 * behaviour, only a name; it is skipped when the engine defines the real one.
 *
 * It also reports, to the page's console, what the engine does not say on its
 * own: an image, script, stylesheet, media file or font that failed to load
 * (with its address), and a promise that was rejected and never handled. A page
 * whose icons show as empty or `?` boxes, or whose app never starts, then says
 * why in the Console tab.
 *
 * Servo (0.6) has no Web Animations API: `element.animate` is not a function, and
 * Google's results page stops on "a.animate is not a function". The stand-in
 * below runs no animation; it keeps the promise of one. `animate()` returns an
 * object that finishes after the delay and duration (firing `onfinish` and
 * resolving `finished`), and a `fill: forwards` or `both` animation leaves the
 * last keyframe's values on the element. It is skipped when the engine has the
 * real one.
 */
(function () {
  'use strict';
  if (typeof window.SVGAnimatedString === 'undefined') {
    try {
      Object.defineProperty(window, 'SVGAnimatedString', {
        value: function SVGAnimatedString() { throw new TypeError('Illegal constructor'); },
        writable: true, configurable: true, enumerable: false
      });
    } catch (e) { /* a page that froze the global object already ran */ }
  }
})();

(function () {
  'use strict';
  var seen = {};
  var count = 0;
  function say(message) {
    if (count >= 200) return;
    if (seen[message]) return;
    seen[message] = true;
    count++;
    try { console.warn('Ferrite: ' + message); } catch (e) { /* no console */ }
  }
  function brief(text) {
    text = String(text);
    return text.length > 160 ? text.slice(0, 160) + '...' : text;
  }
  var KINDS = { img: 1, script: 1, link: 1, source: 1, video: 1, audio: 1, track: 1, iframe: 1, object: 1, embed: 1 };
  // A resource's error event does not bubble, but the capture phase sees it.
  window.addEventListener('error', function (event) {
    var target = event && event.target;
    if (!target || target === window || !target.tagName) return;
    var tag = String(target.tagName).toLowerCase();
    if (!KINDS[tag]) return;
    say(tag + ' failed to load: ' + brief(target.currentSrc || target.src || target.href || target.data || '(no address)'));
  }, true);
  window.addEventListener('unhandledrejection', function (event) {
    var reason = event && event.reason;
    say('unhandled promise rejection: ' + brief(reason && reason.message ? reason.message : reason));
  });
  try {
    if (document.fonts && document.fonts.addEventListener) {
      document.fonts.addEventListener('loadingerror', function (event) {
        var faces = (event && event.fontfaces) || [];
        for (var i = 0; i < faces.length; i++) say('font failed to load: ' + brief(faces[i].family));
        if (!faces.length) say('a font failed to load');
      });
    }
  } catch (e) { /* no FontFaceSet events */ }
})();

(function () {
  'use strict';
  if (typeof Element === 'undefined' || typeof Element.prototype.animate === 'function') return;
  var SKIP = { offset: 1, easing: 1, composite: 1, computedOffset: 1 };

  // The values the last keyframe holds, as { cssProperty: value }.
  function endValues(keyframes) {
    var out = {}, i, k, v;
    if (!keyframes) return out;
    if (Array.prototype.isPrototypeOf(keyframes) || typeof keyframes.length === 'number') {
      var last = keyframes[keyframes.length - 1];
      if (last) for (k in last) if (!SKIP[k]) out[k] = last[k];
    } else {
      for (k in keyframes) {
        if (SKIP[k]) continue;
        v = keyframes[k];
        out[k] = (typeof v === 'object' && v && typeof v.length === 'number') ? v[v.length - 1] : v;
      }
    }
    return out;
  }

  function Animation(target, keyframes, options) {
    var self = this;
    var opts = typeof options === 'number' ? { duration: options } : (options || {});
    var duration = Number(opts.duration) || 0;
    var delay = Number(opts.delay) || 0;
    var iterations = opts.iterations === undefined ? 1 : Number(opts.iterations);
    var endDelay = Number(opts.endDelay) || 0;
    var fill = opts.fill || 'none';
    var values = endValues(keyframes);
    var timer = 0;
    var done, fail;
    this.id = opts.id || '';
    this.effect = { target: target, getTiming: function () { return opts; }, getComputedTiming: function () { return opts; } };
    this.timeline = null;
    this.playState = 'running';
    this.pending = false;
    this.playbackRate = 1;
    this.currentTime = 0;
    this.startTime = null;
    this.onfinish = null;
    this.oncancel = null;
    this.onremove = null;
    this.replaceState = 'active';
    this.ready = Promise.resolve(this);
    this.finished = new Promise(function (resolve, reject) { done = resolve; fail = reject; });
    this.finished.catch(function () { /* a cancelled animation is not a page error */ });

    function settle() {
      timer = 0;
      if (self.playState !== 'running') return;
      self.playState = 'finished';
      self.currentTime = delay + duration * (iterations === Infinity ? 1 : iterations) + endDelay;
      if (fill === 'forwards' || fill === 'both') {
        try { for (var k in values) target.style[k] = values[k]; } catch (e) { /* a detached or odd element */ }
      }
      var event = { type: 'finish', target: self, currentTime: self.currentTime };
      try { if (typeof self.onfinish === 'function') self.onfinish(event); } catch (e) { setTimeout(function () { throw e; }, 0); }
      done(self);
    }
    function start() {
      if (iterations === Infinity) return; // never finishes, as the real one
      var total = delay + duration * iterations + endDelay;
      timer = setTimeout(settle, Math.max(0, total));
    }
    this.finish = function () { if (timer) clearTimeout(timer); settle(); };
    this.cancel = function () {
      if (timer) clearTimeout(timer);
      timer = 0;
      if (self.playState === 'idle') return;
      self.playState = 'idle';
      try { if (typeof self.oncancel === 'function') self.oncancel({ type: 'cancel', target: self }); } catch (e) { /* ignore */ }
      fail(new DOMException('The animation was aborted.', 'AbortError'));
    };
    this.pause = function () { if (timer) clearTimeout(timer); timer = 0; if (self.playState === 'running') self.playState = 'paused'; };
    this.play = function () { if (self.playState === 'paused') { self.playState = 'running'; start(); } };
    this.reverse = function () { self.finish(); };
    this.persist = function () {};
    this.commitStyles = function () { try { for (var k in values) target.style[k] = values[k]; } catch (e) { /* ignore */ } };
    this.updatePlaybackRate = function (rate) { self.playbackRate = rate; };
    this.addEventListener = function (type, fn) {
      if (type === 'finish' && !self.onfinish) self.onfinish = fn;
      else if (type === 'cancel' && !self.oncancel) self.oncancel = fn;
    };
    this.removeEventListener = function () {};
    start();
  }

  try {
    Object.defineProperty(Element.prototype, 'animate', {
      value: function animate(keyframes, options) { return new Animation(this, keyframes, options); },
      writable: true, configurable: true, enumerable: false
    });
    if (typeof Element.prototype.getAnimations !== 'function') {
      Object.defineProperty(Element.prototype, 'getAnimations', {
        value: function getAnimations() { return []; }, writable: true, configurable: true, enumerable: false
      });
    }
    if (typeof document !== 'undefined' && typeof document.getAnimations !== 'function') {
      Object.defineProperty(document, 'getAnimations', {
        value: function getAnimations() { return []; }, writable: true, configurable: true, enumerable: false
      });
    }
  } catch (e) { /* a frozen prototype */ }
})();
