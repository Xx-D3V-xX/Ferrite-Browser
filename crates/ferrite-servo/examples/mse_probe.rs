//! Checks, in a real headless Servo session built with the GStreamer media backend,
//! that Media Source Extensions work: a page builds a stream out of `SourceBuffer`s and
//! it plays, seeks, stalls and ends as it should. The data is the fragmented MP4 and
//! WebM files `ferrite-mse` tests its parsers with.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo,media --example mse_probe
//! ```
//!
//! Needs the GStreamer development files and plugins (see `docs/COMMANDS.md`). It prints
//! one `PASS` or `FAIL` line per check and exits non-zero if any failed.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};

const PAGE: &str = include_str!("mse_probe.html");
const AV_MP4: &[u8] = include_bytes!("../../ferrite-mse/tests/fixtures/av_h264_aac.mp4");
const V_MP4: &[u8] = include_bytes!("../../ferrite-mse/tests/fixtures/v_h264.mp4");
const A_MP4: &[u8] = include_bytes!("../../ferrite-mse/tests/fixtures/a_aac.mp4");
const AV_WEBM: &[u8] = include_bytes!("../../ferrite-mse/tests/fixtures/av_vp9_opus.webm");

/// A loopback server: the page, and the media files it appends.
fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let (ctype, body): (&str, &[u8]) = if request.starts_with("GET /av_h264_aac.mp4") {
            ("video/mp4", AV_MP4)
        } else if request.starts_with("GET /v_h264.mp4") {
            ("video/mp4", V_MP4)
        } else if request.starts_with("GET /a_aac.mp4") {
            ("audio/mp4", A_MP4)
        } else if request.starts_with("GET /av_vp9_opus.webm") {
            ("video/webm", AV_WEBM)
        } else {
            ("text/html; charset=utf-8", PAGE.as_bytes())
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
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
    let end = Instant::now() + Duration::from_secs(240);
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
        println!("FAIL the page did not finish within 240 s");
        failed += 1;
    }
    let passed = lines.iter().filter(|l| l.starts_with("PASS ")).count();
    println!("{passed} passed, {failed} failed");
    // Shut the engine down cleanly: exiting with it running crashes in its exit handlers.
    drop(session);
    shutdown_engine();
    std::process::exit(i32::from(failed > 0 || !errors.is_empty()));
}
