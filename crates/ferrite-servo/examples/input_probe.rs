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

fn check(name: &str, ok: bool, detail: &str) -> bool {
    println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

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

    std::process::exit(if ok { 0 } else { 1 });
}
