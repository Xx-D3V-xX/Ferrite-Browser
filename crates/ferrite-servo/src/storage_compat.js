/* Ferrite compatibility script: the Cache API (`caches`).
 *
 * Servo 0.6 hides `caches` behind its service-worker switch, and what is behind
 * it answers one question (`cache.keys()`); `match`, `put`, `add` and the rest do
 * not exist. A site that finds `caches` and then cannot use it breaks, so this
 * defines the whole interface itself, on top of IndexedDB (which the engine has,
 * and which keeps what is stored per site, as the specification asks).
 *
 *   caches.open / has / delete / keys / match
 *   cache.match / matchAll / add / addAll / put / delete / keys
 *
 * A response is stored as its status, status text, headers and body bytes, and
 * comes back as a new `Response`. Like the real thing it is only offered on a
 * secure origin (https, or localhost). It is skipped when the engine has a working
 * `caches` already. What it does not do: a service worker never sees these caches
 * (there are no service workers), opaque responses keep no body, and `url` /
 * `redirected` on a returned response come from what was stored.
 */
(function () {
  'use strict';
  // Runs in a page and, prepended to a service worker's script (`sw_compat.js`), in a worker.
  var G = typeof window !== 'undefined' ? window : (typeof self !== 'undefined' ? self : null);
  if (!G || typeof indexedDB === 'undefined') return;
  if (G.isSecureContext === false) return;
  try {
    if (typeof G.caches === 'object' && G.caches && typeof Cache === 'function' &&
        typeof Cache.prototype.match === 'function') return;
  } catch (e) { /* fall through and define our own */ }

  var DB_NAME = '__ferrite_cache_storage';
  var TOKEN = {};
  var dbPromise = null;

  function openDb() {
    if (dbPromise) return dbPromise;
    dbPromise = new Promise(function (resolve, reject) {
      var req = indexedDB.open(DB_NAME, 1);
      req.onupgradeneeded = function () {
        var db = req.result;
        // One record per cache (its creation number), one per stored response.
        db.createObjectStore('caches', { keyPath: 'name' });
        db.createObjectStore('entries', { keyPath: ['cache', 'url', 'method'] });
      };
      req.onsuccess = function () { resolve(req.result); };
      req.onerror = function () { dbPromise = null; reject(req.error); };
    });
    return dbPromise;
  }

  // Runs `work(stores, finish)` in one transaction; `finish(value)` resolves with
  // the value once the transaction has committed. Requests inside `work` are
  // chained with callbacks, not promises, so the transaction stays active.
  function transact(mode, names, work) {
    return openDb().then(function (db) {
      return new Promise(function (resolve, reject) {
        var tx = db.transaction(names, mode);
        var value;
        var stores = {};
        for (var i = 0; i < names.length; i++) stores[names[i]] = tx.objectStore(names[i]);
        tx.oncomplete = function () { resolve(value); };
        tx.onerror = function () { reject(tx.error || new DOMException('Cache storage failed', 'UnknownError')); };
        tx.onabort = function () { reject(tx.error || new DOMException('Cache storage was aborted', 'AbortError')); };
        try {
          work(stores, function (v) { value = v; }, reject);
        } catch (e) { try { tx.abort(); } catch (e2) { /* done */ } reject(e); }
      });
    });
  }

  function fragmentless(url) {
    var hash = url.indexOf('#');
    return hash < 0 ? url : url.slice(0, hash);
  }
  function searchless(url) {
    var q = url.indexOf('?');
    return q < 0 ? url : url.slice(0, q);
  }

  function toRequest(input, init) {
    if (input instanceof Request) return init ? new Request(input, init) : input;
    return new Request(String(input), init);
  }

  function checkScheme(request) {
    var scheme = new URL(request.url).protocol;
    if (scheme !== 'http:' && scheme !== 'https:') throw new TypeError('Request scheme "' + scheme + '" is unsupported');
  }

  var NULL_BODY = { 101: 1, 204: 1, 205: 1, 304: 1 };

  function headerPairs(headers) {
    var out = [];
    headers.forEach(function (value, name) { out.push([name, value]); });
    return out;
  }

  function responseFrom(entry) {
    var body = NULL_BODY[entry.status] ? null : entry.body;
    var response = new Response(body, {
      status: entry.status === 0 ? 200 : entry.status,
      statusText: entry.statusText,
      headers: entry.headers
    });
    try { Object.defineProperty(response, 'url', { value: entry.url, configurable: true }); } catch (e) { /* frozen */ }
    return response;
  }

  // Whether a stored response may answer `request` (the Vary header, step by step).
  function varyAllows(entry, request, options) {
    if (options && options.ignoreVary) return true;
    var vary = null;
    for (var i = 0; i < entry.headers.length; i++) if (entry.headers[i][0].toLowerCase() === 'vary') vary = entry.headers[i][1];
    if (vary === null) return true;
    var names = vary.split(',');
    for (var j = 0; j < names.length; j++) {
      var name = names[j].trim().toLowerCase();
      if (name === '*') return false;
      if (!name) continue;
      var stored = entry.requestHeaders && Object.prototype.hasOwnProperty.call(entry.requestHeaders, name) ? entry.requestHeaders[name] : null;
      if (stored !== request.headers.get(name)) return false;
    }
    return true;
  }

  function urlMatches(entry, request, options) {
    var wanted = fragmentless(request.url);
    var have = entry.url;
    if (options && options.ignoreSearch) { wanted = searchless(wanted); have = searchless(have); }
    if (wanted !== have) return false;
    if (!(options && options.ignoreMethod) && entry.method !== request.method) return false;
    return varyAllows(entry, request, options);
  }

  // The stored entries of one cache, oldest first.
  function entriesOf(store, cacheName, done) {
    var range = IDBKeyRange.bound([cacheName], [cacheName, []]);
    var req = store.getAll(range);
    req.onsuccess = function () {
      var list = req.result || [];
      list.sort(function (a, b) { return a.seq - b.seq; });
      done(list);
    };
  }

  function cacheMeta(store, name, done) {
    var req = store.get(name);
    req.onsuccess = function () { done(req.result || null); };
  }

  function Cache(token, name) {
    if (token !== TOKEN) throw new TypeError('Illegal constructor');
    Object.defineProperty(this, '__name', { value: name });
  }

  function requestArg(request, options) {
    if (request === undefined) return null;
    var r = request instanceof Request ? request : new Request(String(request));
    if (r.method !== 'GET' && r.method !== 'HEAD' && !(options && options.ignoreMethod)) return 'no-match';
    return r;
  }

  function findAll(cacheName, request, options, onlyFirst) {
    var r = requestArg(request, options);
    return transact('readonly', ['entries'], function (stores, finish) {
      entriesOf(stores.entries, cacheName, function (list) {
        if (r === 'no-match') { finish([]); return; }
        var out = [];
        for (var i = 0; i < list.length; i++) {
          if (r === null || urlMatches(list[i], r, options)) {
            out.push(list[i]);
            if (onlyFirst) break;
          }
        }
        finish(out);
      });
    });
  }

  Cache.prototype.match = function match(request, options) {
    return findAll(this.__name, request, options, true).then(function (found) {
      return found.length ? responseFrom(found[0]) : undefined;
    });
  };
  Cache.prototype.matchAll = function matchAll(request, options) {
    return findAll(this.__name, request, options, false).then(function (found) { return found.map(responseFrom); });
  };

  Cache.prototype.put = function put(request, response) {
    var name = this.__name;
    var r;
    try {
      r = toRequest(request);
      checkScheme(r);
      if (r.method !== 'GET') throw new TypeError('Request method must be GET');
      if (!(response instanceof Response)) throw new TypeError('The second argument must be a Response');
      if (response.status === 206) throw new TypeError('Partial responses cannot be cached');
      var vary = response.headers.get('vary');
      if (vary && vary.split(',').some(function (n) { return n.trim() === '*'; })) throw new TypeError('Responses with "Vary: *" cannot be cached');
      if (response.bodyUsed) throw new TypeError('The response body has already been used');
    } catch (e) { return Promise.reject(e); }
    var copy = response.clone();
    return copy.arrayBuffer().then(function (bytes) {
      var requestHeaders = {};
      var vary2 = response.headers.get('vary');
      if (vary2) vary2.split(',').forEach(function (n) {
        n = n.trim().toLowerCase();
        if (n) requestHeaders[n] = r.headers.get(n);
      });
      return transact('readwrite', ['caches', 'entries'], function (stores, finish, fail) {
        cacheMeta(stores.caches, name, function (meta) {
          if (!meta) { fail(new DOMException('The cache was deleted', 'InvalidStateError')); return; }
          var entry = {
            cache: name, url: fragmentless(r.url), method: r.method,
            status: response.status, statusText: response.statusText,
            headers: headerPairs(response.headers), requestHeaders: requestHeaders,
            body: bytes, seq: ++meta.counter
          };
          stores.caches.put(meta);
          // The same request is replaced in place; it keeps no old position.
          stores.entries.put(entry);
          finish(undefined);
        });
      });
    });
  };

  Cache.prototype.add = function add(request) { return this.addAll([request]); };
  Cache.prototype.addAll = function addAll(requests) {
    var cache = this;
    var list;
    try {
      list = Array.prototype.slice.call(requests).map(function (item) {
        var r = toRequest(item);
        checkScheme(r);
        if (r.method !== 'GET') throw new TypeError('Request method must be GET');
        return r;
      });
    } catch (e) { return Promise.reject(e); }
    return Promise.all(list.map(function (r) {
      return fetch(r).then(function (response) {
        if (!response.ok) throw new TypeError('Request failed (' + response.status + ') for ' + r.url);
        return response;
      });
    })).then(function (responses) {
      return responses.reduce(function (chain, response, i) {
        return chain.then(function () { return cache.put(list[i], response); });
      }, Promise.resolve());
    }).then(function () { return undefined; });
  };

  Cache.prototype['delete'] = function (request, options) {
    var name = this.__name;
    var r = requestArg(request, options);
    if (r === null) return Promise.reject(new TypeError('A request is required'));
    if (r === 'no-match') return Promise.resolve(false);
    return transact('readwrite', ['entries'], function (stores, finish) {
      entriesOf(stores.entries, name, function (list) {
        var hit = list.filter(function (e) { return urlMatches(e, r, options); });
        hit.forEach(function (e) { stores.entries['delete']([e.cache, e.url, e.method]); });
        finish(hit.length > 0);
      });
    });
  };

  Cache.prototype.keys = function keys(request, options) {
    return findAll(this.__name, request, options, false).then(function (found) {
      return found.map(function (e) { return new Request(e.url, { method: e.method }); });
    });
  };

  function CacheStorage(token) {
    if (token !== TOKEN) throw new TypeError('Illegal constructor');
  }
  CacheStorage.prototype.open = function open(name) {
    name = String(name);
    return transact('readwrite', ['caches'], function (stores, finish) {
      cacheMeta(stores.caches, name, function (meta) {
        if (meta) { finish(true); return; }
        // `order` is when the cache was made, so `keys()` lists them in that order.
        stores.caches.put({ name: name, counter: 0, order: Date.now() * 1000 + Math.floor(Math.random() * 1000) });
        finish(true);
      });
    }).then(function () { return new Cache(TOKEN, name); });
  };
  CacheStorage.prototype.has = function has(name) {
    name = String(name);
    return transact('readonly', ['caches'], function (stores, finish) {
      cacheMeta(stores.caches, name, function (meta) { finish(!!meta); });
    });
  };
  CacheStorage.prototype['delete'] = function (name) {
    name = String(name);
    return transact('readwrite', ['caches', 'entries'], function (stores, finish) {
      cacheMeta(stores.caches, name, function (meta) {
        if (!meta) { finish(false); return; }
        stores.caches['delete'](name);
        entriesOf(stores.entries, name, function (list) {
          list.forEach(function (e) { stores.entries['delete']([e.cache, e.url, e.method]); });
          finish(true);
        });
      });
    });
  };
  CacheStorage.prototype.keys = function keys() {
    return transact('readonly', ['caches'], function (stores, finish) {
      var req = stores.caches.getAll();
      req.onsuccess = function () {
        var list = req.result || [];
        list.sort(function (a, b) { return a.order - b.order; });
        finish(list.map(function (c) { return c.name; }));
      };
    });
  };
  CacheStorage.prototype.match = function match(request, options) {
    var wanted = options && options.cacheName !== undefined ? String(options.cacheName) : null;
    return this.keys().then(function (names) {
      if (wanted !== null) names = names.filter(function (n) { return n === wanted; });
      return names.reduce(function (chain, name) {
        return chain.then(function (found) {
          if (found) return found;
          return new Cache(TOKEN, name).match(request, options);
        });
      }, Promise.resolve(undefined));
    });
  };

  var storage = new CacheStorage(TOKEN);
  try {
    Object.defineProperty(G, 'Cache', { value: Cache, writable: true, configurable: true, enumerable: false });
    Object.defineProperty(G, 'CacheStorage', { value: CacheStorage, writable: true, configurable: true, enumerable: false });
    Object.defineProperty(G, 'caches', { get: function () { return storage; }, configurable: true, enumerable: true });
  } catch (e) { /* a frozen global object */ }
})();
