/* Ferrite compatibility script: web interfaces Servo does not define.
 *
 * Google's sign-in bundle reads `className` on any element and, to tell an
 * SVG's animated string from a plain one, tests `x instanceof SVGAnimatedString`.
 * Servo has no such interface, so the test throws "SVGAnimatedString is not
 * defined" and the sign-in page's script stops. In Servo `className` on an SVG
 * element is already a plain string, so the right answer to that test is
 * "no, it is not one": a constructor nothing is an instance of. It adds no
 * behaviour, only a name; it is skipped when the engine defines the real one.
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
