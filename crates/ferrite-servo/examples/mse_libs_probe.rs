//! Plays streams with real adaptive-streaming libraries (hls.js, dash.js, Shaka Player)
//! in a headless Servo session, to check Media Source Extensions with the code pages
//! actually use. Run through `scripts/mse-libs-probe.sh`, which fetches the libraries and
//! makes the streams; this program serves a directory and loads one page from it.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo,media --example mse_libs_probe -- DIR PAGE
//! ```
//!
//! The page prints `PASS`/`FAIL` lines to the console and sets its title to `DONE` when
//! it has finished. Exits non-zero if anything failed.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "mpd" => "application/dash+xml",
        "m3u8" => "application/vnd.apple.mpegurl",
        "mp4" | "m4s" => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// A loopback server for the files in `dir`.
fn serve(listener: TcpListener, dir: PathBuf) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/")
            .trim_start_matches('/')
            .to_string();
        let found = (!path.contains(".."))
            .then(|| std::fs::read(dir.join(&path)).ok())
            .flatten();
        let (status, ctype, body) = match found {
            Some(body) => ("200 OK", content_type(&path), body),
            None => ("404 Not Found", "text/plain", b"not found".to_vec()),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(&body);
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
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(page)) = (args.next(), args.next()) else {
        println!("usage: mse_libs_probe DIR PAGE");
        std::process::exit(2);
    };
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            println!("FAIL loopback server: {e}");
            std::process::exit(2);
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    std::thread::spawn(move || serve(listener, PathBuf::from(dir)));

    let mut session = match HeadlessServoSession::new(900, 600) {
        Ok(s) => s,
        Err(e) => {
            println!("FAIL session: {e}");
            std::process::exit(2);
        }
    };
    spin_for(&mut session, 200);
    session.navigate(&format!("http://127.0.0.1:{port}/{page}"));

    // `MSE_PROBE_VERBOSE=1` shows the page's whole console (a library's own debug log).
    let verbose = std::env::var_os("MSE_PROBE_VERBOSE").is_some();
    let mut lines: Vec<String> = Vec::new();
    let end = Instant::now() + Duration::from_secs(120);
    let mut finished = false;
    while Instant::now() < end {
        session.spin();
        for entry in session.take_console_entries() {
            let m = &entry.message;
            if m.starts_with("PASS ")
                || m.starts_with("FAIL ")
                || m.starts_with("EXCEPTION ")
                || m.starts_with("INFO ")
            {
                println!("{m}");
                lines.push(entry.message);
            } else if matches!(entry.level, ferrite_servo::diag::ConsoleLevel::Error) {
                println!("PAGE ERROR {m}");
            } else if verbose {
                println!("CONSOLE {m}");
            }
        }
        if *session.load_status() == LoadStatus::Complete
            && session.page_title().is_some_and(|t| t.starts_with("DONE"))
        {
            finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let failed = lines
        .iter()
        .filter(|l| l.starts_with("FAIL ") || l.starts_with("EXCEPTION "))
        .count()
        + usize::from(!finished);
    if !finished {
        println!("FAIL the page did not finish within 120 s");
    }
    let passed = lines.iter().filter(|l| l.starts_with("PASS ")).count();
    println!("{passed} passed, {failed} failed");
    drop(session);
    shutdown_engine();
    std::process::exit(i32::from(failed > 0));
}
