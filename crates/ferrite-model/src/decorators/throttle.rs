//! Throttling, backoff and the per-request timeout — §10.3's third
//! mechanism.
//!
//! Four things, all required, all wrapped around any provider:
//!
//! - a **global semaphore** capping in-flight requests (default 2);
//! - a **token bucket** limiting the sustained rate;
//! - **exponential backoff with full jitter** on `429`/`5xx`, honouring
//!   `Retry-After` when the provider sends one;
//! - a **per-request timeout**, so a call that never returns is bounded.
//!
//! # Why time is injected
//!
//! R8 forbids assertions on wall-clock behaviour, and a test suite that
//! really sleeps through exponential backoff is a suite that gets disabled.
//! So the bucket reads a [`Clock`] and every wait goes through a
//! [`Sleeper`]. In production those are `SystemClock` and
//! [`TokioSleeper`](crate::testing::TokioSleeper); in a test they are
//! `FixedClock` and [`RecordingSleeper`](crate::testing::RecordingSleeper),
//! which records the delay and advances the clock by exactly that much. The
//! backoff schedule is then something a test reads off a vector rather than
//! something it measures.
//!
//! The timeout is built the same way — a `select!` between the inner call
//! and `sleeper.sleep(timeout)` — rather than out of `tokio::time::timeout`,
//! so that the hang case in §10.4's adversarial matrix is provable in
//! microseconds instead of being asserted by waiting.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrite_core::Clock;
use rand::{Rng, SeedableRng, rngs::StdRng};
use tokio::sync::Semaphore;

use crate::error::ModelError;
use crate::provider::{ModelProvider, ProviderCapabilities, ProviderId};
use crate::request::CompletionRequest;
use crate::response::CompletionResponse;
use crate::testing::Sleeper;

/// A sustained-rate limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    /// Burst size — how many calls may go out back-to-back from a full
    /// bucket.
    pub capacity: u32,
    /// Sustained refill rate.
    pub per_second: f64,
}

/// Exponential backoff with full jitter.
///
/// Full jitter (`delay = uniform(0, cap)`) rather than equal or decorrelated
/// jitter, because the failure being smoothed is many workers colliding on
/// one quota: full jitter spreads a retry storm widest, at the cost of
/// occasionally retrying sooner than a fixed schedule would.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffPolicy {
    /// The first attempt's ceiling; attempt *n*'s is `base * 2^n`.
    pub base: Duration,
    /// The ceiling never grows past this.
    pub max: Duration,
    /// How many times a retryable failure is retried before it is returned.
    pub max_retries: u32,
    /// Seeds the jitter RNG. Fixed by default, so a test asserts an exact
    /// schedule (R8) and two processes still differ if given different
    /// seeds.
    pub jitter_seed: u64,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            max: Duration::from_secs(32),
            max_retries: 4,
            jitter_seed: 42,
        }
    }
}

/// How a [`Throttle`] is set up.
#[derive(Debug, Clone, PartialEq)]
pub struct ThrottleConfig {
    /// In-flight cap (§10.3's default is 2).
    pub max_in_flight: usize,
    /// Sustained rate, or `None` to rely on the semaphore alone.
    pub rate: Option<RateLimit>,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Retry policy.
    pub backoff: BackoffPolicy,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            max_in_flight: crate::config::DEFAULT_MAX_IN_FLIGHT,
            rate: None,
            timeout: Duration::from_secs(crate::config::DEFAULT_TIMEOUT_SECS),
            backoff: BackoffPolicy::default(),
        }
    }
}

impl ThrottleConfig {
    /// The configuration implied by a loaded [`ModelConfig`](crate::ModelConfig).
    #[must_use]
    pub fn from_model_config(config: &crate::config::ModelConfig) -> Self {
        Self {
            max_in_flight: config.max_in_flight,
            timeout: config.request_timeout,
            ..Self::default()
        }
    }
}

/// A clock-driven token bucket.
///
/// Fractional tokens are kept so that a slow refill rate accumulates
/// correctly instead of rounding to zero on every short interval.
#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    per_second: f64,
    last_refill: DateTime<Utc>,
}

impl TokenBucket {
    fn new(limit: RateLimit, now: DateTime<Utc>) -> Self {
        Self {
            capacity: f64::from(limit.capacity),
            tokens: f64::from(limit.capacity),
            per_second: limit.per_second,
            last_refill: now,
        }
    }

    /// Takes a token if one is available, or reports how long to wait.
    ///
    /// `Ok(())` means a token was consumed. `Err(d)` means wait `d` and ask
    /// again — the caller sleeps and retries rather than this function
    /// blocking, so the wait goes through the injected [`Sleeper`].
    fn try_take(&mut self, now: DateTime<Utc>) -> Result<(), Duration> {
        let elapsed = (now - self.last_refill).num_milliseconds().max(0);
        #[allow(clippy::cast_precision_loss)]
        let refill = (elapsed as f64 / 1000.0) * self.per_second;
        self.tokens = (self.tokens + refill).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Ok(());
        }
        if self.per_second <= 0.0 {
            // A zero rate is a stopped bucket; waiting cannot help, so this
            // is configured-to-deadlock and the config is the bug. Report a
            // long wait rather than spinning.
            return Err(Duration::from_secs(u64::MAX / 2));
        }
        let deficit = 1.0 - self.tokens;
        Err(Duration::from_secs_f64(deficit / self.per_second))
    }
}

/// Wraps any provider with the rate-limit discipline of §10.3.
#[derive(Debug)]
pub struct Throttle<P: ModelProvider> {
    inner: P,
    config: ThrottleConfig,
    semaphore: Semaphore,
    bucket: Option<Mutex<TokenBucket>>,
    clock: Arc<dyn Clock>,
    sleeper: Arc<dyn Sleeper>,
    jitter: Mutex<StdRng>,
}

impl<P: ModelProvider> Throttle<P> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(
        inner: P,
        config: ThrottleConfig,
        clock: Arc<dyn Clock>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Self {
        let bucket = config
            .rate
            .map(|rate| Mutex::new(TokenBucket::new(rate, clock.now())));
        Self {
            semaphore: Semaphore::new(config.max_in_flight.max(1)),
            jitter: Mutex::new(StdRng::seed_from_u64(config.backoff.jitter_seed)),
            bucket,
            clock,
            sleeper,
            config,
            inner,
        }
    }

    /// The delay before retry `attempt` (0-based), given what the provider
    /// asked for.
    ///
    /// `Retry-After` wins outright when present: the provider has told us
    /// when it will serve again, and guessing shorter wastes a call while
    /// guessing longer wastes time.
    fn backoff_delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(after) = retry_after {
            return after.min(self.config.backoff.max);
        }
        let exponent = attempt.min(16);
        let ceiling = self
            .config
            .backoff
            .base
            .saturating_mul(1u32 << exponent)
            .min(self.config.backoff.max);
        let jittered = self
            .jitter
            .lock()
            .expect("jitter RNG poisoned")
            .gen_range(0.0..=ceiling.as_secs_f64());
        Duration::from_secs_f64(jittered)
    }

    /// Waits until the token bucket allows a call. No-op when unconfigured.
    async fn await_token(&self) {
        let Some(bucket) = &self.bucket else {
            return;
        };
        loop {
            let wait = {
                let mut bucket = bucket.lock().expect("token bucket poisoned");
                match bucket.try_take(self.clock.now()) {
                    Ok(()) => return,
                    Err(wait) => wait,
                }
            };
            self.sleeper.sleep(wait).await;
        }
    }

    /// One attempt, bounded by the per-request timeout.
    async fn attempt(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let provider = self.inner.id();
        let timeout = self.config.timeout;
        tokio::select! {
            // Biased so a provider that answered in the same poll as the
            // timeout elapsed is reported as a success, not a timeout.
            biased;
            result = self.inner.complete(req) => result,
            () = self.sleeper.sleep(timeout) => Err(ModelError::Timeout { provider, after: timeout }),
        }
    }
}

#[async_trait]
impl<P: ModelProvider> ModelProvider for Throttle<P> {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ModelError> {
        let mut attempt = 0;
        loop {
            let outcome = {
                let _permit = self
                    .semaphore
                    .acquire()
                    .await
                    .expect("the semaphore is owned by this Throttle and is never closed");
                self.await_token().await;
                self.attempt(req.clone()).await
            };

            match outcome {
                Ok(response) => return Ok(response),
                Err(e) if e.is_retryable() && attempt < self.config.backoff.max_retries => {
                    // The permit is already released: sleeping while holding
                    // it would idle a slot that another request could use.
                    let delay = self.backoff_delay(attempt, e.retry_after());
                    self.sleeper.sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ferrite_core::{FixedClock, SystemClock};

    use super::*;
    use crate::backends::MockProvider;
    use crate::provider::ModelTier;
    use crate::request::Message;
    use crate::testing::RecordingSleeper;

    fn req() -> CompletionRequest {
        CompletionRequest::new("tag", ModelTier::Small, vec![Message::user("hi")])
    }

    fn rate_limited() -> ModelError {
        ModelError::RateLimited {
            provider: ProviderId::Mock,
            retry_after: None,
        }
    }

    fn server_error() -> ModelError {
        ModelError::ServerError {
            provider: ProviderId::Mock,
            status: 503,
            body_excerpt: "upstream unavailable".to_string(),
        }
    }

    /// A throttle whose waiting is recorded rather than endured.
    fn throttle<P: ModelProvider>(
        inner: P,
        config: ThrottleConfig,
    ) -> (Throttle<P>, Arc<RecordingSleeper>) {
        let sleeper = Arc::new(RecordingSleeper::new());
        let throttle = Throttle::new(
            inner,
            config,
            Arc::new(SystemClock),
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
        );
        (throttle, sleeper)
    }

    #[tokio::test]
    async fn a_successful_call_passes_straight_through_without_waiting() {
        let (throttle, sleeper) = throttle(
            MockProvider::new().push_content("ok"),
            ThrottleConfig::default(),
        );
        assert_eq!(throttle.complete(req()).await.expect("ok").content, "ok");
        assert_eq!(sleeper.count(), 0, "nothing should have waited");
    }

    #[tokio::test]
    async fn a_429_is_retried_with_backoff_rather_than_propagated_raw() {
        // §10.4's adversarial condition 5: a 429 must go through backoff.
        let inner = Arc::new(
            MockProvider::new()
                .push_error(rate_limited())
                .push_error(rate_limited())
                .push_content("recovered"),
        );
        let (throttle, sleeper) = throttle(Arc::clone(&inner), ThrottleConfig::default());

        let response = throttle.complete(req()).await.expect("third attempt wins");
        assert_eq!(response.content, "recovered");
        assert_eq!(inner.call_count(), 3);
        assert_eq!(sleeper.count(), 2, "one backoff wait per retried failure");
    }

    #[tokio::test]
    async fn a_5xx_is_retried_the_same_way() {
        // §10.4's adversarial condition 6.
        let inner = Arc::new(
            MockProvider::new()
                .push_error(server_error())
                .push_content("recovered"),
        );
        let (throttle, sleeper) = throttle(Arc::clone(&inner), ThrottleConfig::default());

        assert_eq!(
            throttle.complete(req()).await.expect("ok").content,
            "recovered"
        );
        assert_eq!(inner.call_count(), 2);
        assert_eq!(sleeper.count(), 1);
    }

    #[tokio::test]
    async fn backoff_grows_exponentially_and_stays_under_the_ceiling() {
        // R8: the schedule is read off the recorder, not measured.
        let config = ThrottleConfig {
            backoff: BackoffPolicy {
                base: Duration::from_millis(100),
                max: Duration::from_secs(1),
                max_retries: 6,
                jitter_seed: 7,
            },
            ..ThrottleConfig::default()
        };
        let inner = Arc::new(
            MockProvider::new()
                .push_error(rate_limited())
                .push_error(rate_limited())
                .push_error(rate_limited())
                .push_error(rate_limited())
                .push_content("ok"),
        );
        let (throttle, sleeper) = throttle(Arc::clone(&inner), config);
        throttle.complete(req()).await.expect("eventually ok");

        let waits = sleeper.recorded();
        assert_eq!(waits.len(), 4);
        for (attempt, wait) in waits.iter().enumerate() {
            let ceiling = Duration::from_millis(100)
                .saturating_mul(1 << u32::try_from(attempt).expect("small"))
                .min(Duration::from_secs(1));
            assert!(
                *wait <= ceiling,
                "attempt {attempt}: full jitter draws from [0, {ceiling:?}], got {wait:?}"
            );
        }
        assert!(
            waits.iter().any(|w| *w > Duration::from_millis(100)),
            "a purely capped schedule would never exceed the first ceiling: {waits:?}"
        );
    }

    #[tokio::test]
    async fn the_jitter_schedule_is_reproducible_from_its_seed() {
        let schedule = |seed: u64| async move {
            let config = ThrottleConfig {
                backoff: BackoffPolicy {
                    jitter_seed: seed,
                    max_retries: 3,
                    ..BackoffPolicy::default()
                },
                ..ThrottleConfig::default()
            };
            let inner = MockProvider::new()
                .push_error(rate_limited())
                .push_error(rate_limited())
                .push_content("ok");
            let (throttle, sleeper) = throttle(inner, config);
            throttle.complete(req()).await.expect("ok");
            sleeper.recorded()
        };

        assert_eq!(
            schedule(11).await,
            schedule(11).await,
            "R8: same seed, same schedule"
        );
        assert_ne!(
            schedule(11).await,
            schedule(12).await,
            "different seeds must actually spread the retry storm"
        );
    }

    #[tokio::test]
    async fn retry_after_is_honoured_in_place_of_the_computed_backoff() {
        let inner = MockProvider::new()
            .push_error(ModelError::RateLimited {
                provider: ProviderId::Mock,
                retry_after: Some(Duration::from_secs(7)),
            })
            .push_content("ok");
        let (throttle, sleeper) = throttle(inner, ThrottleConfig::default());
        throttle.complete(req()).await.expect("ok");
        assert_eq!(sleeper.recorded(), vec![Duration::from_secs(7)]);
    }

    #[tokio::test]
    async fn retry_after_is_still_capped_by_the_policy_ceiling() {
        // A provider asking us to wait a day should not strand a run for a
        // day; the ceiling is ours to enforce.
        let config = ThrottleConfig {
            backoff: BackoffPolicy {
                max: Duration::from_secs(30),
                ..BackoffPolicy::default()
            },
            ..ThrottleConfig::default()
        };
        let inner = MockProvider::new()
            .push_error(ModelError::RateLimited {
                provider: ProviderId::Mock,
                retry_after: Some(Duration::from_secs(86_400)),
            })
            .push_content("ok");
        let (throttle, sleeper) = throttle(inner, config);
        throttle.complete(req()).await.expect("ok");
        assert_eq!(sleeper.recorded(), vec![Duration::from_secs(30)]);
    }

    #[tokio::test]
    async fn retries_stop_at_the_configured_limit_and_return_the_last_error() {
        let config = ThrottleConfig {
            backoff: BackoffPolicy {
                max_retries: 2,
                ..BackoffPolicy::default()
            },
            ..ThrottleConfig::default()
        };
        let inner = Arc::new(MockProvider::new().always(|_| {
            crate::backends::MockStep::Fail(ModelError::RateLimited {
                provider: ProviderId::Mock,
                retry_after: None,
            })
        }));
        let (throttle, _) = throttle(Arc::clone(&inner), config);

        let err = throttle.complete(req()).await.expect_err("gives up");
        assert!(matches!(err, ModelError::RateLimited { .. }), "{err:?}");
        assert_eq!(
            inner.call_count(),
            3,
            "the original attempt plus two retries"
        );
    }

    #[tokio::test]
    async fn a_terminal_error_is_never_retried() {
        let inner = Arc::new(MockProvider::new().always(|_| {
            crate::backends::MockStep::Fail(ModelError::ClientError {
                provider: ProviderId::Mock,
                status: 401,
                body_excerpt: "bad key".to_string(),
            })
        }));
        let (throttle, sleeper) = throttle(Arc::clone(&inner), ThrottleConfig::default());

        throttle.complete(req()).await.expect_err("401 stands");
        assert_eq!(inner.call_count(), 1, "retrying a 401 only spends quota");
        assert_eq!(sleeper.count(), 0);
    }

    #[tokio::test]
    async fn a_call_that_never_returns_is_bounded_by_the_timeout() {
        // §10.4's adversarial condition 4, proved without waiting: the
        // recording sleeper resolves the timeout arm immediately.
        let config = ThrottleConfig {
            timeout: Duration::from_secs(3),
            backoff: BackoffPolicy {
                max_retries: 0,
                ..BackoffPolicy::default()
            },
            ..ThrottleConfig::default()
        };
        let (throttle, _) = throttle(MockProvider::new().push_hang(), config);

        let err = throttle.complete(req()).await.expect_err("must not hang");
        match err {
            ModelError::Timeout { after, .. } => assert_eq!(after, Duration::from_secs(3)),
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_timeout_is_retried_like_any_other_transient_failure() {
        let config = ThrottleConfig {
            timeout: Duration::from_secs(1),
            backoff: BackoffPolicy {
                max_retries: 1,
                ..BackoffPolicy::default()
            },
            ..ThrottleConfig::default()
        };
        let inner = Arc::new(MockProvider::new().push_hang().push_content("second time"));
        let (throttle, _) = throttle(Arc::clone(&inner), config);
        assert_eq!(
            throttle.complete(req()).await.expect("ok").content,
            "second time"
        );
        assert_eq!(inner.call_count(), 2);
    }

    #[tokio::test]
    async fn the_semaphore_caps_concurrent_calls() {
        /// Counts how many calls are inside `complete` at once.
        #[derive(Debug)]
        struct ConcurrencyProbe {
            in_flight: AtomicUsize,
            peak: AtomicUsize,
        }

        #[async_trait]
        impl ModelProvider for ConcurrencyProbe {
            fn id(&self) -> ProviderId {
                ProviderId::Mock
            }

            async fn complete(
                &self,
                req: CompletionRequest,
            ) -> Result<CompletionResponse, ModelError> {
                let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                tokio::task::yield_now().await;
                self.in_flight.fetch_sub(1, Ordering::SeqCst);
                crate::guard::finalize(
                    ProviderId::Mock,
                    &req,
                    crate::guard::RawCompletion {
                        content: "ok".to_string(),
                        usage: crate::response::TokenUsage::default(),
                    },
                    1024,
                )
            }

            fn capabilities(&self) -> ProviderCapabilities {
                ProviderCapabilities {
                    supports_json_schema: false,
                    context_window_tokens: 1,
                    tier: ModelTier::Small,
                    reaches_network: false,
                }
            }
        }

        let probe = Arc::new(ConcurrencyProbe {
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let throttle = Arc::new(Throttle::new(
            Arc::clone(&probe),
            ThrottleConfig {
                max_in_flight: 2,
                ..ThrottleConfig::default()
            },
            Arc::new(SystemClock),
            // The real sleeper here, deliberately. A RecordingSleeper makes
            // every wait instantaneous *including the timeout*, so any inner
            // future that yields even once would be reported as having
            // exceeded it — correct in virtual time, useless for a test
            // about concurrency. Nothing below asserts on a duration, so R8
            // is untouched: the 60-second timer is registered and never
            // fires.
            Arc::new(crate::testing::TokioSleeper),
        ));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let throttle = Arc::clone(&throttle);
            handles.push(tokio::spawn(async move { throttle.complete(req()).await }));
        }
        for handle in handles {
            handle.await.expect("joins").expect("ok");
        }

        assert!(
            probe.peak.load(Ordering::SeqCst) <= 2,
            "§10.3's default cap is 2 in flight, saw {}",
            probe.peak.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn the_token_bucket_paces_calls_against_the_injected_clock() {
        // Nothing here sleeps for real: the recorder advances the FixedClock
        // by exactly the duration it was asked to wait, so the bucket
        // refills as if time had passed.
        let clock = Arc::new(FixedClock::at_epoch());
        let for_hook = Arc::clone(&clock);
        let sleeper = Arc::new(RecordingSleeper::with_hook(move |d| {
            for_hook.advance(chrono::TimeDelta::from_std(d).expect("representable"));
        }));

        let config = ThrottleConfig {
            rate: Some(RateLimit {
                capacity: 2,
                per_second: 1.0,
            }),
            ..ThrottleConfig::default()
        };
        let throttle = Throttle::new(
            MockProvider::new().always_content("ok"),
            config,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
        );

        for _ in 0..4 {
            throttle.complete(req()).await.expect("ok");
        }

        let waits = sleeper.recorded();
        assert_eq!(
            waits.len(),
            2,
            "a burst of 2 goes out free; the next two each wait for a refill: {waits:?}"
        );
        for wait in &waits {
            assert!(
                (wait.as_secs_f64() - 1.0).abs() < 1e-6,
                "one token per second means a one-second wait, got {wait:?}"
            );
        }
        assert_eq!(
            clock.now(),
            DateTime::UNIX_EPOCH + chrono::TimeDelta::seconds(2)
        );
    }

    #[tokio::test]
    async fn no_token_bucket_means_no_pacing() {
        let (throttle, sleeper) = throttle(
            MockProvider::new().always_content("ok"),
            ThrottleConfig::default(),
        );
        for _ in 0..5 {
            throttle.complete(req()).await.expect("ok");
        }
        assert_eq!(sleeper.count(), 0);
    }

    #[test]
    fn a_throttle_config_takes_its_limits_from_the_loaded_model_config() {
        let env = crate::config::MapEnv::new()
            .with("FERRITE_MODEL_SMALL", "s")
            .with("FERRITE_MODEL_MAIN", "m")
            .with("FERRITE_MODEL_CACHE_DIR", "/tmp/x")
            .with("FERRITE_MODEL_MAX_IN_FLIGHT", "3")
            .with("FERRITE_MODEL_TIMEOUT_SECS", "9");
        let config = crate::config::ModelConfig::load(&env).expect("loads");
        let throttle = ThrottleConfig::from_model_config(&config);
        assert_eq!(throttle.max_in_flight, 3);
        assert_eq!(throttle.timeout, Duration::from_secs(9));
    }
}
