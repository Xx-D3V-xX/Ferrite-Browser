//! Drives a real headless Servo session against a local page and reports
//! whether scrolling, clicking, typing and reload actually work — the checks
//! that the widget-tree tests in `ferrite-ui` cannot make.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo --example input_probe
//! cargo run -p ferrite-servo --features servo --example input_probe -- https://example.org/form
//! ```
//!
//! With no argument it loads a built-in page. A page given as an argument must
//! contain `#q` (text input), `#cb` (checkbox) and `#btn` (button) and be
//! taller than the viewport. Exits non-zero when a check fails. No network or
//! window is needed for the built-in page: the session renders on the CPU.

use ferrite_servo::session::{
    HeadlessServoSession, LoadStatus, PageKey, PageKeyEvent, PageNamedKey,
};
use std::time::{Duration, Instant};

const PAGE: &str = "<!doctype html><html><head><meta charset=utf-8><title>probe</title>\
<style>body{font-family:sans-serif;margin:20px}.pad{height:2200px}input,button{font-size:18px;margin:8px;padding:6px}</style></head>\
<body><h1>Probe form</h1><input id=q type=text placeholder='type here'>\
<label><input id=cb type=checkbox> tick</label><button id=btn onclick='window.clicks=(window.clicks||0)+1'>click</button>\
<div class=pad>tall</div>\
<script>window.clicks=0;document.getElementById('cb').addEventListener('click',function(){window.cbclicks=(window.cbclicks||0)+1});</script>\
</body></html>";

const W: u32 = 1000;
const H: u32 = 700;

fn spin_for(session: &mut HeadlessServoSession, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < end {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Waits until `expect` (a URL fragment) is the loaded page. Waiting on the
/// status alone returns at once, because the initial `about:blank` is already
/// "complete".
fn wait_loaded(session: &mut HeadlessServoSession, what: &str, expect: &str) -> bool {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        session.spin();
        if *session.load_status() == LoadStatus::Complete && session.current_url().contains(expect)
        {
            spin_for(session, 300);
            return true;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    println!("FAIL {what}: page did not finish loading");
    false
}

fn js(session: &mut HeadlessServoSession, script: &str) -> String {
    session
        .execute_js(script)
        .unwrap_or_else(|e| format!("<err {e}>"))
}

fn center_of(session: &mut HeadlessServoSession, id: &str) -> (f32, f32) {
    let raw = js(
        session,
        &format!(
            "(function(){{var r=document.getElementById('{id}').getBoundingClientRect();\
             return JSON.stringify([r.left+r.width/2, r.top+r.height/2]);}})()"
        ),
    );
    let digits: Vec<f32> = raw
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|p| p.parse().ok())
        .collect();
    (
        digits.first().copied().unwrap_or(0.0),
        digits.get(1).copied().unwrap_or(0.0),
    )
}

fn click(session: &mut HeadlessServoSession, x: f32, y: f32) {
    session.send_mouse_move(x, y);
    spin_for(session, 60);
    session.send_mouse_down(x, y);
    session.send_mouse_up(x, y);
    spin_for(session, 120);
}

fn type_text(session: &mut HeadlessServoSession, text: &str) {
    for ch in text.chars() {
        for down in [true, false] {
            session.send_key(&PageKeyEvent {
                down,
                key: PageKey::Character(ch.to_string()),
                shift: false,
                ctrl: false,
                alt: false,
                meta: false,
            });
        }
        spin_for(session, 30);
    }
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// One UI tick over two tabs, in the way selected by `MODE` (see the variants).
fn tick(a: &mut HeadlessServoSession, b: &mut HeadlessServoSession) {
    match std::env::var("MODE").as_deref() {
        Ok("state_only") => {
            a.pump_engine();
            a.sync_and_read();
            b.sync_state();
        }
        Ok("b_only") => {
            b.pump_engine();
            b.sync_and_read();
            a.sync_state();
        }
        Ok("no_a_read") => {
            a.pump_engine();
            a.sync_state();
            b.sync_and_read();
        }
        _ => {
            a.pump_engine();
            a.sync_and_read();
            b.sync_and_read();
        }
    }
}

fn check(name: &str, ok: bool, detail: &str) -> bool {
    println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("data:text/html;charset=utf-8,{}", percent_encode(PAGE)));
    // Enough of the URL to tell the loaded page from the initial about:blank.
    let expect: String = url.chars().take(24).collect();
    let mut session = match HeadlessServoSession::new(W, H) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL session: {e}");
            std::process::exit(2);
        }
    };
    spin_for(&mut session, 200);
    session.navigate(&url);
    if !wait_loaded(&mut session, "first load", &expect) {
        std::process::exit(1);
    }
    let mut ok = true;

    // Scroll: first the page's own API (is scrolling possible at all?), then
    // the wheel path the app uses (a pointer move first, as the UI always does).
    js(&mut session, "window.scrollTo(0,250)");
    spin_for(&mut session, 300);
    let via_js = js(&mut session, "String(window.scrollY)");
    println!("INFO window.scrollTo(0,250) -> scrollY {via_js}");
    js(&mut session, "window.scrollTo(0,0)");
    spin_for(&mut session, 300);
    let before = js(&mut session, "String(window.scrollY)");
    session.send_mouse_move(400.0, 300.0);
    spin_for(&mut session, 100);
    // Negative dy = scroll down (the OS wheel convention the UI passes through).
    session.send_scroll(400.0, 300.0, 0.0, -300.0);
    spin_for(&mut session, 600);
    let after = js(&mut session, "String(window.scrollY)");
    ok &= check(
        "wheel scroll (300px, not doubled)",
        after.contains("300"),
        &format!("scrollY {before} -> {after}"),
    );
    js(&mut session, "window.scrollTo(0,0)");
    spin_for(&mut session, 200);

    // Click + type.
    let (x, y) = center_of(&mut session, "q");
    click(&mut session, x, y);
    type_text(&mut session, "hello");
    let value = js(&mut session, "document.getElementById('q').value");
    ok &= check("typing", value.contains("hello"), &value);
    session.send_key(&PageKeyEvent {
        down: true,
        key: PageKey::Named(PageNamedKey::Backspace),
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    });
    spin_for(&mut session, 100);
    let value = js(&mut session, "document.getElementById('q').value");
    ok &= check(
        "backspace",
        value.contains("hell") && !value.contains("hello"),
        &value,
    );

    // Checkbox: exactly one click event, ends checked.
    let (x, y) = center_of(&mut session, "cb");
    click(&mut session, x, y);
    let checked = js(
        &mut session,
        "String(document.getElementById('cb').checked)",
    );
    let clicks = js(&mut session, "String(window.cbclicks||0)");
    ok &= check(
        "checkbox",
        checked.contains("true") && clicks.contains('1'),
        &format!("checked={checked} click events={clicks}"),
    );

    // Button.
    let (x, y) = center_of(&mut session, "btn");
    click(&mut session, x, y);
    let n = js(&mut session, "String(window.clicks)");
    ok &= check(
        "button",
        n.contains('1'),
        &format!("handler ran {n} time(s)"),
    );

    // Reload.
    session.reload();
    ok &= wait_loaded(&mut session, "reload", &expect);
    println!("PASS reload: survived");

    // Same checks after a zoom-out transform on the root, to see whether it
    // interferes with scrolling/hit-testing.
    js(
        &mut session,
        "(function(){var el=document.documentElement;el.style.transformOrigin='0 0';\
         el.style.transform='scale(0.9000)';el.style.width='111.1111%';})()",
    );
    spin_for(&mut session, 300);
    let before = js(&mut session, "String(window.scrollY)");
    session.send_mouse_move(400.0, 300.0);
    spin_for(&mut session, 100);
    // Negative dy = scroll down (the OS wheel convention the UI passes through).
    session.send_scroll(400.0, 300.0, 0.0, -300.0);
    spin_for(&mut session, 600);
    let after = js(&mut session, "String(window.scrollY)");
    ok &= check(
        "wheel scroll under zoom<1",
        after.contains("300"),
        &format!("scrollY {before} -> {after}"),
    );

    // A second tab: another session on the same engine, driven the way the
    // app drives tabs (pump once per tick, then sync every session).
    let mut b = match HeadlessServoSession::new(W, H) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL second session: {e}");
            std::process::exit(2);
        }
    };
    // The app makes the new tab the active one the moment it exists.
    if std::env::var("NO_ACTIVATE").is_err() {
        session.set_active(false);
        b.set_active(true);
    }
    // Let the tab's initial about:blank settle first, as it does in the app
    // (a navigation issued in the same instant is lost to it).
    for _ in 0..15 {
        tick(&mut session, &mut b);
        std::thread::sleep(Duration::from_millis(16));
    }
    b.navigate(&url);
    let load_started = Instant::now();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end
        && !(*b.load_status() == LoadStatus::Complete && b.current_url().contains(&expect))
    {
        tick(&mut session, &mut b);
        std::thread::sleep(Duration::from_millis(16));
        if std::env::var("DEBUG_B").is_ok() && load_started.elapsed().as_millis() % 1000 < 20 {
            println!(
                "DEBUG b: status={:?} url={:.40} title={:?} elapsed={:?}",
                b.load_status(),
                b.current_url(),
                b.page_title(),
                load_started.elapsed()
            );
        }
    }
    println!("INFO second tab loaded in {:?}", load_started.elapsed());
    for _ in 0..30 {
        tick(&mut session, &mut b);
        std::thread::sleep(Duration::from_millis(16));
    }
    let (x, y) = center_of(&mut b, "q");
    click(&mut b, x, y);
    type_text(&mut b, "tab2");
    let value = js(&mut b, "document.getElementById('q').value");
    ok &= check("second tab: typing", value.contains("tab2"), &value);
    b.send_mouse_move(400.0, 300.0);
    spin_for(&mut b, 100);
    b.send_scroll(400.0, 300.0, 0.0, -300.0);
    spin_for(&mut b, 600);
    let after = js(&mut b, "String(window.scrollY)");
    ok &= check(
        "second tab: wheel scroll",
        after.contains("300"),
        &format!("scrollY {after}"),
    );

    // Back to the first tab (freshly reloaded, so the zoom transform above is
    // gone): it must react again.
    session.set_active(true);
    b.set_active(false);
    session.reload();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        tick(&mut session, &mut b);
        if *session.load_status() == LoadStatus::Complete
            && js(&mut session, "String(!!document.getElementById('btn'))").contains("true")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    for _ in 0..30 {
        tick(&mut session, &mut b);
        std::thread::sleep(Duration::from_millis(16));
    }
    let (x, y) = center_of(&mut session, "btn");
    click(&mut session, x, y);
    let n = js(&mut session, "String(window.clicks)");
    ok &= check(
        "first tab again: button",
        n.contains('1'),
        &format!("handler ran {n} time(s)"),
    );
    let (x, y) = center_of(&mut session, "q");
    click(&mut session, x, y);
    type_text(&mut session, "back");
    let value = js(&mut session, "document.getElementById('q').value");
    ok &= check("first tab again: typing", value.contains("back"), &value);

    // Shut the engine down cleanly rather than exiting with Servo's threads running.
    drop(session);
    drop(b);
    ferrite_servo::session::shutdown_engine();
    std::process::exit(if ok { 0 } else { 1 });
}
