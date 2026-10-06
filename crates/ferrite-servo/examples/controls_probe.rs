//! Checks that the engine's page controls reach the embedder and that answers
//! take effect: a `<select>`, `confirm()`, `prompt()`, a colour input and a file
//! input. The page is `controls_probe.html` (or another, given as the argument).
//! Exits non-zero when a check fails.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ferrite_servo::diag::{ControlAnswer, DialogKind, PageControl};
use ferrite_servo::session::{HeadlessServoSession, LoadStatus};

fn pump(s: &mut HeadlessServoSession, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        s.spin();
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn click(s: &mut HeadlessServoSession, x: f32, y: f32) {
    s.send_mouse_move(x, y);
    pump(s, 80);
    s.send_mouse_down(x, y);
    s.send_mouse_up(x, y);
    pump(s, 400);
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    let page = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(path).expect("read the page"),
        None => include_str!("controls_probe.html").to_string(),
    };
    let mut failed = 0;
    let mut check = |ok: bool, what: &str, got: String| {
        println!(
            "{} {what}{}",
            if ok { "PASS" } else { "FAIL" },
            if ok {
                String::new()
            } else {
                format!(": {got}")
            }
        );
        if !ok {
            failed += 1;
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for st in listener.incoming() {
            let Ok(mut st) = st else { continue };
            let mut b = [0u8; 4096];
            let _ = st.read(&mut b);
            let _ = write!(st, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
        }
    });
    let mut s = HeadlessServoSession::new(800, 500).unwrap();
    s.set_active(true);
    s.navigate(&format!("http://127.0.0.1:{port}/"));
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(20) {
        s.spin();
        std::thread::sleep(Duration::from_millis(8));
        if *s.load_status() == LoadStatus::Complete && s.current_url().contains("127.0.0.1") {
            break;
        }
    }
    pump(&mut s, 500);

    click(&mut s, 100.0, 35.0);
    println!("SELECT  control = {:?}", s.page_control());
    if let Some(PageControl::Select { options, .. }) = s.page_control() {
        let pick = options
            .iter()
            .find(|o| o.label == "cherry")
            .map(|o| o.index)
            .unwrap();
        s.answer_control(ControlAnswer::Select(vec![pick]));
        pump(&mut s, 400);
        let value = format!("{:?}", s.execute_js("document.getElementById('s').value"));
        check(
            value.contains("cherry"),
            "a <select> answered with cherry",
            value,
        );
    } else {
        check(
            false,
            "a <select> opens a control",
            format!("{:?}", s.page_control()),
        );
    }

    click(&mut s, 100.0, 95.0);
    let control = s.page_control();
    check(
        matches!(
            control,
            Some(PageControl::Dialog {
                kind: DialogKind::Confirm,
                ..
            })
        ),
        "confirm() asks",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Accept(None));
    pump(&mut s, 400);
    let result = format!("{:?}", s.execute_js("String(window.r)"));
    check(
        result.contains("true"),
        "confirm() accepted returns true",
        result,
    );

    click(&mut s, 100.0, 155.0);
    let control = s.page_control();
    check(
        matches!(
            control,
            Some(PageControl::Dialog {
                kind: DialogKind::Prompt,
                ..
            })
        ),
        "prompt() asks",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Accept(Some("alice".into())));
    pump(&mut s, 400);
    let result = format!("{:?}", s.execute_js("String(window.pr)"));
    check(
        result.contains("alice"),
        "prompt() returns the answer",
        result,
    );

    click(&mut s, 60.0, 275.0);
    let control = s.page_control();
    check(
        matches!(control, Some(PageControl::Color { .. })),
        "a colour input opens a picker",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Color("#ff0000".into()));
    pump(&mut s, 400);
    let value = format!("{:?}", s.execute_js("document.getElementById('c').value"));
    check(
        value.contains("#ff0000"),
        "the colour answer is the input's value",
        value,
    );

    click(&mut s, 100.0, 215.0);
    let control = s.page_control();
    check(
        matches!(control, Some(PageControl::File { .. })),
        "a file input opens a picker",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Dismiss);
    println!("CURSOR  {:?}", s.cursor());
    println!(
        "{}",
        if failed == 0 {
            "ALL PASS"
        } else {
            "SOME FAILED"
        }
    );
    drop(s);
    ferrite_servo::session::shutdown_engine();
    std::process::exit(i32::from(failed != 0));
}
