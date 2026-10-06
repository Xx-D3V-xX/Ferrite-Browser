//! Loads a page in the real engine and reports what happened: a PNG of the
//! rendered frame, the title, how long the load took, every console message,
//! a summary of the requests made, a crash if the page's engine crashed, and the
//! answer to `PAGE_SHOT_JS` if set. The quickest way to answer "does this
//! page work in Ferrite?" without the app.
//!
//!   cargo run -p ferrite-servo --features servo --example page_shot -- \
//!       https://github.com 8000 /tmp/github.png [width height]
//!
//! Needs a display-less GL context only (software rendering), so it also runs
//! on a CI machine.

use std::time::{Duration, Instant};

use ferrite_servo::session::{HeadlessServoSession, LoadStatus};

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "https://example.com".into());
    let wait_ms: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8000);
    let out = args.next().unwrap_or_else(|| "page_shot.png".into());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);
    // The display scale: 2 renders as a Retina screen would (a 1280-pixel
    // frame is then a 640 CSS-pixel viewport).
    let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    ferrite_servo::session::set_display_scale(scale);

    let mut session = HeadlessServoSession::new(width, height).expect("engine starts");
    // PAGE_SHOT_BACKGROUND=1 loads the page as a background tab (throttled, hidden).
    let background = std::env::var_os("PAGE_SHOT_BACKGROUND").is_some();
    session.set_active(!background);
    if background {
        // Let the engine apply the throttle to the blank page first; a page
        // loaded afterwards in the same tab inherits it.
        let settle = Instant::now() + Duration::from_millis(700);
        while Instant::now() < settle {
            session.spin();
            std::thread::sleep(Duration::from_millis(8));
        }
    }
    let started = Instant::now();
    session.navigate(&url);

    let mut complete_at = None;
    let deadline = started + Duration::from_millis(wait_ms);
    while Instant::now() < deadline {
        session.spin();
        std::thread::sleep(Duration::from_millis(8));
        if complete_at.is_none() && *session.load_status() == LoadStatus::Complete {
            complete_at = Some(started.elapsed());
        }
    }

    println!("URL     {}", session.current_url());
    println!("TITLE   {:?}", session.page_title());
    println!(
        "VIEWPORT {:?}",
        session.execute_js("innerWidth + 'x' + innerHeight + ' @' + devicePixelRatio")
    );
    match complete_at {
        Some(t) => println!("LOADED  {} ms (load event)", t.as_millis()),
        None => println!("LOADED  never within {wait_ms} ms"),
    }
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for event in session.take_net_events() {
        *kinds.entry(event.kind).or_default() += 1;
    }
    println!("REQUESTS {kinds:?}");
    // PAGE_SHOT_JS: an expression to ask the page once it has loaded (the real-site
    // check asks a video page whether its video plays).
    if let Ok(expression) = std::env::var("PAGE_SHOT_JS") {
        println!("JS      {:?}", session.execute_js(&expression));
    }
    if let Some(crash) = session.take_crash() {
        println!("CRASH   {crash:?}");
    }
    for entry in session.take_console_entries() {
        println!(
            "CONSOLE {:<5} {}",
            entry.level.label(),
            entry.message.chars().take(300).collect::<String>()
        );
    }
    match session.get_frame() {
        Some((w, h, rgba)) => {
            let file = std::fs::File::create(&out).expect("create output");
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .and_then(|mut writer| writer.write_image_data(&rgba))
                .expect("write png");
            println!("FRAME   {w}x{h} -> {out}");
            // The colour a few rows from the top and the bottom, in the middle:
            // CI loads a page that is red at the top and blue at the bottom and
            // checks the picture is neither upside down nor red/blue swapped.
            let at = |y: u32| {
                let i = ((y * w + w / 2) * 4) as usize;
                rgba.get(i..i + 3).map_or_else(
                    || "?".to_string(),
                    |p| format!("{},{},{}", p[0], p[1], p[2]),
                )
            };
            if h > 10 {
                println!("PIXELS  top {} bottom {}", at(5), at(h - 5));
            }
        }
        None => println!("FRAME   none rendered"),
    }
    // Close cleanly so the engine writes its profile (cookies, HTTP cache).
    #[allow(clippy::drop_non_drop)]
    drop(session);
    ferrite_servo::session::shutdown_engine();
}
