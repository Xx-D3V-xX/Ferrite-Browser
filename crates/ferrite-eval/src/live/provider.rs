//! The model stack a live run calls through, and what it learns about each call.
//!
//! ```text
//! Probe(case-level) -> Cache -> Throttle -> Budget -> Probe(live) -> backend
//! ```
//!
//! Everything cross-cutting is `ferrite-model`'s own decorator, reused rather than
//! re-implemented (`docs/REBUILD_DIRECTIVE.md` §10.3, §10.5), but in an order
//! chosen for a quota rather than for the app:
//!
//! - [`Cache`] is outermost, so a response already recorded costs nothing at all:
//!   it consumes no pause, no retry slot and none of the `--max-calls` cap. At
//!   temperature 0 a response is a pure function of its request, so a re-run of an
//!   unchanged case is free, and so is a second mode whose first prompt is
//!   identical to the first's (the guard only changes what happens *after* an
//!   action is refused).
//! - [`Throttle`] is the pacing (`--pause-ms` as a one-token bucket), the retry
//!   policy (`--max-attempts`), exponential backoff with full jitter, and the
//!   `Retry-After` the provider sent (capped at `--backoff-max-ms`).
//! - [`Budget`] sits *inside* the throttle, so it counts every request that is
//!   actually sent, a retry included. `--max-calls` is therefore a hard cap on
//!   requests to the backend, the number a quota is measured in. When it is spent
//!   it writes a partial-results ledger and refuses every further call.
//!
//! The two [`Probe`]s are this crate's addition and change no behaviour. The outer
//! one sees each call as the pipeline made it (its final result, its latency, the
//! typed error that survived the retries); the inner one sees only what reached
//! the backend, so cache hits and retries can be told apart: `live_attempts -
//! (logical - cache_hits)` is the number of retries. [`Redactor`] scrubs every
//! error text a probe keeps, and the budget's ledger file.
//!
//! # Classifying a failure
//!
//! A model failing to answer is not one thing. A **rate limit, timeout, outage or
//! rejected key** says nothing about the model's behaviour: the case did not run,
//! and must be retried, never scored. A **malformed or empty answer** is the
//! model's behaviour: the fingerprint falls back to empty (`CLAUDE.md`: fail to
//! empty, never a bypass) or the agent's loop ends, and the case is scored as such.
//! [`ErrorClass::is_infrastructure`] draws that line in one place.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ferrite_core::Clock;
use ferrite_model::backends::{GeminiProvider, OllamaProvider};
use ferrite_model::decorators::{BackoffPolicy, RateLimit};
use ferrite_model::testing::Sleeper;
use ferrite_model::{
    Budget, Cache, CompletionRequest, CompletionResponse, EnvSource, LayeredEnv, MapEnv,
    ModelConfig, ModelError, ModelProvider, ModelTier, ProviderCapabilities, ProviderId,
    SecretStore, Throttle, ThrottleConfig,
};

use super::config::{LiveArgs, ModelTags, ProviderKind};
use super::record::Redactor;

/// What kind of failure a model call ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// HTTP 429 that survived every retry.
    RateLimited,
    /// HTTP 5xx that survived every retry.
    ServerError,
    /// The per-request timeout elapsed on every attempt.
    Timeout,
    /// The request never reached the provider.
    Transport,
    /// A 4xx other than 429: a bad key, a retired model tag, a bad request.
    Client,
    /// The `--max-calls` ceiling is spent.
    BudgetExhausted,
    /// No key, or a configuration the provider rejected.
    Config,
    /// The model answered, but with nothing usable: malformed JSON, off-schema,
    /// empty, oversized.
    ModelOutput,
    /// Anything else.
    Other,
}

impl ErrorClass {
    /// Classifies a [`ModelError`].
    #[must_use]
    pub fn of(error: &ModelError) -> Self {
        match error {
            ModelError::RateLimited { .. } => Self::RateLimited,
            ModelError::ServerError { .. } => Self::ServerError,
            ModelError::Timeout { .. } => Self::Timeout,
            ModelError::Transport { .. } => Self::Transport,
            ModelError::ClientError { .. } => Self::Client,
            ModelError::BudgetExhausted { .. } => Self::BudgetExhausted,
            ModelError::MissingApiKey { .. }
            | ModelError::ModelTagUnavailable { .. }
            | ModelError::Config(_) => Self::Config,
            ModelError::MalformedJson { .. }
            | ModelError::EmptyResponse { .. }
            | ModelError::ResponseTooLarge { .. }
            | ModelError::SchemaViolation { .. } => Self::ModelOutput,
            _ => Self::Other,
        }
    }

    /// The stable name stored in a result.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::ServerError => "server_error",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Client => "client_error",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Config => "config",
            Self::ModelOutput => "model_output",
            Self::Other => "other",
        }
    }

    /// The call failed for a reason that says nothing about the model's behaviour:
    /// the case did not run, and a result built on it would be an artifact.
    #[must_use]
    pub const fn is_infrastructure(self) -> bool {
        !matches!(self, Self::ModelOutput)
    }

    /// A retry could plausibly succeed. A spent budget, a missing key and a
    /// rejected request return the same answer next time.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ServerError | Self::Timeout | Self::Transport
        )
    }
}

/// One call, as a [`Probe`] saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEvent {
    /// The request's label (`fingerprint`, `agent step`).
    pub label: String,
    /// The call succeeded.
    pub ok: bool,
    /// The response came from the cache.
    pub cache_hit: bool,
    /// Prompt tokens reported.
    pub prompt_tokens: u32,
    /// Completion tokens reported.
    pub eval_tokens: u32,
    /// Wall-clock time.
    pub latency_ms: u64,
    /// The failure, when it failed.
    pub error: Option<(ErrorClass, String)>,
}

/// Everything a [`Probe`] has seen since it was last drained.
#[derive(Debug, Default)]
pub struct ProbeStats {
    events: Mutex<Vec<CallEvent>>,
}

impl ProbeStats {
    /// Removes and returns the events so far.
    #[must_use]
    pub fn drain(&self) -> Vec<CallEvent> {
        std::mem::take(&mut *self.events.lock().expect("probe stats poisoned"))
    }

    fn push(&self, event: CallEvent) {
        self.events
            .lock()
            .expect("probe stats poisoned")
            .push(event);
    }
}

/// A transparent decorator that records every call.
#[derive(Debug)]
pub struct Probe<P> {
    inner: P,
    stats: Arc<ProbeStats>,
    redactor: Redactor,
}

impl<P: ModelProvider> Probe<P> {
    /// Wraps `inner`, recording into `stats`. Error text is passed through
    /// `redactor` before it is kept.
    #[must_use]
    pub fn new(inner: P, stats: Arc<ProbeStats>, redactor: Redactor) -> Self {
        Self {
            inner,
            stats,
            redactor,
        }
    }
}

#[async_trait]
impl<P: ModelProvider> ModelProvider for Probe<P> {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let label = req.label.clone();
        let started = Instant::now();
        let outcome = self.inner.complete(req).await;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let event = match &outcome {
            Ok(response) => CallEvent {
                label,
                ok: true,
                cache_hit: response.provenance.cache_hit,
                prompt_tokens: response.usage.prompt_eval_count,
                eval_tokens: response.usage.eval_count,
                latency_ms,
                error: None,
            },
            Err(error) => CallEvent {
                label,
                ok: false,
                cache_hit: false,
                prompt_tokens: 0,
                eval_tokens: 0,
                latency_ms,
                error: Some((
                    ErrorClass::of(error),
                    self.redactor.bounded(&error.to_string()),
                )),
            },
        };
        self.stats.push(event);
        outcome
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

/// How the stack is paced and capped.
#[derive(Debug, Clone, PartialEq)]
pub struct StackConfig {
    /// Hard cap on calls (cache hits included) in this invocation.
    pub max_calls: u32,
    /// The least time between two calls.
    pub pause: Duration,
    /// Tries per call, first included.
    pub max_attempts: u32,
    /// The first backoff's ceiling.
    pub backoff_base: Duration,
    /// The longest wait, a server's `Retry-After` included.
    pub backoff_max: Duration,
    /// Per-request timeout.
    pub timeout: Duration,
    /// The response cache, or `None` for no caching.
    pub cache_dir: Option<std::path::PathBuf>,
    /// Where the budget writes its ledger when it runs out.
    pub partial_results_path: std::path::PathBuf,
}

impl StackConfig {
    /// The stack the arguments describe, with its files under `args.out_dir`.
    #[must_use]
    pub fn from_args(args: &LiveArgs, use_cache: bool) -> Self {
        Self {
            max_calls: args.max_calls,
            pause: args.pause,
            max_attempts: args.max_attempts,
            backoff_base: args.backoff_base,
            backoff_max: args.backoff_max,
            timeout: args.timeout,
            cache_dir: (use_cache && !args.no_cache).then(|| args.out_dir.join("model-cache")),
            partial_results_path: args.out_dir.join("budget-partial-results.json"),
        }
    }

    fn throttle(&self) -> ThrottleConfig {
        ThrottleConfig {
            // One call at a time: a batch run exists to stay under a limit, not to
            // finish fast.
            max_in_flight: 1,
            rate: (!self.pause.is_zero()).then(|| RateLimit {
                capacity: 1,
                per_second: 1.0 / self.pause.as_secs_f64(),
            }),
            timeout: self.timeout,
            backoff: BackoffPolicy {
                base: self.backoff_base,
                max: self.backoff_max,
                max_retries: self.max_attempts.saturating_sub(1),
                jitter_seed: 42,
            },
        }
    }
}

type CacheHandle = Option<Arc<Cache<Arc<dyn ModelProvider>>>>;
type SummaryFn = Box<dyn Fn() -> StackSummary + Send + Sync>;
type FlushFn = Box<dyn Fn() + Send + Sync>;

/// What the stack has spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackSummary {
    /// The `--max-calls` cap.
    pub cap: u32,
    /// Requests sent to the backend, retries included: what the cap counts.
    pub backend_calls: u32,
    /// Calls the cache answered.
    pub cache_hits: u32,
    /// Prompt tokens the backend reported.
    pub prompt_tokens: u64,
    /// Completion tokens the backend reported.
    pub eval_tokens: u64,
}

/// A provider wrapped in the full stack, plus the handles a runner reads.
pub struct ModelStack {
    /// The provider every call goes through.
    pub provider: Arc<dyn ModelProvider>,
    /// Calls as the pipeline made them.
    pub outer: Arc<ProbeStats>,
    /// Attempts that reached the backend.
    pub live: Arc<ProbeStats>,
    summary: SummaryFn,
    flush: FlushFn,
}

impl std::fmt::Debug for ModelStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelStack")
            .field("provider", &self.provider.id())
            .field("summary", &(self.summary)())
            .finish()
    }
}

impl ModelStack {
    /// What has been spent so far.
    #[must_use]
    pub fn summary(&self) -> StackSummary {
        (self.summary)()
    }

    /// Writes the budget ledger (scrubbed of secrets) and the cache's hit
    /// statistics. Best effort: a failure here must not turn a good run into a bad
    /// one.
    pub fn flush(&self) {
        (self.flush)();
    }
}

/// Wraps `inner` in the stack. Generic over the clock and sleeper so a test can
/// assert on backoff without sleeping.
pub fn build_stack<P: ModelProvider + 'static>(
    inner: P,
    config: &StackConfig,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
    redactor: &Redactor,
) -> ModelStack {
    let live = Arc::new(ProbeStats::default());
    let outer = Arc::new(ProbeStats::default());

    let base: Arc<dyn ModelProvider> = Arc::new(inner);
    let counted: Arc<dyn ModelProvider> =
        Arc::new(Probe::new(base, live.clone(), redactor.clone()));
    let budget = Arc::new(Budget::new(
        counted,
        config.max_calls,
        &config.partial_results_path,
        clock.clone(),
    ));
    let throttled: Arc<dyn ModelProvider> = Arc::new(Throttle::new(
        budget.clone(),
        config.throttle(),
        clock.clone(),
        sleeper,
    ));
    let (cached, cache_handle): (Arc<dyn ModelProvider>, CacheHandle) = match &config.cache_dir {
        Some(dir) => {
            let cache = Arc::new(Cache::new(throttled, dir, clock));
            (cache.clone(), Some(cache))
        }
        None => (throttled, None),
    };
    let provider: Arc<dyn ModelProvider> =
        Arc::new(Probe::new(cached, outer.clone(), redactor.clone()));

    let cap = config.max_calls;
    let summary_budget = budget.clone();
    let summary_cache = cache_handle.clone();
    let flush_budget = budget;
    let ledger = config.partial_results_path.clone();
    let scrub = redactor.clone();
    ModelStack {
        provider,
        outer,
        live,
        summary: Box::new(move || {
            let b = summary_budget.summary();
            StackSummary {
                cap,
                backend_calls: b.calls_used,
                cache_hits: summary_cache
                    .as_ref()
                    .map_or(0, |c| u32::try_from(c.stats().hits).unwrap_or(u32::MAX)),
                prompt_tokens: b.prompt_tokens,
                eval_tokens: b.eval_tokens,
            }
        }),
        flush: Box::new(move || {
            let _ = flush_budget.write_partial_results();
            // The ledger records each failed call's error text, which a provider can
            // build from what a server sent back; scrub it before it stays on disk.
            if let Ok(text) = std::fs::read_to_string(&ledger) {
                let clean = scrub.scrub(&text);
                if clean != text {
                    let _ = std::fs::write(&ledger, clean);
                }
            }
            if let Some(cache) = &cache_handle {
                let _ = cache.flush_stats();
            }
        }),
    }
}

/// Builds the real (or mock) backend for `args` and wraps it.
///
/// Keys are resolved by `ferrite-model` from `env` and then `store` (the OS
/// keyring), and nowhere else.
///
/// # Errors
///
/// [`ModelError::MissingApiKey`] when Gemini or Ollama Cloud has no key anywhere,
/// [`ModelError::Config`] for a malformed setting.
pub fn connect(
    args: &LiveArgs,
    tags: &ModelTags,
    env: &dyn EnvSource,
    store: &dyn SecretStore,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
) -> Result<ModelStack, ModelError> {
    let redactor = Redactor::from_env(env);
    let kind = args.provider.unwrap_or(ProviderKind::Mock);
    if kind == ProviderKind::Mock {
        let config = StackConfig::from_args(args, false);
        let mock = super::mock_model::build(args.mock_behavior);
        return Ok(build_stack(mock, &config, clock, sleeper, &redactor));
    }

    let config = StackConfig::from_args(args, true);
    let mut flags = MapEnv::new()
        .with(ModelTier::Small.env_var(), tags.small.clone())
        .with(ModelTier::Main.env_var(), tags.main.clone())
        .with(
            "FERRITE_MODEL_CACHE_DIR",
            args.out_dir.join("model-cache").display().to_string(),
        )
        .with("FERRITE_MODEL_CALL_BUDGET", args.max_calls.to_string());
    if let Some(url) = &args.base_url {
        let var = match kind {
            ProviderKind::Gemini => "FERRITE_GEMINI_BASE_URL",
            _ => "FERRITE_OLLAMA_BASE_URL",
        };
        flags = flags.with(var, url.clone());
    }
    let layered = LayeredEnv::new(&flags, env);
    let model_config = ModelConfig::load(&layered)?;
    match kind {
        ProviderKind::Gemini => {
            let provider = GeminiProvider::from_config(&model_config, ModelTier::Main, env, store)?;
            Ok(build_stack(provider, &config, clock, sleeper, &redactor))
        }
        ProviderKind::Ollama => {
            let provider = OllamaProvider::from_config(&model_config, ModelTier::Main, env, store)?;
            Ok(build_stack(provider, &config, clock, sleeper, &redactor))
        }
        ProviderKind::Mock => unreachable!("handled above"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use ferrite_core::FixedClock;
    use ferrite_model::testing::RecordingSleeper;
    use ferrite_model::{Message, MockProvider, MockStep, ProviderId};

    use super::*;

    fn req(label: &str) -> CompletionRequest {
        CompletionRequest::new("tag", ModelTier::Main, vec![Message::user("hi")]).with_label(label)
    }

    fn rate_limited(after: Option<u64>) -> MockStep {
        MockStep::Fail(ModelError::RateLimited {
            provider: ProviderId::Mock,
            retry_after: after.map(Duration::from_secs),
        })
    }

    fn config(dir: &std::path::Path) -> StackConfig {
        StackConfig {
            max_calls: 100,
            pause: Duration::ZERO,
            max_attempts: 5,
            backoff_base: Duration::from_millis(500),
            backoff_max: Duration::from_secs(90),
            timeout: Duration::from_secs(30),
            cache_dir: None,
            partial_results_path: dir.join("partial.json"),
        }
    }

    fn tmp(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-live-provider-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stack(mock: MockProvider, config: &StackConfig) -> (ModelStack, Arc<RecordingSleeper>) {
        let sleeper = Arc::new(RecordingSleeper::new());
        let clock: Arc<dyn Clock> = Arc::new(FixedClock::at_epoch());
        let stack = build_stack(mock, config, clock, sleeper.clone(), &Redactor::default());
        (stack, sleeper)
    }

    #[tokio::test]
    async fn a_rate_limit_is_retried_honouring_retry_after_and_the_call_succeeds() {
        let dir = tmp("retry-after");
        let mock = MockProvider::new()
            .push(rate_limited(Some(30)))
            .push(rate_limited(Some(7)))
            .push_content("ok");
        let (stack, sleeper) = stack(mock, &config(&dir));

        let response = stack
            .provider
            .complete(req("agent step"))
            .await
            .expect("recovers");
        assert_eq!(response.content, "ok");

        // The waits are exactly what the provider asked for, not a guess.
        assert_eq!(
            sleeper.recorded(),
            vec![Duration::from_secs(30), Duration::from_secs(7)]
        );
        let outer = stack.outer.drain();
        assert_eq!(outer.len(), 1, "one logical call");
        assert!(outer[0].ok);
        let live = stack.live.drain();
        assert_eq!(live.len(), 3, "three attempts reached the backend");
        assert_eq!(live.iter().filter(|e| !e.ok).count(), 2);
        assert!(live
            .iter()
            .all(|e| e.ok || matches!(e.error, Some((ErrorClass::RateLimited, _)))));
    }

    #[tokio::test]
    async fn a_retry_after_longer_than_the_cap_is_capped_not_obeyed_for_an_hour() {
        let dir = tmp("cap");
        let mut cfg = config(&dir);
        cfg.backoff_max = Duration::from_secs(60);
        let mock = MockProvider::new()
            .push(rate_limited(Some(3600)))
            .push_content("ok");
        let (stack, sleeper) = stack(mock, &cfg);
        stack.provider.complete(req("x")).await.expect("recovers");
        assert_eq!(sleeper.recorded(), vec![Duration::from_secs(60)]);
    }

    #[tokio::test]
    async fn without_retry_after_the_backoff_grows_and_is_bounded_by_the_cap() {
        let dir = tmp("backoff");
        let mut cfg = config(&dir);
        cfg.max_attempts = 4;
        cfg.backoff_base = Duration::from_secs(2);
        cfg.backoff_max = Duration::from_secs(10);
        let mock = MockProvider::new()
            .push(rate_limited(None))
            .push(rate_limited(None))
            .push(rate_limited(None))
            .push_content("ok");
        let (stack, sleeper) = stack(mock, &cfg);
        stack
            .provider
            .complete(req("x"))
            .await
            .expect("recovers on the 4th try");
        let waits = sleeper.recorded();
        assert_eq!(waits.len(), 3);
        // Full jitter: each wait is in [0, min(max, base * 2^n)].
        for (n, wait) in waits.iter().enumerate() {
            let ceiling = Duration::from_secs(2 * (1 << n)).min(Duration::from_secs(10));
            assert!(*wait <= ceiling, "wait {n} = {wait:?} exceeds {ceiling:?}");
        }
    }

    #[tokio::test]
    async fn when_every_attempt_fails_the_typed_error_reaches_the_caller_after_exactly_max_attempts(
    ) {
        let dir = tmp("exhaust");
        let mut cfg = config(&dir);
        cfg.max_attempts = 3;
        let mock = MockProvider::new().always(|_| rate_limited(None));
        let (stack, sleeper) = stack(mock, &cfg);
        let err = stack
            .provider
            .complete(req("x"))
            .await
            .expect_err("never succeeds");
        assert!(matches!(err, ModelError::RateLimited { .. }), "{err:?}");
        assert_eq!(
            stack.live.drain().len(),
            3,
            "no more attempts than --max-attempts"
        );
        assert_eq!(sleeper.count(), 2);
        let outer = stack.outer.drain();
        assert_eq!(outer[0].error.as_ref().unwrap().0, ErrorClass::RateLimited);
    }

    #[tokio::test]
    async fn a_client_error_is_not_retried() {
        let dir = tmp("client");
        let mock = MockProvider::new().always(|_| {
            MockStep::Fail(ModelError::ClientError {
                provider: ProviderId::Mock,
                status: 401,
                body_excerpt: "bad key".to_string(),
            })
        });
        let (stack, sleeper) = stack(mock, &config(&dir));
        let err = stack
            .provider
            .complete(req("x"))
            .await
            .expect_err("rejected");
        assert!(matches!(err, ModelError::ClientError { .. }));
        assert_eq!(
            stack.live.drain().len(),
            1,
            "retrying a rejected request spends quota for nothing"
        );
        assert_eq!(sleeper.count(), 0);
    }

    #[tokio::test]
    async fn the_budget_is_a_hard_cap_and_leaves_a_ledger() {
        let dir = tmp("budget");
        let mut cfg = config(&dir);
        cfg.max_calls = 2;
        let mock = MockProvider::new().always_content("ok");
        let (stack, _) = stack(mock, &cfg);
        stack.provider.complete(req("a")).await.expect("1");
        stack.provider.complete(req("b")).await.expect("2");
        let err = stack
            .provider
            .complete(req("c"))
            .await
            .expect_err("budget spent");
        assert!(matches!(err, ModelError::BudgetExhausted { .. }), "{err:?}");
        assert_eq!(
            stack.live.drain().len(),
            2,
            "the third call never reached the backend"
        );
        assert_eq!(stack.summary().backend_calls, 2);
        assert!(dir.join("partial.json").exists(), "the ledger was written");
        let outer = stack.outer.drain();
        assert_eq!(
            outer[2].error.as_ref().unwrap().0,
            ErrorClass::BudgetExhausted
        );
    }

    #[tokio::test]
    async fn a_cached_response_costs_no_attempt_and_the_probes_can_tell() {
        let dir = tmp("cache");
        let mut cfg = config(&dir);
        cfg.cache_dir = Some(dir.join("cache"));
        let mock = MockProvider::new().always_content("ok");
        let (stack, _) = stack(mock, &cfg);
        stack
            .provider
            .complete(req("fingerprint"))
            .await
            .expect("miss");
        stack
            .provider
            .complete(req("fingerprint"))
            .await
            .expect("hit");
        let outer = stack.outer.drain();
        assert_eq!(outer.len(), 2);
        assert!(!outer[0].cache_hit && outer[1].cache_hit);
        assert_eq!(
            stack.live.drain().len(),
            1,
            "only the miss reached the backend"
        );
        assert_eq!(stack.summary().cache_hits, 1);
    }

    #[tokio::test]
    async fn pacing_waits_between_calls_through_the_token_bucket() {
        let dir = tmp("pace");
        let mut cfg = config(&dir);
        cfg.pause = Duration::from_millis(2000);
        let mock = MockProvider::new().always_content("ok");
        let clock_handle = Arc::new(FixedClock::at_epoch());
        let hook_clock = clock_handle.clone();
        let sleeper_with_clock = Arc::new(RecordingSleeper::with_hook(move |d| {
            hook_clock.advance(chrono::TimeDelta::from_std(d).unwrap());
        }));
        let stack = build_stack(
            mock,
            &cfg,
            clock_handle as Arc<dyn Clock>,
            sleeper_with_clock.clone(),
            &Redactor::default(),
        );
        for _ in 0..3 {
            stack.provider.complete(req("x")).await.expect("ok");
        }
        let waits = sleeper_with_clock.recorded();
        assert_eq!(
            waits.len(),
            2,
            "the first call is free, each later one waits the pause"
        );
        assert!(
            waits.iter().all(|w| *w >= Duration::from_millis(1999)),
            "{waits:?}"
        );
    }

    #[test]
    fn infrastructure_failures_are_told_apart_from_the_models_own_bad_output() {
        let provider = ProviderId::Mock;
        for infra in [
            ModelError::RateLimited {
                provider,
                retry_after: None,
            },
            ModelError::ServerError {
                provider,
                status: 503,
                body_excerpt: String::new(),
            },
            ModelError::Timeout {
                provider,
                after: Duration::from_secs(1),
            },
            ModelError::Transport {
                provider,
                detail: String::new(),
            },
            ModelError::ClientError {
                provider,
                status: 404,
                body_excerpt: String::new(),
            },
            ModelError::Config("x".into()),
        ] {
            assert!(ErrorClass::of(&infra).is_infrastructure(), "{infra:?}");
        }
        for bad_output in [
            ModelError::MalformedJson {
                provider,
                detail: String::new(),
            },
            ModelError::EmptyResponse { provider },
            ModelError::SchemaViolation {
                detail: String::new(),
            },
        ] {
            let class = ErrorClass::of(&bad_output);
            assert!(!class.is_infrastructure(), "{bad_output:?}");
            assert_eq!(class, ErrorClass::ModelOutput);
        }
        assert!(ErrorClass::RateLimited.is_retryable());
        assert!(!ErrorClass::BudgetExhausted.is_retryable());
        assert!(!ErrorClass::Client.is_retryable());
    }

    #[tokio::test]
    async fn a_probe_never_stores_a_key_an_error_happens_to_echo() {
        let key = "AIzaFAKEKEYFORTESTSONLY0123456789abc";
        let dir = tmp("redact");
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let mock = MockProvider::new().always(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            MockStep::Fail(ModelError::ClientError {
                provider: ProviderId::Mock,
                status: 400,
                body_excerpt: format!("API key not valid: {key}"),
            })
        });
        let sleeper = Arc::new(RecordingSleeper::new());
        let clock: Arc<dyn Clock> = Arc::new(FixedClock::at_epoch());
        let stack = build_stack(mock, &config(&dir), clock, sleeper, &Redactor::new([key]));
        let _ = stack.provider.complete(req("x")).await;
        for event in stack.outer.drain().into_iter().chain(stack.live.drain()) {
            let (_, text) = event.error.expect("failed");
            assert!(!text.contains(key), "{text}");
            assert!(text.contains(crate::live::record::REDACTED));
        }
    }
}
