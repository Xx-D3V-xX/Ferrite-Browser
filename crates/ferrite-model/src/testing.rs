//! Test doubles that live in the library rather than behind `cfg(test)`.
//!
//! [`MockProvider`](crate::backends::MockProvider) is a first-class backend
//! by §10.5's own list, and the same reasoning applies to these two: an
//! integration test in `tests/` links the library as an ordinary dependent,
//! so anything gated on `cfg(test)` is invisible to it, and the alternative
//! — a `test-util` feature that every test target has to remember to enable
//! — is a footgun that fails by silently skipping coverage.
//!
//! Neither type can reach the network or read the wall clock.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

/// Something that can wait.
///
/// Injected into [`Throttle`](crate::decorators::Throttle) so that backoff
/// timing is *asserted* rather than *endured*: R8 forbids a test whose
/// outcome depends on how long a real sleep took, and a suite that actually
/// sleeps through exponential backoff is a suite nobody runs.
#[async_trait]
pub trait Sleeper: std::fmt::Debug + Send + Sync {
    /// Waits for `duration`.
    async fn sleep(&self, duration: Duration);
}

/// The real thing.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

type SleepHook = Box<dyn Fn(Duration) + Send + Sync>;

/// A [`Sleeper`] that records what it was asked to wait for and returns
/// immediately.
///
/// It collapses *all* waiting, the per-request timeout included. A test
/// that wants an inner call to survive the timeout must therefore either
/// keep that call synchronous (so it resolves in the same poll) or use
/// [`TokioSleeper`]: against this sleeper, a provider that yields even once
/// has, in virtual time, taken longer than the timeout.
///
/// The optional hook is how a test keeps an injected
/// [`Clock`](ferrite_core::Clock) consistent with the waiting that
/// "happened": pass a closure that advances a `FixedClock` by the same
/// delta, and a token bucket driven by that clock refills exactly as it
/// would have in real time — deterministically, in microseconds.
pub struct RecordingSleeper {
    recorded: Mutex<Vec<Duration>>,
    on_sleep: Option<SleepHook>,
}

impl std::fmt::Debug for RecordingSleeper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSleeper")
            .field("recorded", &self.recorded.lock().ok().map(|r| r.len()))
            .field("has_hook", &self.on_sleep.is_some())
            .finish()
    }
}

impl Default for RecordingSleeper {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingSleeper {
    /// Records sleeps and does nothing else.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            on_sleep: None,
        }
    }

    /// Records sleeps and calls `hook` with each duration — typically to
    /// advance a `FixedClock`.
    #[must_use]
    pub fn with_hook(hook: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            on_sleep: Some(Box::new(hook)),
        }
    }

    /// Every duration slept, in order.
    #[must_use]
    pub fn recorded(&self) -> Vec<Duration> {
        self.recorded
            .lock()
            .expect("RecordingSleeper poisoned")
            .clone()
    }

    /// How many times it was asked to wait.
    #[must_use]
    pub fn count(&self) -> usize {
        self.recorded
            .lock()
            .expect("RecordingSleeper poisoned")
            .len()
    }
}

#[async_trait]
impl Sleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        self.recorded
            .lock()
            .expect("RecordingSleeper poisoned")
            .push(duration);
        if let Some(hook) = &self.on_sleep {
            hook(duration);
        }
    }
}

/// A directory under the OS temp dir that deletes itself on drop.
///
/// A hand-rolled 20 lines rather than a `tempfile` dependency: §7.3's
/// dependency hygiene asks whether a crate earns its place, and this one
/// would be earning it for test scaffolding alone.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a uniquely-named directory tagged with `label`.
    ///
    /// # Panics
    ///
    /// If the directory cannot be created — there is no useful way for a
    /// test to continue without one.
    #[must_use]
    pub fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ferrite-model-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("could not create a temp directory");
        Self { path }
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_recording_sleeper_records_in_order_and_returns_immediately() {
        let sleeper = RecordingSleeper::new();
        sleeper.sleep(Duration::from_millis(10)).await;
        sleeper.sleep(Duration::from_millis(20)).await;
        assert_eq!(
            sleeper.recorded(),
            vec![Duration::from_millis(10), Duration::from_millis(20)]
        );
        assert_eq!(sleeper.count(), 2);
    }

    #[tokio::test]
    async fn the_hook_lets_a_test_advance_its_own_clock_by_the_slept_amount() {
        use std::sync::Arc;

        use ferrite_core::{Clock, FixedClock};

        let clock = Arc::new(FixedClock::at_epoch());
        let for_hook = Arc::clone(&clock);
        let sleeper = RecordingSleeper::with_hook(move |d| {
            for_hook
                .advance(chrono::TimeDelta::from_std(d).expect("test durations are representable"));
        });

        sleeper.sleep(Duration::from_secs(5)).await;
        assert_eq!(
            clock.now(),
            chrono::DateTime::UNIX_EPOCH + chrono::TimeDelta::seconds(5)
        );
    }

    #[test]
    fn a_temp_dir_exists_while_it_is_held_and_is_gone_afterwards() {
        let path = {
            let dir = TempDir::new("selftest");
            assert!(dir.path().is_dir());
            dir.path().to_path_buf()
        };
        assert!(!path.exists(), "the directory must clean itself up");
    }

    #[test]
    fn two_temp_dirs_never_collide() {
        let a = TempDir::new("collide");
        let b = TempDir::new("collide");
        assert_ne!(a.path(), b.path());
    }
}
