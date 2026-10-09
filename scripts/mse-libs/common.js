// Set `?trace` on the page to log every append and what it left buffered.
if (location.search.indexOf('trace') >= 0) (function(){
  var ab = SourceBuffer.prototype.appendBuffer;
  SourceBuffer.prototype.appendBuffer = function(d){
    var sb = this, n = d.byteLength, off = sb.timestampOffset;
    sb.addEventListener('updateend', function f(){ sb.removeEventListener('updateend', f); var r = []; for (var i = 0; i < sb.buffered.length; i++) r.push([+sb.buffered.start(i).toFixed(3), +sb.buffered.end(i).toFixed(3)]); log('INFO append ' + n + 'B off=' + off + ' -> ' + JSON.stringify(r)); });
    return ab.call(this, d);
  };
})();
(function(){ var orig = URL.createObjectURL; URL.createObjectURL = function(o){ if (typeof MediaSource !== 'undefined' && o instanceof MediaSource) window.__ms = o; return orig.call(URL, o); }; })();
function log(s){ console.log(s); }
function ok(name, cond, extra){ log((cond ? 'PASS ' : 'FAIL ') + name + (cond ? '' : ' :: ' + JSON.stringify(extra))); }
function wait(ms){ return new Promise(function(r){ setTimeout(r, ms); }); }
function once(el, type, ms){ return new Promise(function(res){ var t = setTimeout(function(){ res(false); }, ms); el.addEventListener(type, function(){ clearTimeout(t); res(true); }, {once: true}); }); }
// What every library page checks once it has started the stream: it plays, time goes on,
// a seek lands and plays on, and the stream ends.
async function standardChecks(v, duration){
  var playing = v.currentTime > 0 || await once(v, 'playing', 20000);
  ok('playback starts', playing && !v.paused, [v.currentTime, v.paused, v.readyState]);
  ok('the video has its size', v.videoWidth === 320 && v.videoHeight === 240, [v.videoWidth, v.videoHeight]);
  await wait(2500);
  ok('time advances', v.currentTime > 1.5, v.currentTime);
  ok('duration is right', Math.abs(v.duration - duration) < 0.5, v.duration);
  var seeked = once(v, 'seeked', 15000);
  v.currentTime = 8;
  ok('a seek far ahead completes', await seeked, v.currentTime);
  await wait(2500);
  ok('and playback carries on from there', v.currentTime > 8.5 && v.currentTime < duration + 0.5, v.currentTime);
  var ended = v.ended || await once(v, 'ended', 20000);
  var b = []; for (var i = 0; i < v.buffered.length; i++) b.push([v.buffered.start(i), v.buffered.end(i)]);
  if (!ended && window.__ms) { var d = []; for (var k = 0; k < __ms.sourceBuffers.length; k++) { var sb = __ms.sourceBuffers[k]; var r = []; for (var q = 0; q < sb.buffered.length; q++) r.push([sb.buffered.start(q), sb.buffered.end(q)]); d.push({updating: sb.updating, buffered: r}); } log('INFO ms ' + __ms.readyState + ' duration=' + __ms.duration + ' ' + JSON.stringify(d) + ' t=' + v.currentTime); await wait(3000); log('INFO ms after 3s t=' + v.currentTime + ' ready=' + v.readyState); }
  ok('the stream ends', ended, [v.currentTime, v.ended, v.readyState, v.paused, b]);
}
