//! Checks, in a real headless Servo session, the page-storage features sites
//! build on: IndexedDB indexes and cursors (`get`/`getAll`/`count` on an index,
//! `openCursor` and `continue`/`advance`/`continuePrimaryKey`/`update`/`delete`),
//! the Cache API (`caches`, built on IndexedDB by `storage_compat.js`), and web
//! storage. Written after YouTube's and Google's page scripts failed with
//! `openCursor is not a function` and `cursor.continue is not a function`.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo --example storage_probe
//! ```
//!
//! The page (`storage_probe.html`) is served from loopback, which is a secure
//! context (the Cache API is only offered on one). It runs its checks and prints
//! one `PASS` or `FAIL` line each to the console; this program prints them and
//! exits non-zero if any failed, or if the page never finished. No outside network
//! or window is needed.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};

const PAGE: &str = include_str!("storage_probe.html");

/// A one-page loopback server: every path answers with the page.
fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
            PAGE.len()
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
    let end = Instant::now() + Duration::from_secs(90);
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
        let loaded = *session.load_status() == LoadStatus::Complete;
        if loaded {
            if let Some(title) = session.page_title() {
                if title.starts_with("DONE") {
                    finished = true;
                    break;
                }
            }
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
        println!("FAIL the page did not finish within 90 s");
        failed += 1;
    }
    let passed = lines.iter().filter(|l| l.starts_with("PASS ")).count();
    println!("{passed} passed, {failed} failed");
    // Shut the engine down cleanly: exiting with it running crashes in its exit handlers.
    drop(session);
    shutdown_engine();
    std::process::exit(i32::from(failed > 0 || !errors.is_empty()));
}
