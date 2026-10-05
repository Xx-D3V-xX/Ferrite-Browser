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
