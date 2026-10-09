//! Checks, in a real headless Servo session built with the GStreamer media backend,
//! the whole of "a page asks for the camera, the microphone or the screen": the page's
//! request becomes a prompt the browser shows; a refusal reaches the page as
//! `NotAllowedError`; a grant gives a `MediaStream` whose tracks have labels, settings,
//! `enabled` and `stop()`; a remembered "allow" is used only when no agent is working
//! and never for the screen; a remembered "block" holds; the browser can end a share;
//! a page cannot make a live capture look idle.
//!
//! ```text
//! cargo run -p ferrite-servo --features servo,media --example capture_probe
//! ```
//!
//! Capture devices are the engine's own test sources (`FERRITE_MOCK_CAPTURE`), since a
//! build machine has no camera or microphone. The prompts are answered by this program,
//! which stands in for the person. `capture_probe_screen.sh`-style real screen capture
//! is covered separately (see docs/COMMANDS.md). Exits non-zero if any check fails.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ferrite_servo::permissions::{CapabilityKind, PermissionChoice, PermissionPrompt};
use ferrite_servo::session::{shutdown_engine, HeadlessServoSession, LoadStatus};

const PAGE: &str = include_str!("capture_probe.html");

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

/// What a step expects to be asked, and what the stand-in person answers.
struct Plan {
    /// `None`: no prompt may appear.
    prompt: Option<(&'static [CapabilityKind], bool)>,
    choice: PermissionChoice,
}

struct Checks {
    failed: u32,
    passed: u32,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        if ok {
            self.passed += 1;
            println!("PASS {name}");
        } else {
            self.failed += 1;
            println!("FAIL {name} :: {detail}");
        }
    }
}

/// Run page function `step`, answer prompts as `plan` says, return the page's result
/// and the prompts that appeared.
fn run_step(
    session: &mut HeadlessServoSession,
    step: &str,
    plan: &Plan,
) -> (String, Vec<PermissionPrompt>) {
    let _ = session.execute_js("window.__r = null; 0");
    let _ = session.execute_js(&format!("{step}(); 0"));
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(40);
    while Instant::now() < end {
        session.spin();
        if let Some(prompt) = session.permission_prompt() {
            seen.push(prompt);
            session.answer_permission(plan.choice);
        }
        let raw = session
            .execute_js("window.__r === null ? '' : window.__r")
            .unwrap_or_default();
        let text = raw
            .trim_start_matches("String(\"")
            .trim_end_matches("\")")
            .to_string();
        if !text.is_empty() {
            return (text, seen);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    ("TIMEOUT".to_string(), seen)
}

fn expect_prompt(
    checks: &mut Checks,
    name: &str,
    seen: &[PermissionPrompt],
    want: Option<(&[CapabilityKind], bool)>,
) {
    match want {
        None => checks.check(name, seen.is_empty(), &format!("{seen:?}")),
        Some((kinds, agent)) => checks.check(
            name,
            seen.len() == 1 && seen[0].kinds == kinds && seen[0].agent_active == agent,
            &format!("{seen:?}"),
        ),
    }
}

// Sessions only implement `Drop` (and so hold the engine) with the `servo` feature.
#[allow(clippy::drop_non_drop)]
fn main() {
    // An empty profile, so remembered decisions start empty and are not kept after.
    let home = std::env::temp_dir().join(format!("ferrite-capture-{}", std::process::id()));
    std::env::set_var("FERRITE_HOME", &home);
    // `FERRITE_PROBE_REAL_SCREEN=1` (run under a display, e.g. `xvfb-run`) skips the test
    // sources and shares the real screen: the one check a build machine can make of the
    // screen source itself.
    let real_screen = std::env::var_os("FERRITE_PROBE_REAL_SCREEN").is_some();
    if !real_screen {
        std::env::set_var("FERRITE_MOCK_CAPTURE", "1");
    }

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
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end
        && !(*session.load_status() == LoadStatus::Complete
            && session.current_url().contains("127.0.0.1"))
    {
        session.spin();
        std::thread::sleep(Duration::from_millis(16));
    }
    let mut c = Checks {
        failed: 0,
        passed: 0,
    };
    use CapabilityKind::{Camera, Microphone, Screen};
    let block = PermissionChoice::Block { remember: false };
    let allow_once = PermissionChoice::Allow { remember: false };

    if real_screen {
        let plan = Plan {
            prompt: Some((&[Screen], false)),
            choice: allow_once,
        };
        let (r, seen) = run_step(&mut session, "s7", &plan);
        expect_prompt(&mut c, "the real screen is asked for", &seen, plan.prompt);
        c.check("a screen share starts", r.contains("label:Screen"), &r);
        let (r, _) = run_step(
            &mut session,
            "s12",
            &Plan {
                prompt: None,
                choice: block,
            },
        );
        let width = r
            .split("w:")
            .nth(1)
            .and_then(|t| t.split(|ch: char| !ch.is_ascii_digit()).next())
            .and_then(|t| t.parse::<u32>().ok())
            .unwrap_or(0);
        c.check(
            "the screen's frames reach a video element at the screen's size",
            width > 0 && r.contains("playing:true"),
            &r,
        );
        session.stop_capture();
        println!("{} passed, {} failed", c.passed, c.failed);
        let failed = c.failed;
        drop(session);
        shutdown_engine();
        let _ = std::fs::remove_dir_all(&home);
        std::process::exit(i32::from(failed > 0));
    }

    // 1. Refused: the page is told NotAllowedError, after a single card for both.
    let plan = Plan {
        prompt: Some((&[Camera, Microphone], false)),
        choice: block,
    };
    let (r, seen) = run_step(&mut session, "s1", &plan);
    c.check(
        "a refusal reaches the page as NotAllowedError",
        r == "NotAllowedError",
        &r,
    );
    expect_prompt(
        &mut c,
        "camera and microphone are asked in one prompt",
        &seen,
        plan.prompt,
    );

    // 2. Allowed once: a stream with a labelled camera and microphone that plays.
    let plan = Plan {
        prompt: Some((&[Camera, Microphone], false)),
        choice: allow_once,
    };
    let (r, seen) = run_step(&mut session, "s2", &plan);
    expect_prompt(
        &mut c,
        "the second request is asked again (a block was not remembered)",
        &seen,
        plan.prompt,
    );
    c.check(
        "the stream has a camera and a microphone, labelled and live",
        r.contains("video:Camera:live:true") && r.contains("audio:Microphone:live:true"),
        &r,
    );
    c.check(
        "the stream is active and has an id",
        r.contains("active:true") && !r.contains("idLen:0"),
        &r,
    );
    c.check(
        "getSettings has a size",
        r.contains("settings:[") && !r.contains("settings:[null"),
        &r,
    );
    let video_width = r
        .split("videoWidth:")
        .nth(1)
        .and_then(|t| t.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|t| t.parse::<u32>().ok())
        .unwrap_or(0);
    c.check("the stream plays in a video element", video_width > 0, &r);
    spin_for(&mut session, 1500);
    c.check(
        "the browser sees the camera and microphone as live",
        session.capture_active() == (true, true, false),
        &format!("{:?}", session.capture_active()),
    );

    // 3. enabled, clone, stop.
    let (r, _) = run_step(
        &mut session,
        "s3",
        &Plan {
            prompt: None,
            choice: block,
        },
    );
    c.check(
        "enabled=false keeps the track live",
        r.contains("disabled:true"),
        &r,
    );
    c.check(
        "a clone keeps the device when its original stops",
        r.contains("cloneLive:true"),
        &r,
    );
    c.check(
        "stop() ends every track and the stream goes inactive",
        r.contains("ended:true") && r.contains("active:false"),
        &r,
    );
    spin_for(&mut session, 1500);
    c.check(
        "the browser sees nothing live after the tracks stop",
        session.capture_active() == (false, false, false),
        &format!("{:?}", session.capture_active()),
    );

    // 4. Allow and remember the microphone; 5. then no prompt.
    let plan = Plan {
        prompt: Some((&[Microphone], false)),
        choice: PermissionChoice::Allow { remember: true },
    };
    let (_, seen) = run_step(&mut session, "s4", &plan);
    expect_prompt(
        &mut c,
        "a microphone request is asked once",
        &seen,
        plan.prompt,
    );
    let (r, seen) = run_step(
        &mut session,
        "s5",
        &Plan {
            prompt: None,
            choice: block,
        },
    );
    expect_prompt(
        &mut c,
        "a remembered allow is used without a prompt",
        &seen,
        None,
    );
    c.check("... and the page gets its stream", r == "granted", &r);

    // 6. The agent is working: the standing allow is not used.
    session.set_agent_active(true);
    let plan = Plan {
        prompt: Some((&[Microphone], true)),
        choice: block,
    };
    let (r, seen) = run_step(&mut session, "s6", &plan);
    session.set_agent_active(false);
    expect_prompt(
        &mut c,
        "while the agent works, a remembered allow is NOT used: the person is asked, and told",
        &seen,
        plan.prompt,
    );
    c.check("... and a refusal stands", r == "NotAllowedError", &r);

    // 7. Screen: asked, allowed once, a screen track.
    let plan = Plan {
        prompt: Some((&[Screen], false)),
        choice: PermissionChoice::Allow { remember: true },
    };
    let (r, seen) = run_step(&mut session, "s7", &plan);
    expect_prompt(&mut c, "sharing the screen is asked", &seen, plan.prompt);
    c.check(
        "a screen share is one video track labelled Screen, a monitor",
        r.contains("n:1")
            && r.contains("label:Screen")
            && r.contains("surface:monitor")
            && r.contains("kind:video"),
        &r,
    );
    spin_for(&mut session, 1500);
    c.check(
        "the browser sees the screen share as live",
        session.capture_active().2,
        &format!("{:?}", session.capture_active()),
    );

    // 8. The browser ends the share.
    session.stop_capture();
    let (r, _) = run_step(
        &mut session,
        "s8",
        &Plan {
            prompt: None,
            choice: block,
        },
    );
    c.check(
        "ending the share from the browser ends the track and fires `ended`",
        r.contains("state:ended") && r.contains("ended:true") && r.contains("active:false"),
        &r,
    );
    spin_for(&mut session, 1500);
    c.check(
        "... and the browser shows nothing live",
        session.capture_active() == (false, false, false),
        &format!("{:?}", session.capture_active()),
    );

    // 9. The screen is never remembered as allowed: asked again, even after "remember".
    let plan = Plan {
        prompt: Some((&[Screen], false)),
        choice: PermissionChoice::Block { remember: true },
    };
    let (r, seen) = run_step(&mut session, "s9", &plan);
    expect_prompt(
        &mut c,
        "the screen is asked again: an allow was never kept",
        &seen,
        plan.prompt,
    );
    c.check(
        "... and the refusal reaches the page",
        r == "NotAllowedError",
        &r,
    );
    let (r, seen) = run_step(
        &mut session,
        "s9",
        &Plan {
            prompt: None,
            choice: block,
        },
    );
    expect_prompt(
        &mut c,
        "a remembered block holds without a prompt",
        &seen,
        None,
    );
    c.check("... and the page is refused", r == "NotAllowedError", &r);

    // 10. The rest of the API.
    let (r, _) = run_step(
        &mut session,
        "s10",
        &Plan {
            prompt: None,
            choice: block,
        },
    );
    c.check(
        "getSupportedConstraints, enumerateDevices, and an empty request is a TypeError",
        r.contains("width:true")
            && r.contains("isArray:true")
            && r.contains("emptyConstraints:TypeError"),
        &r,
    );

    // 11. A page cannot hide a live capture.
    let plan = Plan {
        prompt: Some((&[Camera], false)),
        choice: allow_once,
    };
    let (r, _) = run_step(&mut session, "s11", &plan);
    c.check(
        "the page cannot redefine getUserMedia or remove the capture hook",
        r.contains("redefine:refused") && r.contains("hook:true"),
        &r,
    );
    spin_for(&mut session, 1500);
    c.check(
        "... so a live camera still shows",
        session.capture_active().0,
        &format!("{:?}", session.capture_active()),
    );

    // The audit log kept the decisions.
    println!("{} passed, {} failed", c.passed, c.failed);
    let failed = c.failed;
    drop(session);
    shutdown_engine();
    let _ = std::fs::remove_dir_all(&home);
    std::process::exit(i32::from(failed > 0));
}
