//! `ServoEngine` conformance — the directive's exit gate #3: "the same
//! conformance suite (or an equivalent...) is run against `ServoEngine` at
//! least once, and the result recorded."
//!
//! **`#[ignore]`d by default** — mirrors `ferrite-model`'s live-provider
//! tests (R7's pattern): these need the real `engine-servo` feature (which
//! pulls in `libservo` and a real Servo build) and a real, if only
//! loopback, network socket, so they must never run as part of `just test`.
//! Run explicitly with:
//!
//! ```sh
//! cargo test -p ferrite-engine-servo --features engine-servo -- --ignored --test-threads=1
//! ```
//!
//! # Why this is one big `#[test]` fn, not several small ones
//!
//! This was tried first as several independent `#[test]` functions (one per
//! assertion, matching `ferrite-engine`'s own `tests/conformance.rs` style)
//! and it does not work against the real `servo` feature, for a reason
//! specific to Servo, discovered while attempting exactly that in this
//! session: Servo's `opts` module is a **process-global** singleton
//! (`servo::config::opts`) that panics if initialized twice — its own
//! comment says as much — and `ferrite-servo::session::get_or_init_servo`'s
//! `thread_local!` cache only amortizes that *within one OS thread*.
//! Rust's `libtest` harness spawns a **fresh thread per `#[test]` function**
//! regardless of `--test-threads`, which only bounds how many run
//! *concurrently*, not whether each gets its own thread. So the very first
//! multi-test run of this file produced exactly one passing construction
//! and six `"Already initialized"` panics — a real, load-bearing constraint
//! of the existing (out-of-scope-to-modify, per this charter's file list)
//! `ferrite-servo` session code, not a `ServoEngine` bug. The honest fix is
//! this file's actual shape: one `ServoEngine`, created once, driven
//! through every assertion on the single thread that owns it. See
//! `docs/handoffs/a09.md` and `docs/PROGRESS.md`'s A9 entry for the exact
//! panic text this session captured before restructuring.
//!
//! # Known, unresolved failure as of A9 (T-220)
//!
//! Running this test against the real, successfully-built Servo
//! (`docs/BUILD_BUDGET.md`'s A9 entry) gets past both of the above process
//! constraints — `ServoEngine::new` succeeds, no "Already initialized"
//! panic — but the `navigate()` call itself does not observably complete: a
//! throwaway diagnostic (not committed) confirmed the loopback fixture
//! server never even receives a TCP connection attempt within
//! `ServoEngine`'s `drive_until_loaded` window, so `current_url()` still
//! reports `about:blank` afterward and every subsequent assertion that
//! needs a loaded page fails on `EngineError::OpaqueOrigin`. The leading
//! hypothesis: `HeadlessServoSession` was built to be driven by a genuine
//! winit `EventLoop::run()` tick (see `ferrite-shell`'s real usage) — the
//! module doc on `HeadlessServoSession` itself says "the caller (Iced)
//! drives it by calling `spin()` on every tick" — and plain repeated
//! `spin_event_loop()` calls with no winit loop underneath may not be
//! sufficient to make Servo's networking stack (which likely runs on its
//! own thread/component and needs a live OS-level event loop to progress)
//! actually attempt the fetch. This was not root-caused further or fixed:
//! it would mean either modifying `ferrite-servo`'s `session.rs` (outside
//! this charter's file list) or wrapping a real winit loop inside
//! `ServoEngine` (a materially larger undertaking than "map trait methods
//! onto an existing session type"). Filed as **T-220** for whoever owns
//! `ferrite-servo`/`ferrite-engine-servo` next. See `docs/handoffs/a09.md`.
//!
//! # Why this is "an equivalent" suite, not the literal shared one
//!
//! `ferrite_engine::conformance` holds the assertions that are true of
//! *every* `BrowserEngine` regardless of implementation (see that module's
//! docs for why the full, richly-scripted `MockEngine` suite does not
//! generalize cheaply to a real browser). This test reuses that shared
//! subset against `ServoEngine` and adds real-page equivalents of the
//! `MockEngine`-specific assertions (click, form fill, DOM snapshot, cookie
//! scoping) against a tiny fixture page served over `127.0.0.1` — real,
//! actual HTTP, but never reaching outside the loopback interface.

#![cfg(feature = "engine-servo")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use ferrite_core::Origin;
use ferrite_engine::{conformance, BrowserEngine, Cookie, EngineError, TabId, WaitCondition};
use ferrite_engine_servo::ServoEngine;

const FIXTURE_HTML: &str = r#"<!doctype html>
<html><head><title>Ferrite fixture</title></head>
<body>
  <button id="the-button" onclick="document.getElementById('status').textContent='clicked'">Go</button>
  <input id="the-input" />
  <select id="the-select"><option value="a">A</option><option value="b">B</option></select>
  <p id="status">idle</p>
</body></html>"#;

/// Serves `FIXTURE_HTML` for every request on an ephemeral loopback port,
/// in a background thread, for the lifetime of the returned guard.
struct FixtureServer {
    addr: std::net::SocketAddr,
    _thread: std::thread::JoinHandle<()>,
}

impl FixtureServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback fixture server");
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                Self::handle(stream);
            }
        });
        Self {
            addr,
            _thread: thread,
        }
    }

    fn handle(mut stream: TcpStream) {
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let body = FIXTURE_HTML.as_bytes();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
    }

    fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }
}

#[test]
#[ignore = "needs --features engine-servo, a completed `just build-servo`, and --test-threads=1"]
fn full_servo_conformance_suite_runs_sequentially_in_one_process() {
    let fixture = FixtureServer::start();
    let mut engine = ServoEngine::new(800, 600).expect("ServoEngine::new");

    // ── Shared, engine-agnostic assertions ──
    conformance::navigate_updates_current_url_and_origin(&mut engine, &fixture.url());
    conformance::unknown_tab_id_is_a_typed_error(&mut engine);
    conformance::wait_idle_completes(&mut engine);

    // ── click / dom read: a real page's own JS actually ran ──
    engine
        .wait_for(WaitCondition::Selector("#the-button".to_string()))
        .expect("button must be present");
    engine
        .click("#the-button")
        .expect("click must find the button");
    let (status_text, _) = engine.read_text("#status").expect("read_text #status");
    assert_eq!(
        status_text, "clicked",
        "the fixture page's own onclick handler must have run"
    );

    // ── type_text / select_option against a real page ──
    engine
        .type_text("#the-input", "hello")
        .expect("type_text must find the input");
    engine
        .select_option("#the-select", "b")
        .expect("select_option must find the select");

    // ── cookies_read is scoped to the current page's own origin ──
    let current = Origin::parse(fixture.url()).unwrap();
    let (cookies, _): (Vec<Cookie>, _) = engine.cookies_read(&current).expect("same-origin read");
    assert!(cookies.is_empty(), "fixture page sets no cookies");

    let other = Origin::parse("https://not-the-fixture.example/").unwrap();
    let err = engine.cookies_read(&other).unwrap_err();
    assert!(
        matches!(err, EngineError::Unsupported(_)),
        "reading a different origin's cookies must be refused, not silently answered"
    );

    // ── download / clipboard report their documented unsupported status ──
    assert!(matches!(
        engine.download("http://example.invalid/f"),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        engine.clipboard_read(),
        Err(EngineError::Unsupported(_))
    ));

    // A sanity check that TabId(0) (the engine's original tab) is still
    // switchable to after all of the above.
    engine
        .switch_tab(TabId(0))
        .expect("original tab must still exist");
}
