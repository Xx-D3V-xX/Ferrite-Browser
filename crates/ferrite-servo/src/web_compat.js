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
 *
 * The same goes for a handful of small interfaces Servo 0.6 lacks that sites call
 * without checking, or check and then take a slower path for: `requestIdleCallback`,
 * `scheduler.postTask`, Web Locks (`navigator.locks`, within one page), `screen.orientation`,
 * `navigator.mediaDevices` (no devices: it says so, and refuses), `document.startViewTransition`
 * (runs the update, shows no transition) and `Element.checkVisibility`. Each is skipped
 * when the engine has the real one.
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
    // `src=""` is a placeholder pages use on purpose (a lazy image waiting for its
    // real address); the engine resolves it to the page's own address, which is
    // not a failure worth reporting.
    var raw = target.getAttribute('src');
    if (raw === null) raw = target.getAttribute('href');
    if (raw === null) raw = target.getAttribute('data');
    if (raw !== null && String(raw).trim() === '') return;
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

(function () {
  'use strict';
  function define(target, name, value) {
    if (!target || typeof target[name] !== 'undefined') return;
    try {
      Object.defineProperty(target, name, { value: value, writable: true, configurable: true, enumerable: false });
    } catch (e) { /* a frozen object */ }
  }
  function abortError() { return new DOMException('The operation was aborted.', 'AbortError'); }

  // requestIdleCallback: run when the page has been quiet for a moment, with the
  // budget the callback's deadline object reports.
  if (typeof window !== 'undefined') {
    var idleId = 0, idleTimers = {};
    define(window, 'requestIdleCallback', function requestIdleCallback(callback, options) {
      var id = ++idleId, start = Date.now();
      var delay = options && typeof options.timeout === 'number' ? Math.min(options.timeout, 50) : 1;
      idleTimers[id] = setTimeout(function () {
        delete idleTimers[id];
        var began = Date.now();
        callback({
          didTimeout: !!(options && typeof options.timeout === 'number' && began - start >= options.timeout),
          timeRemaining: function () { return Math.max(0, 50 - (Date.now() - began)); }
        });
      }, delay);
      return id;
    });
    define(window, 'cancelIdleCallback', function cancelIdleCallback(id) {
      if (idleTimers[id]) { clearTimeout(idleTimers[id]); delete idleTimers[id]; }
    });
  }

  // scheduler.postTask / scheduler.yield: a timer with the task's priority as its delay.
  if (typeof window !== 'undefined' && typeof window.scheduler === 'undefined') {
    var scheduler = {
      postTask: function postTask(callback, options) {
        options = options || {};
        var signal = options.signal;
        return new Promise(function (resolve, reject) {
          if (signal && signal.aborted) { reject(signal.reason || abortError()); return; }
          var delay = Number(options.delay) || (options.priority === 'background' ? 4 : 0);
          var timer = setTimeout(function () {
            try { resolve(callback()); } catch (e) { reject(e); }
          }, delay);
          if (signal && signal.addEventListener) signal.addEventListener('abort', function () {
            clearTimeout(timer);
            reject(signal.reason || abortError());
          });
        });
      },
      yield: function () { return new Promise(function (resolve) { setTimeout(resolve, 0); }); }
    };
    define(window, 'scheduler', scheduler);
  }

  // Web Locks, for the one page: exclusive locks queue, shared ones run together,
  // `ifAvailable`, `steal` and `signal` work. Another tab does not see these locks.
  if (typeof navigator !== 'undefined' && typeof navigator.locks === 'undefined') {
    var held = {}, queues = {};
    var granted = function (name) { return held[name] || (held[name] = []); };
    var pump = function (name) {
      var queue = queues[name] || (queues[name] = []);
      var current = granted(name);
      while (queue.length) {
        var next = queue[0];
        var free = next.mode === 'shared'
          ? current.every(function (l) { return l.mode === 'shared'; })
          : current.length === 0;
        if (!free) break;
        queue.shift();
        run(next, name);
      }
    };
    var run = function (req, name) {
      var lock = { name: name, mode: req.mode };
      granted(name).push(lock);
      var release = function () {
        var list = granted(name), i = list.indexOf(lock);
        if (i >= 0) list.splice(i, 1);
        pump(name);
      };
      lock.release = release;
      var result;
      try { result = Promise.resolve(req.callback(lock)); } catch (e) { result = Promise.reject(e); }
      result.then(function (v) { release(); req.resolve(v); }, function (e) { release(); req.reject(e); });
    };
    var LockManager = function LockManager() { throw new TypeError('Illegal constructor'); };
    LockManager.prototype.request = function request(name, a, b) {
      var options = typeof a === 'function' ? {} : (a || {});
      var callback = typeof a === 'function' ? a : b;
      name = String(name);
      if (typeof callback !== 'function') return Promise.reject(new TypeError('A callback is required'));
      if (name.charAt(0) === '-') return Promise.reject(new DOMException('Names starting with "-" are reserved', 'NotSupportedError'));
      var mode = options.mode === 'shared' ? 'shared' : 'exclusive';
      var signal = options.signal;
      if (signal && signal.aborted) return Promise.reject(signal.reason || abortError());
      return new Promise(function (resolve, reject) {
        var req = { mode: mode, callback: callback, resolve: resolve, reject: reject };
        var current = granted(name), queue = queues[name] || (queues[name] = []);
        var free = queue.length === 0 && (mode === 'shared'
          ? current.every(function (l) { return l.mode === 'shared'; })
          : current.length === 0);
        if (options.steal) {
          current.splice(0, current.length);
          run(req, name);
        } else if (options.ifAvailable && !free) {
          try { resolve(callback(null)); } catch (e) { reject(e); }
        } else {
          queue.push(req);
          if (signal && signal.addEventListener) signal.addEventListener('abort', function () {
            var i = queue.indexOf(req);
            if (i >= 0) { queue.splice(i, 1); reject(signal.reason || abortError()); pump(name); }
          });
          pump(name);
        }
      });
    };
    LockManager.prototype.query = function query() {
      var out = { held: [], pending: [] };
      Object.keys(held).forEach(function (n) { held[n].forEach(function (l) { out.held.push({ name: n, mode: l.mode }); }); });
      Object.keys(queues).forEach(function (n) { queues[n].forEach(function (r) { out.pending.push({ name: n, mode: r.mode }); }); });
      return Promise.resolve(out);
    };
    var manager = Object.create(LockManager.prototype);
    try { Object.defineProperty(navigator, 'locks', { get: function () { return manager; }, configurable: true, enumerable: true }); } catch (e) { /* locked */ }
  }

  // screen.orientation: a landscape desktop screen that cannot be locked.
  if (typeof screen !== 'undefined' && typeof screen.orientation === 'undefined') {
    var orientation = {
      type: 'landscape-primary', angle: 0, onchange: null,
      lock: function () { return Promise.reject(new DOMException('Orientation lock is not supported', 'NotSupportedError')); },
      unlock: function () {},
      addEventListener: function () {}, removeEventListener: function () {}, dispatchEvent: function () { return true; }
    };
    try { Object.defineProperty(screen, 'orientation', { get: function () { return orientation; }, configurable: true, enumerable: true }); } catch (e) { /* locked */ }
  }

  // navigator.mediaDevices: there is no camera or microphone to offer. Pages that
  // call it without checking get an answer ("none") instead of a TypeError.
  if (typeof navigator !== 'undefined' && typeof navigator.mediaDevices === 'undefined' && window.isSecureContext) {
    var devices = {
      enumerateDevices: function () { return Promise.resolve([]); },
      getSupportedConstraints: function () { return {}; },
      getUserMedia: function () { return Promise.reject(new DOMException('Requested device not found', 'NotFoundError')); },
      getDisplayMedia: function () { return Promise.reject(new DOMException('Permission denied', 'NotAllowedError')); },
      ondevicechange: null,
      addEventListener: function () {}, removeEventListener: function () {}, dispatchEvent: function () { return true; }
    };
    try { Object.defineProperty(navigator, 'mediaDevices', { get: function () { return devices; }, configurable: true, enumerable: true }); } catch (e) { /* locked */ }
  }

  // document.startViewTransition: update the page, show no transition.
  if (typeof document !== 'undefined' && typeof document.startViewTransition === 'undefined') {
    define(Document.prototype, 'startViewTransition', function startViewTransition(update) {
      var callback = typeof update === 'function' ? update : (update && update.update);
      var done = new Promise(function (resolve, reject) {
        Promise.resolve().then(function () { return callback ? callback() : undefined; }).then(resolve, reject);
      });
      var finished = done.then(function () { return undefined; });
      finished.catch(function () { /* the page sees it through `updateCallbackDone` */ });
      return { ready: done.then(function () { return undefined; }), updateCallbackDone: done, finished: finished, skipTransition: function () {}, types: new Set() };
    });
  }

  // Element.checkVisibility: rendered, and (with the options) visible and opaque.
  if (typeof Element !== 'undefined') {
    define(Element.prototype, 'checkVisibility', function checkVisibility(options) {
      options = options || {};
      if (!this.isConnected) return false;
      var el = this;
      while (el && el.nodeType === 1) {
        var cs = getComputedStyle(el);
        if (cs.display === 'none') return false;
        if (el === this && (options.checkVisibilityCSS || options.visibilityProperty) && cs.visibility !== 'visible') return false;
        if ((options.checkOpacity || options.opacityProperty) && cs.opacity === '0') return false;
        el = el.parentElement;
      }
      return true;
    });
  }
})();
