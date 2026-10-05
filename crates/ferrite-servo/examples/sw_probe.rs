//! Checks, in a real headless Servo session, the service workers `sw_compat.js` provides:
//! register, install, activate, `ready`, `controller` and `controllerchange`, `postMessage`
//! both ways, `clients`, `fetch` events with `respondWith` (and the pass-through when the
//! worker ignores a request), a POST body, the Cache API inside the worker, a remembered
//! registration starting again on the next page, `unregister`, and the refusals.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo --example sw_probe
//! ```
//!
//! Loopback is a secure context, which service workers need. The page logs one `PASS` or
//! `FAIL` line per check; this program prints them and exits non-zero if any failed or the
//! page never finished. No outside network or window is needed.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};

const PAGE: &str = include_str!("sw_probe.html");
const WORKER: &str = include_str!("sw_probe_sw.js");

static CACHE_ME_HITS: AtomicU32 = AtomicU32::new(0);

fn respond(stream: &mut std::net::TcpStream, status: &str, ctype: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = request
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/")
            .to_string();
        match path.as_str() {
            "/sw.js" | "/dir/sw2.js" => respond(&mut stream, "200 OK", "text/javascript", WORKER),
            "/bad.js" => respond(
                &mut stream,
                "200 OK",
                "text/javascript",
                "throw new Error('this worker does not start');",
            ),
            "/ping" => respond(&mut stream, "200 OK", "text/plain", "pong"),
            "/cache-me.txt" => {
                let n = CACHE_ME_HITS.fetch_add(1, Ordering::SeqCst) + 1;
                respond(&mut stream, "200 OK", "text/plain", &format!("hit-{n}"));
            }
            "/" | "/index.html" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE),
            _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
        }
    }
}

fn spin_for(session: &mut HeadlessServoSession, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < end {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    // An empty profile, so a registration from an earlier run cannot be found.
    let home = std::env::temp_dir().join(format!("ferrite-sw-{}", std::process::id()));
    std::env::set_var("FERRITE_HOME", &home);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    std::thread::spawn(move || serve(listener));

    let mut session = match HeadlessServoSession::new(900, 600) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL session: {e}");
            std::process::exit(2);
        }
    };
    spin_for(&mut session, 200);
    session.navigate(&format!("http://127.0.0.1:{port}/"));

    let mut lines: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let end = Instant::now() + Duration::from_secs(120);
    let mut finished = false;
    while Instant::now() < end {
        session.spin();
        for entry in session.take_console_entries() {
            if entry.message.starts_with("PASS ")
                || entry.message.starts_with("FAIL ")
                || entry.message.starts_with("EXCEPTION ")
            {
                lines.push(entry.message);
            } else if matches!(entry.level, ferrite_servo::diag::ConsoleLevel::Error) {
                errors.push(entry.message);
            }
        }
        if *session.load_status() == LoadStatus::Complete
            && session.page_title().is_some_and(|t| t == "DONE")
        {
            finished = true;
            spin_for(&mut session, 300);
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }

    let mut failed = 0;
    for line in &lines {
        if !line.starts_with("PASS ") {
            failed += 1;
        }
        println!("{line}");
    }
    for error in &errors {
        println!("PAGE ERROR {error}");
    }
    if !finished {
        println!("FAIL the page did not finish within 120 s");
        failed += 1;
    }
    let passed = lines.iter().filter(|l| l.starts_with("PASS ")).count();
    println!("{passed} passed, {failed} failed");
    drop(session);
    shutdown_engine();
    let _ = std::fs::remove_dir_all(&home);
    std::process::exit(i32::from(failed > 0));
}
