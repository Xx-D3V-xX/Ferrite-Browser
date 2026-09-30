//! Runs the real page script (`page_ops.js`) inside a headless Servo session
//! and drives a small form through the same `BrowserEngine` calls the agent
//! uses: read the numbered element table, type into a field by `@ref`, tick a
//! checkbox, choose an option, click a button, scroll to an element.
//!
//! ```text
//! cargo run -p ferrite-engine-servo --features engine-servo --example digest_probe
//! ```
//!
//! Needs the real Servo build. The page is served from a loopback listener
//! started here (the engine only admits http/https origins), so no internet or
//! window is needed.
//! Exits non-zero when a check fails.

use ferrite_engine::digest::RenderBudget;
use ferrite_engine::{BrowserEngine, DigestElement};
use ferrite_engine_servo::BorrowedServoEngine;
use ferrite_servo::session::{HeadlessServoSession, LoadStatus};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const W: u32 = 1000;
const H: u32 = 700;

const PAGE: &str = "<!doctype html><html><head><meta charset=utf-8><title>Checkout</title>\
<style>body{font-family:sans-serif;margin:20px}.pad{height:2400px}input,select,button,textarea{font-size:16px;margin:6px;padding:4px}</style></head>\
<body><h1>Checkout</h1><form id=f onsubmit='window.submitted=(window.submitted||0)+1;return false'>\
<label>Email <input id=email type=email placeholder='you@x.com'></label><br>\
<label>Password <input id=pw type=password value='hunter2'></label><br>\
<label>Country <select id=country><option value=us>United States</option><option value=de>Germany</option></select></label><br>\
<label><input id=terms type=checkbox> I accept the terms</label><br>\
<label>Notes <textarea id=notes></textarea></label><br>\
<button id=go type=submit>Place order</button></form>\
<a href='https://example.org/help'>Help centre</a>\
<div class=pad>tall</div><button id=far onclick='window.far=1'>Far button</button></body></html>";

fn spin_for(session: &mut HeadlessServoSession, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < end {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Serves `PAGE` to every request on an ephemeral loopback port.
fn serve_page() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = PAGE.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    format!("http://127.0.0.1:{port}/")
}

fn check(name: &str, ok: bool, detail: &str) -> bool {
    println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn find<'a>(elements: &'a [DigestElement], role: &str, label: &str) -> Option<&'a DigestElement> {
    elements
        .iter()
        .find(|e| e.role == role && e.label.to_lowercase().contains(&label.to_lowercase()))
}

fn main() {
    let mut session = match HeadlessServoSession::new(W, H) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL session: {e}");
            std::process::exit(2);
        }
    };
    spin_for(&mut session, 200);
    let url = serve_page();
    session.navigate(&url);
    let end = Instant::now() + Duration::from_secs(20);
    while !(*session.load_status() == LoadStatus::Complete
        && session.current_url().starts_with(&url))
    {
        if Instant::now() > end {
            println!("FAIL load: page did not finish loading");
            std::process::exit(1);
        }
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
    spin_for(&mut session, 300);

    let mut ok = true;
    let mut engine = BorrowedServoEngine::new(&mut session, W, H);

    let digest = match engine.page_digest() {
        Ok((d, _)) => d,
        Err(e) => {
            println!("FAIL page_digest: {e:?}");
            std::process::exit(1);
        }
    };
    println!("--- digest ---\n{}\n--------------", digest.render(RenderBudget::default()));
    ok &= check(
        "digest",
        digest.title == "Checkout" && digest.elements.len() >= 7,
        &format!("title {:?}, {} elements", digest.title, digest.elements.len()),
    );
    let pw = digest.elements.iter().find(|e| e.sensitive);
    ok &= check(
        "password never read",
        pw.is_some_and(|e| e.value.is_none()),
        "sensitive field has no value in the digest",
    );

    let refs = (
        find(&digest.elements, "textbox", "email").map(|e| e.ref_id),
        find(&digest.elements, "checkbox", "terms").map(|e| e.ref_id),
        find(&digest.elements, "combobox", "country").map(|e| e.ref_id),
        find(&digest.elements, "button", "place order").map(|e| e.ref_id),
        find(&digest.elements, "button", "far button").map(|e| e.ref_id),
    );
    let (Some(email), Some(terms), Some(country), Some(go), Some(far)) = refs else {
        println!("FAIL refs: could not find every control in the digest: {refs:?}");
        std::process::exit(1);
    };

    let r = engine.type_text(&format!("@{email}"), "me@example.com");
    ok &= check("type_text @ref", r.is_ok(), &format!("{r:?}"));
    let r = engine.set_checked(&format!("@{terms}"), true);
    ok &= check("set_checked", r.is_ok(), &format!("{r:?}"));
    let r = engine.select_option(&format!("@{country}"), "Germany");
    ok &= check("select_option (by label)", r.is_ok(), &format!("{r:?}"));
    let r = engine.click(&format!("@{go}"));
    ok &= check("click submit", r.is_ok(), &format!("{r:?}"));
    let r = engine.scroll_to(&format!("@{far}"));
    ok &= check("scroll_to", r.is_ok(), &format!("{r:?}"));
    let r = engine.click(&format!("@{far}"));
    ok &= check("click off-screen button", r.is_ok(), &format!("{r:?}"));

    let state = engine
        .js_execute(
            "JSON.stringify({email:document.getElementById('email').value,\
             terms:document.getElementById('terms').checked,\
             country:document.getElementById('country').value,\
             submitted:window.submitted||0,far:window.far||0,\
             scrollY:Math.round(window.scrollY)})",
        )
        .map(|(s, _)| s)
        .unwrap_or_else(|e| format!("{e:?}"));
    println!("INFO page state: {state}");
    ok &= check(
        "page reflects the actions",
        state.contains("me@example.com")
            && state.contains("true")
            && state.contains("de")
            && state.contains("far")
            && !state.contains("\"submitted\\\":0"),
        &state,
    );

    let (links, _) = engine
        .extract_links(None)
        .unwrap_or_else(|e| panic!("extract_links: {e:?}"));
    ok &= check(
        "extract_links",
        links.iter().any(|l| l.href.contains("example.org/help")),
        &format!("{} link(s)", links.len()),
    );

    std::process::exit(if ok { 0 } else { 1 });
}
