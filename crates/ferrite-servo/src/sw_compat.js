/* Ferrite compatibility script: service workers.
 *
 * Servo 0.6 has a service-worker switch, and behind it a worker that registers and then
 * never leaves "installing": `navigator.serviceWorker.ready` does not exist, no `install`
 * or `activate` event is delivered, no `fetch` event is dispatched. A site that waits for
 * a worker to activate waits for ever. So the engine's is left off and this defines the
 * interface itself, on top of dedicated workers:
 *
 *   navigator.serviceWorker   register, getRegistration(s), ready, controller,
 *                             controllerchange, message, startMessages
 *   ServiceWorkerRegistration installing / waiting / active, scope, update, unregister,
 *                             updatefound, pushManager (no push), showNotification (refused)
 *   ServiceWorker             scriptURL, state, statechange, postMessage
 *   inside the worker         install, activate, message, fetch events with waitUntil and
 *                             respondWith; skipWaiting, clients (get, matchAll, claim),
 *                             registration, caches (the Cache API of storage_compat.js),
 *                             importScripts relative to the worker's own address
 *
 * What differs from the real thing, because there is no browser-wide worker that outlives
 * the page:
 *  - the worker runs inside the page that registered or opened it. A registration is
 *    remembered (IndexedDB), and every later page in its scope starts the worker again;
 *    two tabs each have their own copy, with their own memory.
 *  - a `fetch` event is delivered for what the page's own `fetch()` asks for. Navigations
 *    and what the page loads by markup (images, scripts, stylesheets, frames) and
 *    `XMLHttpRequest` go to the network as before.
 *  - no push, no background sync, no periodic sync, no notifications from the worker, no
 *    module workers (`type: 'module'` is refused).
 * A page is controlled once its worker is active and the page is in its scope (a page that
 * registered it is controlled only after `clients.claim()` or a reload, as specified).
 */
(function () {
  'use strict';
  if (typeof window === 'undefined' || typeof navigator === 'undefined') return;
  if (!window.isSecureContext || typeof Worker !== 'function' || typeof indexedDB === 'undefined') return;
  if (typeof navigator.serviceWorker !== 'undefined') return; // the engine has its own

  var CACHE_SOURCE = __FERRITE_CACHE_SOURCE__;
  var DB_NAME = '__ferrite_service_workers';
  var TOKEN = {};
  var originalFetch = window.fetch ? window.fetch.bind(window) : null;
  var FETCH_TIMEOUT_MS = 30000;

  // ---- the code that runs inside the worker (written as a function so it is checked
  // with the rest of this file, then sent as text) ----
  function workerMain(CFG) {
    var scope = self;
    var nativeAdd = scope.addEventListener.bind(scope);
    var post = function (m) { scope.postMessage(m); };
    var pendingFetches = {};
    var ctl = { skipWaitingCalled: false };

    function illegal() { throw new TypeError('Illegal constructor'); }

    class ExtendableEvent extends Event {
      constructor(type, init) {
        super(type, init);
        Object.defineProperty(this, '__promises', { value: [] });
        Object.defineProperty(this, '__open', { value: true, writable: true });
      }
      waitUntil(promise) {
        if (!this.__open) throw new DOMException('waitUntil() was called too late', 'InvalidStateError');
        this.__promises.push(Promise.resolve(promise));
      }
    }
    function settle(event) {
      // Promises added while others settle count too, until none is left.
      var seen = 0;
      function next() {
        if (seen >= event.__promises.length) { event.__open = false; return Promise.resolve(); }
        var batch = event.__promises.slice(seen);
        seen = event.__promises.length;
        return Promise.all(batch).then(next);
      }
      return next();
    }

    class FetchEvent extends ExtendableEvent {
      constructor(type, init) {
        super(type, init);
        Object.defineProperty(this, 'request', { value: init.request, enumerable: true });
        Object.defineProperty(this, 'clientId', { value: init.clientId || '', enumerable: true });
        Object.defineProperty(this, 'resultingClientId', { value: '', enumerable: true });
        Object.defineProperty(this, 'replacesClientId', { value: '', enumerable: true });
        Object.defineProperty(this, 'preloadResponse', { value: Promise.resolve(undefined), enumerable: true });
        Object.defineProperty(this, 'handled', { value: Promise.resolve(undefined), enumerable: true });
        Object.defineProperty(this, '__response', { value: null, writable: true });
      }
      respondWith(promise) {
        // Only while the event is being dispatched, and once.
        if (this.eventPhase === 0) throw new DOMException('respondWith() was called too late', 'InvalidStateError');
        if (this.__response) throw new DOMException('respondWith() was already called', 'InvalidStateError');
        this.__response = Promise.resolve(promise);
      }
    }

    class ExtendableMessageEvent extends ExtendableEvent {
      constructor(type, init) {
        super(type, init);
        init = init || {};
        Object.defineProperty(this, 'data', { value: init.data, enumerable: true });
        Object.defineProperty(this, 'origin', { value: init.origin || '', enumerable: true });
        Object.defineProperty(this, 'lastEventId', { value: '', enumerable: true });
        Object.defineProperty(this, 'source', { value: init.source || null, enumerable: true });
        Object.defineProperty(this, 'ports', { value: Object.freeze([]), enumerable: true });
      }
    }

    // The page that started this worker, as a client.
    function WindowClient() { illegal(); }
    function makeClient(info) {
      var c = Object.create(WindowClient.prototype);
      Object.defineProperty(c, 'id', { value: info.id, enumerable: true });
      Object.defineProperty(c, 'url', { value: info.url, enumerable: true });
      Object.defineProperty(c, 'type', { value: 'window', enumerable: true });
      Object.defineProperty(c, 'frameType', { value: 'top-level', enumerable: true });
      Object.defineProperty(c, 'visibilityState', { value: 'visible', enumerable: true });
      Object.defineProperty(c, 'focused', { value: true, enumerable: true });
      c.postMessage = function postMessage(message) { post({ __sw: 'toClient', data: message }); };
      c.focus = function focus() { return Promise.resolve(c); };
      c.navigate = function navigate() { return Promise.reject(new TypeError('navigate() is not supported')); };
      return c;
    }
    var clientInfo = null; // set by the page's `boot` message
    function Clients() { illegal(); }
    var clients = Object.create(Clients.prototype);
    clients.get = function get(id) { return Promise.resolve(clientInfo && clientInfo.id === id ? makeClient(clientInfo) : undefined); };
    clients.matchAll = function matchAll() { return Promise.resolve(clientInfo ? [makeClient(clientInfo)] : []); };
    clients.openWindow = function openWindow() { return Promise.reject(new TypeError('openWindow() is not supported')); };
    clients.claim = function claim() { post({ __sw: 'claim' }); return Promise.resolve(); };

    var registration = Object.create(null);
    function ServiceWorkerRegistration() { illegal(); }
    registration = Object.create(ServiceWorkerRegistration.prototype);
    Object.defineProperty(registration, 'scope', { value: CFG.scope, enumerable: true });
    Object.defineProperty(registration, 'updateViaCache', { value: 'imports', enumerable: true });
    registration.showNotification = function () { return Promise.reject(new TypeError('Notifications are not available')); };
    registration.getNotifications = function () { return Promise.resolve([]); };
    registration.update = function () { return Promise.resolve(registration); };
    registration.unregister = function () { post({ __sw: 'unregister' }); return Promise.resolve(true); };

    function define(name, value) {
      try { Object.defineProperty(scope, name, { value: value, writable: true, configurable: true, enumerable: false }); } catch (e) { /* frozen */ }
    }
    define('ExtendableEvent', ExtendableEvent);
    define('FetchEvent', FetchEvent);
    define('ExtendableMessageEvent', ExtendableMessageEvent);
    define('Clients', Clients);
    define('WindowClient', WindowClient);
    define('ServiceWorkerRegistration', ServiceWorkerRegistration);
    define('clients', clients);
    define('registration', registration);
    define('skipWaiting', function skipWaiting() { ctl.skipWaitingCalled = true; post({ __sw: 'skipWaiting' }); return Promise.resolve(); });
    try {
      Object.defineProperty(scope, 'serviceWorker', { value: { scriptURL: CFG.scriptURL, state: 'activated' }, configurable: true });
    } catch (e) { /* frozen */ }

    // `oninstall = fn` and friends.
    ['install', 'activate', 'fetch', 'push', 'sync', 'notificationclick'].forEach(function (type) {
      var handler = null;
      try {
        Object.defineProperty(scope, 'on' + type, {
          configurable: true, enumerable: true,
          get: function () { return handler; },
          set: function (fn) {
            if (handler) scope.removeEventListener(type, handler);
            handler = typeof fn === 'function' ? fn : null;
            if (handler) nativeAdd(type, handler);
          }
        });
      } catch (e) { /* frozen */ }
    });

    // importScripts: a relative address is relative to the worker's script, not to the
    // blob it runs from.
    var nativeImport = scope.importScripts ? scope.importScripts.bind(scope) : null;
    if (nativeImport) {
      define('importScripts', function importScripts() {
        var urls = [];
        for (var i = 0; i < arguments.length; i++) urls.push(new URL(String(arguments[i]), CFG.scriptURL).href);
        return nativeImport.apply(null, urls);
      });
    }

    // Fetch results go back to the page.
    function bodyOf(response) {
      return response.arrayBuffer().then(function (buffer) { return buffer; });
    }
    function handleFetch(msg) {
      var id = msg.id, r = msg.request;
      var init = { method: r.method, headers: r.headers, mode: r.mode === 'navigate' ? 'same-origin' : r.mode, credentials: r.credentials, redirect: r.redirect, cache: r.cache };
      if (r.body) init.body = r.body;
      var request;
      try { request = new Request(r.url, init); } catch (e) { post({ __sw: 'fetchResult', id: id, passthrough: true }); return; }
      var event = new FetchEvent('fetch', { request: request, clientId: clientInfo ? clientInfo.id : '', cancelable: true });
      scope.dispatchEvent(event);
      if (!event.__response) { post({ __sw: 'fetchResult', id: id, passthrough: true }); return; }
      event.__response.then(function (response) {
        if (!(response instanceof Response)) throw new TypeError('respondWith() needs a Response');
        return bodyOf(response).then(function (buffer) {
          var headers = [];
          response.headers.forEach(function (v, k) { headers.push([k, v]); });
          scope.postMessage({ __sw: 'fetchResult', id: id, response: { status: response.status, statusText: response.statusText, headers: headers, body: buffer, url: response.url } }, [buffer]);
        });
      }).catch(function () { post({ __sw: 'fetchResult', id: id, error: true }); });
      settle(event).catch(function () { /* waitUntil failed: nothing to tell the page */ });
    }

    function lifecycle(type, id) {
      var event = new ExtendableEvent(type, {});
      try { scope.dispatchEvent(event); } catch (e) { post({ __sw: 'lifecycle', id: id, ok: false, error: String(e) }); return; }
      settle(event).then(function () { post({ __sw: 'lifecycle', id: id, ok: true }); },
                         function (e) { post({ __sw: 'lifecycle', id: id, ok: false, error: String(e && e.message || e) }); });
    }

    // Our own messages come first and are not shown to the worker's listeners.
    nativeAdd('message', function (e) {
      var m = e.data;
      if (!m || typeof m !== 'object' || !m.__swPage) return;
      e.stopImmediatePropagation();
      switch (m.__swPage) {
        case 'boot': clientInfo = m.client; post({ __sw: 'booted' }); break;
        case 'install': lifecycle('install', m.id); break;
        case 'activate': lifecycle('activate', m.id); break;
        case 'fetch': handleFetch(m); break;
        case 'message': {
          var ev = new ExtendableMessageEvent('message', { data: m.data, origin: location.origin, source: clientInfo ? makeClient(clientInfo) : null });
          scope.dispatchEvent(ev);
          settle(ev).catch(function () { /* ignored, as for any message */ });
          break;
        }
        default: break;
      }
    });
  }

  // ---- storage of what is registered ----
  var dbPromise = null;
  function openDb() {
    if (dbPromise) return dbPromise;
    dbPromise = new Promise(function (resolve, reject) {
      var req = indexedDB.open(DB_NAME, 1);
      req.onupgradeneeded = function () { req.result.createObjectStore('regs', { keyPath: 'scope' }); };
      req.onsuccess = function () { resolve(req.result); };
      req.onerror = function () { dbPromise = null; reject(req.error); };
    });
    return dbPromise;
  }
  function dbDo(mode, work) {
    return openDb().then(function (db) {
      return new Promise(function (resolve, reject) {
        var tx = db.transaction('regs', mode), value;
        tx.oncomplete = function () { resolve(value); };
        tx.onerror = tx.onabort = function () { reject(tx.error); };
        work(tx.objectStore('regs'), function (v) { value = v; });
      });
    });
  }
  function dbAll() { return dbDo('readonly', function (s, done) { var r = s.getAll(); r.onsuccess = function () { done(r.result || []); }; }); }
  function dbPut(rec) { return dbDo('readwrite', function (s) { s.put(rec); }); }
  function dbDelete(scope) { return dbDo('readwrite', function (s) { s['delete'](scope); }); }

  // ---- the objects pages see ----
  function illegal() { throw new TypeError('Illegal constructor'); }
  function define(target, name, value) {
    try { Object.defineProperty(target, name, { value: value, writable: true, configurable: true, enumerable: false }); } catch (e) { /* frozen */ }
  }

  class ServiceWorker extends EventTarget { constructor(token) { super(); if (token !== TOKEN) illegal(); } }
  class ServiceWorkerRegistration extends EventTarget { constructor(token) { super(); if (token !== TOKEN) illegal(); } }
  class ServiceWorkerContainer extends EventTarget { constructor(token) { super(); if (token !== TOKEN) illegal(); } }

  function handlerProperty(proto, type) {
    var key = '__on' + type;
    Object.defineProperty(proto, 'on' + type, {
      configurable: true, enumerable: true,
      get: function () { return this[key] || null; },
      set: function (fn) {
        if (this[key]) this.removeEventListener(type, this[key]);
        this[key] = typeof fn === 'function' ? fn : null;
        if (this[key]) this.addEventListener(type, this[key]);
      }
    });
  }
  handlerProperty(ServiceWorker.prototype, 'statechange');
  handlerProperty(ServiceWorker.prototype, 'error');
  handlerProperty(ServiceWorkerRegistration.prototype, 'updatefound');
  handlerProperty(ServiceWorkerContainer.prototype, 'controllerchange');
  handlerProperty(ServiceWorkerContainer.prototype, 'message');
  handlerProperty(ServiceWorkerContainer.prototype, 'messageerror');

  // What one registration is, inside this page.
  var regs = {};          // scope -> state
  var container = new ServiceWorkerContainer(TOKEN);
  var controllerState = null; // the regs entry that controls this page
  var clientId = (typeof crypto !== 'undefined' && crypto.randomUUID) ? crypto.randomUUID() : String(Date.now()) + Math.random();
  var readyWaiters = [];

  function fire(target, type, init) {
    var ev = init ? new MessageEvent(type, init) : new Event(type);
    try { target.dispatchEvent(ev); } catch (e) { /* a listener threw */ }
  }

  function makeWorkerObject(state) {
    var w = new ServiceWorker(TOKEN);
    var current = 'installing';
    Object.defineProperty(w, 'scriptURL', { value: state.scriptURL, enumerable: true });
    Object.defineProperty(w, 'state', { get: function () { return current; }, enumerable: true });
    w.postMessage = function postMessage(message) {
      if (current === 'redundant') throw new DOMException('The service worker is redundant', 'InvalidStateError');
      if (state.worker) state.worker.postMessage({ __swPage: 'message', data: message });
    };
    w.__set = function (next) {
      if (current === next) return;
      current = next;
      fire(w, 'statechange');
    };
    return w;
  }

  function makeRegistration(state) {
    var r = new ServiceWorkerRegistration(TOKEN);
    Object.defineProperty(r, 'scope', { value: state.scope, enumerable: true });
    Object.defineProperty(r, 'updateViaCache', { value: 'imports', enumerable: true });
    Object.defineProperty(r, 'installing', { get: function () { return state.installing || null; }, enumerable: true });
    Object.defineProperty(r, 'waiting', { get: function () { return state.waiting || null; }, enumerable: true });
    Object.defineProperty(r, 'active', { get: function () { return state.active || null; }, enumerable: true });
    Object.defineProperty(r, 'navigationPreload', { value: {
      enable: function () { return Promise.resolve(); }, disable: function () { return Promise.resolve(); },
      setHeaderValue: function () { return Promise.resolve(); }, getState: function () { return Promise.resolve({ enabled: false, headerValue: 'true' }); }
    }, enumerable: true });
    Object.defineProperty(r, 'pushManager', { value: {
      getSubscription: function () { return Promise.resolve(null); },
      permissionState: function () { return Promise.resolve('denied'); },
      subscribe: function () { return Promise.reject(new DOMException('Registration failed - permission denied', 'NotAllowedError')); }
    }, enumerable: true });
    r.showNotification = function () { return Promise.reject(new TypeError('Notifications are not available')); };
    r.getNotifications = function () { return Promise.resolve([]); };
    r.update = function update() { return updateRegistration(state).then(function () { return r; }); };
    r.unregister = function unregister() { return unregisterState(state); };
    return r;
  }

  function inScope(scope, url) {
    return url.indexOf(scope) === 0;
  }
  function longestScopeFor(url) {
    var best = null;
    Object.keys(regs).forEach(function (s) {
      if (inScope(s, url) && (best === null || s.length > best.length)) best = s;
    });
    return best;
  }

  function resolveReady() {
    var s = longestScopeFor(location.href);
    if (!s || !regs[s].active) return;
    var list = readyWaiters; readyWaiters = [];
    list.forEach(function (resolve) { resolve(regs[s].registration); });
  }

  function setController(state) {
    var next = state && state.active ? state : null;
    if (controllerState === next) return;
    controllerState = next;
    // Only a page a worker controls has its fetch() routed; every other page keeps the
    // browser's own fetch untouched (anti-abuse scripts treat a replaced fetch as a sign
    // of tampering, and Google's sign-in refused the browser while it was replaced on
    // every page).
    if (next) routeFetches();
    fire(container, 'controllerchange');
  }

  // Start the worker's thread for a registration record; resolves once its script ran.
  function bootWorker(state, text) {
    return new Promise(function (resolve, reject) {
      var cfg = { scriptURL: state.scriptURL, scope: state.scope };
      var source = CACHE_SOURCE + '\n;(' + workerMain.toString() + ')(' + JSON.stringify(cfg) + ');\n' + text;
      var url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
      var worker;
      try { worker = new Worker(url); } catch (e) { reject(e); return; }
      var booted = false;
      worker.onerror = function (e) {
        if (e && e.preventDefault) e.preventDefault();
        if (!booted) { booted = true; reject(new TypeError('The service worker script failed to run: ' + (e && e.message || 'error'))); }
      };
      worker.onmessage = function (e) { onWorkerMessage(state, worker, e.data, function () { if (!booted) { booted = true; resolve(worker); } }); };
      worker.postMessage({ __swPage: 'boot', client: { id: clientId, url: location.href } });
      setTimeout(function () { if (!booted) { booted = true; reject(new TypeError('The service worker script did not start')); } }, 15000);
    });
  }

  var lifecycleWaiters = {};
  var nextId = 1;
  function ask(worker, kind) {
    return new Promise(function (resolve, reject) {
      var id = nextId++;
      lifecycleWaiters[id] = { resolve: resolve, reject: reject };
      worker.postMessage({ __swPage: kind, id: id });
    });
  }

  var fetchWaiters = {};
  function onWorkerMessage(state, worker, m, booted) {
    if (!m || typeof m !== 'object' || !m.__sw) return;
    switch (m.__sw) {
      case 'booted': booted(); break;
      case 'lifecycle': {
        var w = lifecycleWaiters[m.id]; delete lifecycleWaiters[m.id];
        if (w) (m.ok ? w.resolve() : w.reject(new Error(m.error || 'failed')));
        break;
      }
      case 'skipWaiting': state.skipWaiting = true; if (state.waiting) activate(state); break;
      case 'claim': if (state.active && inScope(state.scope, location.href)) setController(state); break;
      case 'unregister': unregisterState(state); break;
      case 'toClient': {
        var from = state.active || state.installing || state.waiting;
        var ev = new MessageEvent('message', { data: m.data, origin: location.origin });
        try { Object.defineProperty(ev, 'source', { value: from }); } catch (e) { /* keep null */ }
        try { container.dispatchEvent(ev); } catch (e) { /* a listener threw */ }
        break;
      }
      case 'fetchResult': {
        var f = fetchWaiters[m.id]; delete fetchWaiters[m.id];
        if (f) f(m);
        break;
      }
      default: break;
    }
  }

  function activate(state) {
    var sw = state.waiting;
    if (!sw) return Promise.resolve();
    state.waiting = null;
    state.active = sw;
    sw.__set('activating');
    return ask(state.worker, 'activate').then(function () {
      sw.__set('activated');
      resolveReady();
      // A page loaded in scope after the worker was active is controlled from the start;
      // the page that registered it is not, until `clients.claim()` (the worker asks).
      if (state.controlOnActivate) setController(state);
    }, function () {
      sw.__set('redundant');
      state.active = null;
    });
  }

  function install(state) {
    var sw = state.installing;
    return ask(state.worker, 'install').then(function () {
      state.installing = null;
      state.waiting = sw;
      sw.__set('installed');
      // Nothing to wait for: the page has no older worker for this scope.
      return activate(state);
    }, function () {
      sw.__set('redundant');
      state.installing = null;
      if (state.worker) { try { state.worker.terminate(); } catch (e) { /* gone */ } }
    });
  }

  function validate(scriptURL, options) {
    var script;
    try { script = new URL(String(scriptURL), location.href); } catch (e) { throw new TypeError('Invalid script URL'); }
    if (script.origin !== location.origin) throw new DOMException('The script must be from the same origin', 'SecurityError');
    if (options && options.type === 'module') throw new TypeError('Module service workers are not supported');
    var maxScope = new URL('./', script).href;
    var scope = options && options.scope !== undefined ? new URL(String(options.scope), location.href).href : maxScope;
    if (new URL(scope).origin !== location.origin) throw new DOMException('The scope must be from the same origin', 'SecurityError');
    scope = scope.replace(/[?#].*$/, '');
    if (scope.indexOf(maxScope) !== 0) {
      throw new DOMException('The path of the provided scope is not under the max scope allowed', 'SecurityError');
    }
    return { scriptURL: script.href.replace(/#.*$/, ''), scope: scope };
  }

  function fetchScript(url) {
    if (!originalFetch) return Promise.reject(new TypeError('fetch is not available'));
    return originalFetch(url, { cache: 'no-store', credentials: 'same-origin' }).then(function (r) {
      if (!r.ok) throw new TypeError('A bad HTTP response code (' + r.status + ') was received when fetching the script');
      var type = (r.headers.get('content-type') || '').toLowerCase();
      if (type && !/javascript|ecmascript|text\/plain|application\/octet-stream|text\/x-/.test(type)) {
        throw new DOMException('The script has an unsupported MIME type (' + type + ')', 'SecurityError');
      }
      return r.text();
    });
  }

  function hashOf(text) {
    var h = 5381;
    for (var i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0;
    return h + ':' + text.length;
  }

  function startState(rec, text, controlOnActivate) {
    var state = regs[rec.scope] || (regs[rec.scope] = { scope: rec.scope, scriptURL: rec.scriptURL, hash: rec.hash });
    state.scriptURL = rec.scriptURL; state.hash = rec.hash;
    state.controlOnActivate = !!controlOnActivate;
    if (!state.registration) state.registration = makeRegistration(state);
    var sw = makeWorkerObject(state);
    var previous = state.worker;
    state.installing = sw;
    return bootWorker(state, text).then(function (worker) {
      state.worker = worker;
      sw.__set('installing');
      if (previous) { try { previous.terminate(); } catch (e) { /* gone */ } }
      return state;
    }, function (e) {
      state.installing = null;
      sw.__set('redundant');
      throw e;
    });
  }

  function registerImpl(scriptURL, options) {
    var parsed;
    try { parsed = validate(scriptURL, options); } catch (e) { return Promise.reject(e); }
    return fetchScript(parsed.scriptURL).then(function (text) {
      var hash = hashOf(text);
      var existing = regs[parsed.scope];
      if (existing && existing.scriptURL === parsed.scriptURL && existing.hash === hash && (existing.active || existing.waiting || existing.installing)) {
        return existing.registration;
      }
      var rec = { scope: parsed.scope, scriptURL: parsed.scriptURL, hash: hash, type: 'classic', updateViaCache: 'imports', installedAt: Date.now() };
      var hadWorker = !!(existing && (existing.active || existing.waiting));
      return startState(rec, text, false).then(function (state) {
        fire(state.registration, 'updatefound');
        dbPut(rec).catch(function () { /* not remembered across pages */ });
        install(state);
        return state.registration;
      });
    });
  }

  function updateRegistration(state) {
    return fetchScript(state.scriptURL).then(function (text) {
      var hash = hashOf(text);
      if (hash === state.hash) return;
      var rec = { scope: state.scope, scriptURL: state.scriptURL, hash: hash, type: 'classic', updateViaCache: 'imports', installedAt: Date.now() };
      return startState(rec, text, !!controllerState && controllerState === state).then(function () {
        fire(state.registration, 'updatefound');
        dbPut(rec).catch(function () { /* not remembered */ });
        install(state);
      });
    });
  }

  function unregisterState(state) {
    delete regs[state.scope];
    [state.installing, state.waiting, state.active].forEach(function (w) { if (w && w.__set) w.__set('redundant'); });
    state.installing = state.waiting = state.active = null;
    if (state.worker) { try { state.worker.terminate(); } catch (e) { /* gone */ } }
    if (controllerState === state) { controllerState = null; fire(container, 'controllerchange'); }
    return dbDelete(state.scope).then(function () { return true; }, function () { return true; });
  }

  // ---- navigator.serviceWorker ----
  Object.defineProperty(container, 'controller', { get: function () { return controllerState ? controllerState.active : null; }, enumerable: true });
  var readyPromise = new Promise(function (resolve) { readyWaiters.push(resolve); });
  Object.defineProperty(container, 'ready', { get: function () { return readyPromise; }, enumerable: true });
  container.register = function register(scriptURL, options) { return registerImpl(scriptURL, options); };
  container.getRegistration = function getRegistration(clientURL) {
    var url;
    try { url = new URL(clientURL === undefined || clientURL === '' ? location.href : String(clientURL), location.href).href; }
    catch (e) { return Promise.reject(new TypeError('Invalid URL')); }
    var s = longestScopeFor(url);
    return Promise.resolve(s ? regs[s].registration : undefined);
  };
  container.getRegistrations = function getRegistrations() {
    return Promise.resolve(Object.keys(regs).map(function (s) { return regs[s].registration; }));
  };
  container.startMessages = function startMessages() {};
  Object.defineProperty(Navigator.prototype, 'serviceWorker', { get: function () { return container; }, configurable: true, enumerable: true });
  define(window, 'ServiceWorker', ServiceWorker);
  define(window, 'ServiceWorkerRegistration', ServiceWorkerRegistration);
  define(window, 'ServiceWorkerContainer', ServiceWorkerContainer);

  // ---- fetch events for what the page asks for (installed once a worker controls it) ----
  var fetchRouted = false;
  function routeFetches() {
    if (fetchRouted || !originalFetch) return;
    fetchRouted = true;
    var routed = function fetch(input, init) {
      var state = controllerState;
      if (!state || !state.active || !state.worker) return originalFetch(input, init);
      var request;
      try { request = new Request(input, init); } catch (e) { return originalFetch(input, init); }
      // The worker's own script and the registration's are never routed back to it.
      if (request.url === state.scriptURL) return originalFetch(request);
      var network = request.clone();
      return request.arrayBuffer().then(function (body) {
        var id = nextId++;
        var headers = [];
        request.headers.forEach(function (v, k) { headers.push([k, v]); });
        return new Promise(function (resolve, reject) {
          var done = false;
          var timer = setTimeout(function () { if (!done) { done = true; delete fetchWaiters[id]; resolve(originalFetch(network)); } }, FETCH_TIMEOUT_MS);
          fetchWaiters[id] = function (m) {
            if (done) return; done = true; clearTimeout(timer);
            if (m.passthrough) { resolve(originalFetch(network)); return; }
            if (m.error || !m.response) { reject(new TypeError('Failed to fetch')); return; }
            var res = m.response;
            var nullBody = res.status === 204 || res.status === 205 || res.status === 304;
            var out = new Response(nullBody ? null : res.body, { status: res.status, statusText: res.statusText, headers: res.headers });
            try { Object.defineProperty(out, 'url', { value: res.url || request.url }); } catch (e) { /* read-only */ }
            resolve(out);
          };
          state.worker.postMessage({ __swPage: 'fetch', id: id, request: {
            url: request.url, method: request.method, headers: headers, mode: request.mode,
            credentials: request.credentials, redirect: request.redirect, cache: request.cache,
            body: body.byteLength ? body : null
          } });
        });
      });
    };
    Object.defineProperty(window, 'fetch', { value: routed, writable: true, configurable: true, enumerable: true });
  }

  // ---- registrations from earlier pages: the worker starts again for this page ----
  function resume() {
    dbAll().then(function (list) {
      list.forEach(function (rec) {
        if (regs[rec.scope]) return;
        // Only what applies to this page is started (others when a page needs them).
        var relevant = inScope(rec.scope, location.href);
        if (!relevant) {
          var state = regs[rec.scope] = { scope: rec.scope, scriptURL: rec.scriptURL, hash: rec.hash, dormant: true };
          state.registration = makeRegistration(state);
          return;
        }
        fetchScript(rec.scriptURL).then(function (text) {
          var current = rec;
          if (hashOf(text) !== rec.hash) current = { scope: rec.scope, scriptURL: rec.scriptURL, hash: hashOf(text), type: 'classic', updateViaCache: 'imports', installedAt: Date.now() };
          return startState(current, text, true).then(function (state) {
            if (current !== rec) {
              // The script changed since it was installed: it goes through install again.
              dbPut(current).catch(function () { /* not remembered */ });
              return install(state);
            }
            // Installed and activated by an earlier page: this one only starts it again,
            // as a browser restarts a worker, with no install or activate event.
            var sw = state.installing;
            state.installing = null;
            state.active = sw;
            sw.__set('activated');
            setController(state);
            resolveReady();
          });
        }).catch(function () { /* the script is gone or broken: the page is simply not controlled */ });
      });
    }, function () { /* no IndexedDB: nothing remembered */ });
  }
  resume();
})();
