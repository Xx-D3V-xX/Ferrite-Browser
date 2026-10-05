// The live runner against a fake model server on the loopback interface.
//
// The unit tests in `ferrite_eval::live` drive the stack with `ferrite-model`'s
// scripted `MockProvider`; this file drives it through the REAL backends
// (`OllamaProvider`, `GeminiProvider`) over a real HTTP connection to a server this
// test owns. That is what proves the wire-level promises: a 429's `Retry-After`
// header reaches the backoff, a retry really re-sends, a key in a request URL never
// reaches a printed line or a stored file, and a whole batch (prediction, agent
// loop, guard, resume) works through an HTTP backend.
//
// Nothing leaves the machine (R7): the only address used is 127.0.0.1, and no real
// key or model exists anywhere in this file.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ferrite_eval::live::cli::{execute, Host, EXIT_OK, EXIT_SOME_FAILED, EXIT_USAGE};
use ferrite_eval::live::config::LiveArgs;
use ferrite_eval::live::store;
use ferrite_ipi::dry_run::DryRunOrchestrator;
use ferrite_model::secret::NoSecretStore;
use ferrite_model::testing::Sleeper;
use ferrite_model::MapEnv;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const FAKE_KEY: &str = "AIzaFAKEKEYFORTESTSONLY0123456789abc";

#[derive(Debug, Clone)]
struct Req {
    path: String,
    body: String,
}

struct Resp {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Resp {
    fn ok(body: &str) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_string(),
        }
    }
}

struct Fake {
    addr: SocketAddr,
    hits: Arc<Mutex<Vec<Req>>>,
}

impl Fake {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn hits(&self) -> Vec<Req> {
        self.hits.lock().unwrap().clone()
    }
}

/// A one-request-per-connection HTTP/1.1 server. `handler` gets the 0-based index
/// of the request and the request.
async fn spawn(handler: impl Fn(usize, &Req) -> Resp + Send + Sync + 'static) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let hits: Arc<Mutex<Vec<Req>>> = Arc::default();
    let seen = hits.clone();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head_end, content_length) = loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + content_length {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("")
                    .to_string();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                let req = Req { path, body };
                let index = {
                    let mut hits = seen.lock().unwrap();
                    hits.push(req.clone());
                    hits.len() - 1
                };
                let resp = handler(index, &req);
                let mut out = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                    resp.status,
                    resp.body.len()
                );
                for (k, v) in resp.headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                out.push_str(&resp.body);
                let _ = socket.write_all(out.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Fake { addr, hits }
}

/// Records every backoff wait and returns at once, EXCEPT the per-request timeout
/// (an hour), which never fires: a real HTTP call has to be allowed to finish, and
/// `RecordingSleeper` would time every one of them out.
#[derive(Debug, Default)]
struct BackoffRecorder {
    waits: Mutex<Vec<Duration>>,
}

#[async_trait]
impl Sleeper for BackoffRecorder {
    async fn sleep(&self, duration: Duration) {
        if duration >= Duration::from_secs(1000) {
            std::future::pending::<()>().await;
        }
        self.waits.lock().unwrap().push(duration);
    }
}

fn chat(content: &str) -> String {
    serde_json::json!({
        "message": {"role": "assistant", "content": content},
        "prompt_eval_count": 10,
        "eval_count": 2,
    })
    .to_string()
}

fn gemini(content: &str) -> String {
    serde_json::json!({
        "candidates": [{"content": {"parts": [{"text": content}]}}],
        "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 2},
    })
    .to_string()
}

fn tmp(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ferrite-live-fake-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(files(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

async fn run(argv: &[String], env: &MapEnv, sleeper: Arc<BackoffRecorder>) -> (i32, String) {
    let args = LiveArgs::parse(argv, env).expect("parses");
    let secrets = NoSecretStore;
    let factory = |path, content| {
        DryRunOrchestrator::with_test_twin_key(
            path,
            content,
            Box::new(MapEnv::new().with(ferrite_ipi::twin::TWIN_KEY_ENV_VAR, "test-only")),
            Box::new(NoSecretStore),
        )
    };
    let host = Host {
        env,
        secrets: &secrets,
        clock: Arc::new(ferrite_core::SystemClock),
        sleeper,
        orchestrator: &factory,
    };
    let mut out = Vec::new();
    let code = execute(args, &host, &mut out).await;
    (code, String::from_utf8_lossy(&out).to_string())
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_string()).collect()
}

#[tokio::test]
async fn a_429_with_retry_after_is_waited_out_and_the_call_is_resent() {
    let fake = spawn(|i, _| {
        if i == 0 {
            Resp {
                status: 429,
                headers: vec![("retry-after", "7".to_string())],
                body: "{\"error\":\"slow down\"}".to_string(),
            }
        } else {
            Resp::ok(&chat("[]"))
        }
    })
    .await;
    let out = tmp("retry-after");
    let args = LiveArgs::parse(
        &argv(&[
            "--provider",
            "ollama",
            "--base-url",
            &fake.url(),
            "--model",
            "fake:1b",
            "--max-attempts",
            "3",
            "--timeout-secs",
            "3600",
            "--out",
            out.to_str().unwrap(),
        ]),
        &MapEnv::new(),
    )
    .unwrap();
    let sleeper = Arc::new(BackoffRecorder::default());
    let stack = ferrite_eval::live::provider::connect(
        &args,
        &args.model_tags().unwrap(),
        &MapEnv::new(),
        &NoSecretStore,
        Arc::new(ferrite_core::SystemClock),
        sleeper.clone(),
    )
    .expect("a local Ollama needs no key");

    let req = ferrite_model::CompletionRequest::new(
        "fake:1b",
        ferrite_model::ModelTier::Main,
        vec![ferrite_model::Message::user("hi")],
    );
    use ferrite_model::ModelProvider as _;
    let response = stack
        .provider
        .complete(req)
        .await
        .expect("recovers after the 429");
    assert_eq!(response.content, "[]");
    assert_eq!(
        *sleeper.waits.lock().unwrap(),
        vec![Duration::from_secs(7)],
        "the wait is the server's Retry-After, parsed off the real header"
    );
    assert_eq!(fake.hits().len(), 2, "the request was really re-sent");
    let live = stack.live.drain();
    assert_eq!((live.len(), live.iter().filter(|e| !e.ok).count()), (2, 1));
}

#[tokio::test]
async fn a_whole_batch_runs_through_an_http_backend_and_a_rerun_costs_nothing() {
    // The fake answers fingerprint calls (they carry a `format`) with a capability
    // list and every agent step with a final answer.
    let fake = spawn(|_, req| {
        if req.body.contains("\"format\"") {
            Resp::ok(&chat("[\"web.read\"]"))
        } else {
            Resp::ok(&chat("{\"action\":\"finish\",\"answer\":\"All done.\"}"))
        }
    })
    .await;
    let out = tmp("batch");
    let cmd = argv(&[
        "--provider",
        "ollama",
        "--base-url",
        &fake.url(),
        "--model",
        "fake:1b",
        "--corpus",
        "agentdojo",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "4",
        "--seed",
        "11",
        "--modes",
        "off,guard",
        "--pause-ms",
        "0",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(&cmd, &MapEnv::new(), Arc::new(BackoffRecorder::default())).await;
    assert_eq!(code, EXIT_OK, "{text}");
    assert!(text.contains("results written: 8"), "{text}");

    let results = store::read_all(&out).unwrap();
    assert_eq!(results.by_key.len(), 8);
    assert!(results
        .by_key
        .values()
        .all(|r| r.provider == "ollama" && r.error.is_none()));
    // Token counts are what the (fake) backend reported, and every call was counted.
    let sent: u32 = results.by_key.values().map(|r| r.calls.live_attempts).sum();
    let asked: u32 = results.by_key.values().map(|r| r.calls.logical).sum();
    let cached: u32 = results.by_key.values().map(|r| r.calls.cache_hits).sum();
    assert_eq!(
        sent as usize,
        fake.hits().len(),
        "the accounting matches what reached the server"
    );
    assert_eq!(
        asked,
        sent + cached,
        "no retries here: every call is sent or cached"
    );
    assert!(
        cached >= 4,
        "the guard run's first prompt is identical to the baseline's, so it is free: {cached}"
    );
    assert!(results.by_key.values().any(|r| r.calls.prompt_tokens >= 10));
    assert!(
        results.by_key.values().all(|r| r.outcome.task_completed),
        "a model that finishes completes"
    );

    // The same command takes the next batch; and re-running a finished window is free.
    let before = fake.hits().len();
    let all_done = argv(&[
        "--provider",
        "ollama",
        "--base-url",
        &fake.url(),
        "--model",
        "fake:1b",
        "--corpus",
        "agentdojo",
        "--suite",
        "agentdojo/slack",
        "--offset",
        "0",
        "--limit",
        "4",
        "--seed",
        "11",
        "--modes",
        "off,guard",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(
        &all_done,
        &MapEnv::new(),
        Arc::new(BackoffRecorder::default()),
    )
    .await;
    assert_eq!(code, EXIT_OK, "{text}");
    assert_eq!(
        fake.hits().len(),
        before,
        "stored results are reused: no request was made"
    );
    assert!(text.contains("results written: 0"), "{text}");
}

#[tokio::test]
async fn a_server_that_answers_garbage_is_the_models_behaviour_not_an_outage() {
    // 200 OK with a body that is not an action: the loop's own parse-and-retry
    // runs, then the run ends. Nothing is executed and nothing is an error.
    let fake = spawn(|_, req| {
        if req.body.contains("\"format\"") {
            Resp::ok(&chat("not even json"))
        } else {
            Resp::ok(&chat("Sure, I will do that!"))
        }
    })
    .await;
    let out = tmp("garbage");
    let cmd = argv(&[
        "--provider",
        "ollama",
        "--base-url",
        &fake.url(),
        "--model",
        "fake:1b",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "2",
        "--modes",
        "guard",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(&cmd, &MapEnv::new(), Arc::new(BackoffRecorder::default())).await;
    assert_eq!(code, EXIT_OK, "{text}");
    for r in store::read_all(&out).unwrap().by_key.values() {
        assert!(r.error.is_none());
        let p = r.prediction.as_ref().unwrap();
        assert!(p.degraded && p.may_use.is_empty(), "fail to empty");
        assert_eq!(r.stop_reason, "malformed_action");
        assert!(r.actions.is_empty() && !r.outcome.attack_executed);
    }
}

#[tokio::test]
async fn a_server_that_keeps_saying_429_stops_the_run_and_stores_errors_not_scores() {
    let fake = spawn(|_, _| Resp {
        status: 429,
        headers: vec![("retry-after", "0".to_string())],
        body: "{}".to_string(),
    })
    .await;
    let out = tmp("always429");
    let cmd = argv(&[
        "--provider",
        "ollama",
        "--base-url",
        &fake.url(),
        "--model",
        "fake:1b",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "10",
        "--modes",
        "off",
        "--max-attempts",
        "2",
        "--max-consecutive-failures",
        "2",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(&cmd, &MapEnv::new(), Arc::new(BackoffRecorder::default())).await;
    assert_eq!(code, ferrite_eval::live::cli::EXIT_PROVIDER, "{text}");
    assert!(
        text.contains("not hammering it") || text.contains("kept failing"),
        "{text}"
    );
    let results = store::read_all(&out).unwrap();
    assert_eq!(
        results.by_key.len(),
        2,
        "stopped after two failures in a row"
    );
    assert!(results
        .by_key
        .values()
        .all(|r| r.error.as_ref().is_some_and(|e| e.class == "rate_limited")));
    // 2 cases x at most 2 attempts each reached the server; the cap held.
    assert!(fake.hits().len() <= 4, "{} requests", fake.hits().len());
}

#[tokio::test]
async fn a_key_in_the_request_url_never_reaches_output_or_files_even_when_the_server_echoes_it() {
    let fake = spawn(|_, req| {
        // A hostile or sloppy server that reflects what it was sent, key included.
        Resp {
            status: 400,
            headers: Vec::new(),
            body: format!(
                "{{\"error\":{{\"message\":\"API key not valid: {} (path {})\"}}}}",
                FAKE_KEY, req.path
            ),
        }
    })
    .await;
    let out = tmp("secret");
    let env = MapEnv::new().with("FERRITE_GEMINI_API_KEY", FAKE_KEY);
    let cmd = argv(&[
        "--provider",
        "gemini",
        "--base-url",
        &fake.url(),
        "--model",
        "fake-model",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "2",
        "--modes",
        "off",
        "--max-consecutive-failures",
        "100",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(&cmd, &env, Arc::new(BackoffRecorder::default())).await;
    assert_eq!(code, EXIT_SOME_FAILED, "{text}");
    assert!(
        fake.hits().iter().all(|h| h.path.contains("key=")),
        "the key really was sent in the URL, so the absence below is redaction, not omission"
    );
    assert!(
        !text.contains(FAKE_KEY),
        "printed output leaked the key: {text}"
    );
    let mut stored = 0;
    for file in files(&out) {
        let body = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            !body.contains(FAKE_KEY),
            "{} leaked the key",
            file.display()
        );
        stored += usize::from(body.contains("client_error"));
    }
    assert!(
        stored >= 1,
        "the failure was stored, classified, with the key scrubbed"
    );
}

#[tokio::test]
async fn an_unreachable_server_is_a_classified_error_and_the_key_stays_out() {
    // Nothing listens on this port: a transport error whose text, in a careless
    // client, would include the request URL and so the key.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = listener.local_addr().unwrap();
    drop(listener);
    let out = tmp("dead");
    let env = MapEnv::new().with("FERRITE_GEMINI_API_KEY", FAKE_KEY);
    let cmd = argv(&[
        "--provider",
        "gemini",
        "--base-url",
        &format!("http://{dead}"),
        "--model",
        "fake-model",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "1",
        "--modes",
        "off",
        "--max-attempts",
        "1",
        "--max-consecutive-failures",
        "100",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (_, text) = run(&cmd, &env, Arc::new(BackoffRecorder::default())).await;
    assert!(!text.contains(FAKE_KEY), "{text}");
    for file in files(&out) {
        let body = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(!body.contains(FAKE_KEY), "{}", file.display());
        for (at, _) in body.match_indices("key=") {
            assert!(
                body[at + 4..].starts_with("<redacted>"),
                "a request URL's key= query may be stored only redacted: {} near {:?}",
                file.display(),
                &body[at.saturating_sub(20)..(at + 24).min(body.len())]
            );
        }
    }
    let results = store::read_all(&out).unwrap();
    assert!(results
        .by_key
        .values()
        .all(|r| r.error.as_ref().is_some_and(|e| e.class == "transport")));
}

#[tokio::test]
async fn a_hosted_provider_with_no_key_is_refused_before_any_request() {
    let fake = spawn(|_, _| Resp::ok(&gemini("[]"))).await;
    let out = tmp("nokey");
    let cmd = argv(&[
        "--provider",
        "gemini",
        "--base-url",
        &fake.url(),
        "--model",
        "m",
        "--suite",
        "agentdojo/slack",
        "--batch-size",
        "1",
        "--timeout-secs",
        "3600",
        "--out",
        out.to_str().unwrap(),
    ]);
    let (code, text) = run(&cmd, &MapEnv::new(), Arc::new(BackoffRecorder::default())).await;
    assert_eq!(code, EXIT_USAGE, "{text}");
    assert_eq!(fake.hits().len(), 0, "no request is made without a key");
}
