//! Checks, in a real headless Servo session, that the Web APIs benchmark and
//! framework bundles (Speedometer 3.x among them) assume are present. Written
//! after Speedometer's Next.js suite died on `crypto.getRandomValues() not
//! supported`: the `servo` crate was built without its `webcrypto` feature, so
//! `window.crypto` did not exist at all.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo --example web_api_probe
//! ```
//!
//! The page is served from loopback (a secure context, which `randomUUID` and
//! `crypto.subtle` require; a `data:` URL is not one). No outside network or
//! window is needed. Exits non-zero when a *required* API is missing; APIs only
//! reported as `INFO` are listed so a gap is visible but do not fail the run.

use ferrite_servo::session::{HeadlessServoSession, LoadStatus};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// Each entry is `(name, expression)`; the expression must evaluate to a truthy
/// value when the API works. All run synchronously inside one try/catch.
const REQUIRED: &[(&str, &str)] = &[
    ("window.crypto", "typeof crypto === 'object' && crypto === window.crypto"),
    (
        "crypto.getRandomValues(Uint8Array)",
        "(function(){var a=new Uint8Array(32);crypto.getRandomValues(a);return a.some(function(x){return x!==0})})()",
    ),
    (
        "crypto.getRandomValues(Uint32Array)",
        "(function(){var a=new Uint32Array(8);return crypto.getRandomValues(a)===a})()",
    ),
    (
        "crypto.randomUUID()",
        "/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(crypto.randomUUID())",
    ),
    ("crypto.subtle present", "typeof crypto.subtle === 'object' && crypto.subtle !== null"),
    ("self.isSecureContext", "self.isSecureContext === true"),
    ("TextEncoder / TextDecoder", "new TextDecoder().decode(new TextEncoder().encode('é')) === 'é'"),
    ("structuredClone", "structuredClone({a:[1,{b:2}]}).a[1].b === 2"),
    ("queueMicrotask", "typeof queueMicrotask === 'function'"),
    ("MutationObserver", "typeof MutationObserver === 'function'"),
    ("ResizeObserver", "typeof ResizeObserver === 'function'"),
    ("IntersectionObserver", "typeof IntersectionObserver === 'function'"),
    ("customElements", "typeof customElements === 'object'"),
    ("Shadow DOM", "!!document.createElement('div').attachShadow({mode:'open'})"),
    ("performance.now/mark/measure", "(function(){performance.mark('a');performance.mark('b');performance.measure('m','a','b');return typeof performance.now()==='number'})()"),
    ("requestAnimationFrame", "typeof requestAnimationFrame === 'function'"),
    ("AbortController", "typeof AbortController === 'function' && typeof AbortSignal === 'function'"),
    ("URL / URLSearchParams", "new URL('https://a.b/c?d=1').searchParams.get('d') === '1'"),
    ("fetch / Headers / Request", "typeof fetch === 'function' && typeof Headers === 'function' && typeof Request === 'function'"),
    ("Intl.DateTimeFormat / NumberFormat", "typeof Intl.DateTimeFormat === 'function' && typeof Intl.NumberFormat === 'function'"),
    ("Promise.allSettled / WeakRef", "typeof Promise.allSettled === 'function' && typeof WeakRef === 'function'"),
    ("Array.prototype.at / Object.hasOwn", "[1,2,3].at(-1) === 3 && Object.hasOwn({a:1},'a')"),
    ("localStorage / sessionStorage", "typeof localStorage === 'object' && typeof sessionStorage === 'object'"),
    ("history.pushState", "typeof history.pushState === 'function'"),
    ("matchMedia", "typeof matchMedia === 'function' && typeof matchMedia('(min-width:1px)').matches === 'boolean'"),
    ("getComputedStyle", "typeof getComputedStyle(document.body).display === 'string'"),
];

/// Present in current Chrome, Firefox and Safari and used by real sites, but
/// not needed to run a page: a gap is reported as `INFO`, never as a failure.
const OPTIONAL: &[(&str, &str)] = &[
    (
        "requestIdleCallback",
        "typeof requestIdleCallback === 'function'",
    ),
    ("adoptedStyleSheets", "'adoptedStyleSheets' in document"),
    ("FontFace", "typeof FontFace === 'function'"),
    (
        "scheduler.postTask",
        "typeof scheduler === 'object' && typeof scheduler.postTask === 'function'",
    ),
    (
        "navigator.sendBeacon",
        "typeof navigator.sendBeacon === 'function'",
    ),
    (
        "BroadcastChannel / MessageChannel",
        "typeof BroadcastChannel === 'function' && typeof MessageChannel === 'function'",
    ),
    ("Worker", "typeof Worker === 'function'"),
    ("WebSocket", "typeof WebSocket === 'function'"),
    ("OffscreenCanvas", "typeof OffscreenCanvas === 'function'"),
    (
        "CSS.supports",
        "typeof CSS === 'object' && CSS.supports('display','grid')",
    ),
    (
        "ElementInternals (form-associated custom elements)",
        "typeof ElementInternals === 'function'",
    ),
    ("Intl.Segmenter", "typeof Intl.Segmenter === 'function'"),
    ("Navigation API", "typeof navigation === 'object'"),
];

/// Asynchronous checks: each is a `(name, promise-expression)`; the page awaits
/// all of them and publishes `window.__async = {name: "ok" | "<error>"}`.
const ASYNC: &[(&str, &str)] = &[
    (
        "crypto.subtle.digest(SHA-256)",
        "crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc')).then(function(b){\
         var h=Array.from(new Uint8Array(b)).map(function(x){return ('0'+x.toString(16)).slice(-2)}).join('');\
         if(h!=='ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')throw new Error('wrong digest '+h);})",
    ),
    (
        "crypto.subtle.generateKey+encrypt/decrypt (AES-GCM)",
        "crypto.subtle.generateKey({name:'AES-GCM',length:256},true,['encrypt','decrypt']).then(function(k){\
         var iv=crypto.getRandomValues(new Uint8Array(12));\
         return crypto.subtle.encrypt({name:'AES-GCM',iv:iv},k,new TextEncoder().encode('hi')).then(function(c){\
         return crypto.subtle.decrypt({name:'AES-GCM',iv:iv},k,c)}).then(function(p){\
         if(new TextDecoder().decode(p)!=='hi')throw new Error('round trip failed');})})",
    ),
    (
        "crypto.subtle.sign/verify (HMAC)",
        "crypto.subtle.generateKey({name:'HMAC',hash:'SHA-256'},true,['sign','verify']).then(function(k){\
         var d=new TextEncoder().encode('x');\
         return crypto.subtle.sign('HMAC',k,d).then(function(s){return crypto.subtle.verify('HMAC',k,s,d)})}).then(function(v){\
         if(v!==true)throw new Error('verify returned '+v);})",
    ),
    (
        "fetch() of a same-origin resource",
        "fetch('/ping').then(function(r){return r.text()}).then(function(t){if(t!=='pong')throw new Error('got '+t);})",
    ),
    (
        "setTimeout/Promise ordering",
        "new Promise(function(res){var o=[];setTimeout(function(){o.push('t');if(o.join('')==='mt')res();else throw new Error(o.join(''))},0);Promise.resolve().then(function(){o.push('m')})})",
    ),
];

fn page() -> String {
    let mut sync = String::from("var R={};\n");
    for (name, expr) in REQUIRED {
        sync.push_str(&format!(
            "try{{R[{name:?}]=!!({expr})?'ok':'falsy'}}catch(e){{R[{name:?}]='threw: '+e}}\n"
        ));
    }
    sync.push_str("var O={};\n");
    for (name, expr) in OPTIONAL {
        sync.push_str(&format!(
            "try{{O[{name:?}]=!!({expr})?'ok':'missing'}}catch(e){{O[{name:?}]='missing: '+e}}\n"
        ));
    }
    let mut asyncs = String::from("var A={};var ps=[];\n");
    for (name, expr) in ASYNC {
        asyncs.push_str(&format!(
            "ps.push((function(){{try{{return Promise.resolve({expr}).then(function(){{A[{name:?}]='ok'}},function(e){{A[{name:?}]='rejected: '+e}})}}catch(e){{A[{name:?}]='threw: '+e;return Promise.resolve()}}}})());\n"
        ));
    }
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>web api probe</title></head><body>\
         <script>{sync}{asyncs}window.__sync=R;window.__optional=O;Promise.all(ps).then(function(){{window.__async=A}});</script>\
         </body></html>"
    )
}

/// A one-page loopback server: `/` is the probe page, `/ping` answers `pong`.
fn serve(listener: TcpListener, page: String) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let (ctype, body) = if request.starts_with("GET /ping") {
            ("text/plain", "pong".to_string())
        } else {
            ("text/html; charset=utf-8", page.clone())
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn spin_for(session: &mut HeadlessServoSession, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < end {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn js(session: &mut HeadlessServoSession, script: &str) -> String {
    session
        .execute_js(script)
        .unwrap_or_else(|e| format!("<err {e}>"))
}

/// The `{name: result}` object published under `window.<var>`, as pairs. The
/// page flattens it to `name=>verdict~~name=>verdict` because the session hands
/// a string back Debug-quoted, so control characters would arrive escaped.
fn results(session: &mut HeadlessServoSession, var: &str) -> Vec<(String, String)> {
    let raw = js(
        session,
        &format!(
            "(function(){{var o=window.{var};if(!o)return '';\
             return Object.keys(o).map(function(k){{return k+'=>'+String(o[k]).replace(/[\"~]/g,\"'\")}}).join('~~')}})()"
        ),
    );
    // `execute_js` returns the value Debug-formatted: String("...").
    raw.trim_start_matches("String(\"")
        .trim_end_matches("\")")
        .trim_matches('"')
        .split("~~")
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let (k, v) = r.split_once("=>")?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            println!("FAIL loopback server: {e}");
            std::process::exit(2);
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let body = page();
    std::thread::spawn(move || serve(listener, body));

    let mut session = match HeadlessServoSession::new(900, 600) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL session: {e}");
            std::process::exit(2);
        }
    };
    spin_for(&mut session, 200);
    let url = format!("http://127.0.0.1:{port}/");
    session.navigate(&url);
    let end = Instant::now() + Duration::from_secs(20);
    let mut loaded = false;
    while Instant::now() < end {
        session.spin();
        if *session.load_status() == LoadStatus::Complete
            && session.current_url().contains("127.0.0.1")
        {
            loaded = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    if !loaded {
        println!("FAIL page did not load");
        std::process::exit(1);
    }
    // The async checks finish on their own; wait for them to publish.
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end && !js(&mut session, "String(!!window.__async)").contains("true") {
        spin_for(&mut session, 100);
    }

    let mut failed = 0;
    for (name, verdict) in results(&mut session, "__sync")
        .into_iter()
        .chain(results(&mut session, "__async"))
    {
        let ok = verdict == "ok";
        if !ok {
            failed += 1;
        }
        println!(
            "{} {name}{}",
            if ok { "PASS" } else { "FAIL" },
            if ok {
                String::new()
            } else {
                format!(": {verdict}")
            }
        );
    }
    for (name, verdict) in results(&mut session, "__optional") {
        println!(
            "INFO {name}: {}",
            if verdict == "ok" {
                "present"
            } else {
                "missing"
            }
        );
    }
    if !js(&mut session, "String(!!window.__async)").contains("true") {
        println!("FAIL async checks never finished");
        failed += 1;
    }
    println!(
        "{}",
        if failed == 0 {
            "ALL PASS"
        } else {
            "SOME FAILED"
        }
    );
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
