// The service worker the probe registers.
self.addEventListener('install', function (e) { e.waitUntil(new Promise(function (r) { setTimeout(r, 50); })); });
self.addEventListener('activate', function (e) { e.waitUntil(Promise.resolve()); });
self.addEventListener('message', function (e) {
  if (e.data === 'claim') { e.waitUntil(self.clients.claim()); return; }
  if (e.data === 'who') {
    e.waitUntil(self.clients.matchAll().then(function (list) {
      e.source.postMessage('client:' + list.length + ':' + (list[0] && list[0].url));
    }));
    return;
  }
  e.source.postMessage('echo:' + e.data);
});
self.addEventListener('fetch', function (e) {
  var url = new URL(e.request.url);
  if (url.pathname === '/virtual.txt') {
    e.respondWith(new Response('from-sw', { status: 201, headers: { 'x-from': 'sw' } }));
  } else if (url.pathname === '/echo') {
    e.respondWith(e.request.text().then(function (t) { return new Response(t.toUpperCase()); }));
  } else if (url.pathname === '/cache-me.txt') {
    e.respondWith(caches.open('t').then(function (c) {
      return c.match(e.request).then(function (hit) {
        if (hit) return hit;
        return fetch(e.request).then(function (r) { c.put(e.request, r.clone()); return r; });
      });
    }));
  }
});
