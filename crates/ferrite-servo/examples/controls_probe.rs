//! Checks that the engine's page controls reach the embedder and that answers
//! take effect: a `<select>`, `confirm()`, `prompt()`, and a colour input.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ferrite_servo::diag::{ControlAnswer, PageControl};
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

fn main() {
    let page = std::fs::read_to_string(std::env::args().nth(1).expect("page path")).unwrap();
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
        println!(
            "SELECT  value after answer = {:?}",
            s.execute_js("document.getElementById('s').value")
        );
    }

    click(&mut s, 100.0, 95.0);
    println!("CONFIRM control = {:?}", s.page_control());
    s.answer_control(ControlAnswer::Accept(None));
    pump(&mut s, 400);
    println!("CONFIRM result = {:?}", s.execute_js("String(window.r)"));

    click(&mut s, 100.0, 155.0);
    println!("PROMPT  control = {:?}", s.page_control());
    s.answer_control(ControlAnswer::Accept(Some("alice".into())));
    pump(&mut s, 400);
    println!("PROMPT  result = {:?}", s.execute_js("String(window.pr)"));

    click(&mut s, 60.0, 275.0);
    println!("COLOR   control = {:?}", s.page_control());
    s.answer_control(ControlAnswer::Color("#ff0000".into()));
    pump(&mut s, 400);
    println!(
        "COLOR   value = {:?}",
        s.execute_js("document.getElementById('c').value")
    );

    click(&mut s, 100.0, 215.0);
    println!("FILE    control = {:?}", s.page_control());
    s.answer_control(ControlAnswer::Dismiss);
    println!("CURSOR  {:?}", s.cursor());
}
