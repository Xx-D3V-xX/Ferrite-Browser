//! Checks that a login-style state survives a restart: a cookie set by a
//! server response and a `localStorage` value written by the page.
//!
//! Run it twice with the same `FERRITE_HOME`:
//!
//! ```text
//! FERRITE_HOME=/tmp/ferrite-profile-test cargo run -p ferrite-servo --features servo --example profile_probe -- set
//! FERRITE_HOME=/tmp/ferrite-profile-test cargo run -p ferrite-servo --features servo --example profile_probe -- get
//! ```
//!
//! `set` visits a loopback page that sets a persistent cookie and writes
//! `localStorage`, then shuts the engine down cleanly (which is when Servo
//! writes the profile). `get` starts a fresh process and reports what it still
//! has. Exits non-zero when `get` finds nothing.

use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const PORT: u16 = 8123;

fn serve() {
    let listener = TcpListener::bind(("127.0.0.1", PORT)).expect("bind loopback");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            let saw_cookie = request.contains("cookie:") && request.contains("ferrite_sid=abc123");
            let body = format!(
                "<!doctype html><title>profile</title><body>\
                 <script>if(!localStorage.getItem('ferrite_ls')){{localStorage.setItem('ferrite_ls','stored');}}\
                 window.__server_saw_cookie={saw_cookie};</script>ok</body>"
            );
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: ferrite_sid=abc123; Max-Age=3600; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "get".to_string());
    serve();
    let mut session = HeadlessServoSession::new(800, 600).expect("session");
    let url = format!("http://127.0.0.1:{PORT}/");
    session.navigate(&url);
    let end = Instant::now() + Duration::from_secs(20);
    while !(*session.load_status() == LoadStatus::Complete
        && session.current_url().starts_with("http://127.0.0.1"))
    {
        if Instant::now() > end {
            println!("FAIL load: page did not finish loading");
            std::process::exit(1);
        }
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
    let seen = session
        .execute_js("JSON.stringify({ls: localStorage.getItem('ferrite_ls'), server_saw_cookie: window.__server_saw_cookie})")
        .unwrap_or_default();
    println!("INFO {mode}: {seen}");
    let ok = seen.contains("stored") && seen.contains("true");
    drop(session);
    shutdown_engine();
    if mode == "get" {
        println!(
            "{} restart: localStorage and cookie {}",
            if ok { "PASS" } else { "FAIL" },
            if ok { "survived" } else { "did not survive" }
        );
        std::process::exit(if ok { 0 } else { 1 });
    }
    println!("INFO profile written; now run with `get`");
    std::process::exit(0);
}
