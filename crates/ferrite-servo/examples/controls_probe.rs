//! Checks that the engine's page controls reach the embedder and that answers
//! take effect: a `<select>`, `confirm()`, `prompt()`, a colour input, a file
//! input and an HTTP sign-in. The page is `controls_probe.html` (or another,
//! given as the argument). Exits non-zero when a check fails.
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
            let n = st.read(&mut b).unwrap_or(0);
            let request = String::from_utf8_lossy(&b[..n]);
            if request.starts_with("GET /private") {
                // HTTP basic authentication: `ann` / `hunter2`.
                let signed_in = request
                    .lines()
                    .any(|l| l.eq_ignore_ascii_case("authorization: Basic YW5uOmh1bnRlcjI="));
                let (status, extra, body) = if signed_in {
                    ("200 OK", "", "<title>inside</title>signed in")
                } else {
                    (
                        "401 Unauthorized",
                        "WWW-Authenticate: Basic realm=\"probe\"\r\n",
                        "<title>denied</title>no",
                    )
                };
                let _ = write!(st, "HTTP/1.1 {status}\r\n{extra}Content-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                continue;
            }
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

    // HTTP authentication: a 401 asks the embedder; Cancel shows the 401 page and
    // a username and password reach the server.
    let title = |s: &mut HeadlessServoSession| format!("{:?}", s.execute_js("document.title"));
    let auth_prompt = |s: &mut HeadlessServoSession| {
        s.navigate(&format!("http://127.0.0.1:{port}/private"));
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(15) {
            pump(s, 50);
            if let Some(control @ PageControl::Auth { .. }) = s.page_control() {
                return Some(control);
            }
        }
        None
    };
    let control = auth_prompt(&mut s);
    check(
        matches!(&control, Some(PageControl::Auth { host, for_proxy: false }) if host == "127.0.0.1"),
        "a 401 asks for a username and password",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Dismiss);
    pump(&mut s, 1500);
    let shown = title(&mut s);
    check(shown.contains("denied"), "Cancel shows the 401 page", shown);
    let control = auth_prompt(&mut s);
    check(
        matches!(control, Some(PageControl::Auth { .. })),
        "a second visit asks again",
        format!("{control:?}"),
    );
    s.answer_control(ControlAnswer::Credentials {
        username: "ann".into(),
        password: ferrite_servo::diag::Password("hunter2".into()),
    });
    pump(&mut s, 1500);
    let shown = title(&mut s);
    check(
        shown.contains("inside"),
        "the credentials reach the server",
        shown,
    );
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
