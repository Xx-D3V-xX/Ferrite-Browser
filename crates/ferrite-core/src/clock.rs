//! Injectable time.
//!
//! R8 forbids wall-clock reads in assertions: a test that calls
//! `Utc::now()` is a test whose outcome depends on when it runs. Everything
//! in the workspace that needs the time takes a `&dyn Clock` (or a generic
//! `C: Clock`) instead, so production wires in [`SystemClock`] and tests wire
//! in `FixedClock`, which only moves when a test moves it.
//!
//! `FixedClock` is gated behind `cfg(test)` in this crate and the `test-util`
//! feature for downstream crates, so no test-only code reaches a default
//! build (R9) — it is deliberately not linked here, because in a default
//! build there is no item to link to, and `cargo doc` failing on that link is
//! the proof.

use chrono::{DateTime, Utc};

/// A source of the current time.
///
/// Object-safe on purpose: a struct that needs a clock stores
/// `Arc<dyn Clock>` rather than being generic over one, which keeps the clock
/// out of the signatures of everything that transitively holds it.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The current instant, in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// The real clock. The only thing in the workspace that may read wall time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that returns exactly what a test set it to.
///
/// Advancing is explicit and manual, so an elapsed-time assertion measures
/// the code under test rather than the machine it runs on. Interior
/// mutability (rather than `&mut self`) is what lets it sit behind a shared
/// `Arc<dyn Clock>` while a test still drives it.
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug)]
pub struct FixedClock {
    now: std::sync::Mutex<DateTime<Utc>>,
}

#[cfg(any(test, feature = "test-util"))]
impl FixedClock {
    /// A clock stopped at `start`.
    #[must_use]
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: std::sync::Mutex::new(start),
        }
    }

    /// A clock stopped at the Unix epoch — the default starting point for a
    /// test that does not care about the absolute instant, only about
    /// ordering and deltas.
    #[must_use]
    pub fn at_epoch() -> Self {
        Self::new(DateTime::UNIX_EPOCH)
    }

    /// Moves the clock forward (or backward, for a negative delta).
    ///
    /// # Panics
    ///
    /// If the resulting instant is outside `chrono`'s representable range, or
    /// if another thread panicked while holding the clock.
    pub fn advance(&self, by: chrono::TimeDelta) {
        let mut now = self.now.lock().expect("FixedClock poisoned");
        *now = now
            .checked_add_signed(by)
            .expect("FixedClock advanced out of the representable range");
    }

    /// Jumps the clock to an exact instant.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the clock.
    pub fn set(&self, to: DateTime<Utc>) {
        *self.now.lock().expect("FixedClock poisoned") = to;
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Default for FixedClock {
    /// Equivalent to [`FixedClock::at_epoch`].
    fn default() -> Self {
        Self::at_epoch()
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("FixedClock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use super::*;

    /// Stands in for production code that takes a clock without caring which.
    fn elapsed_between_two_reads(clock: &dyn Clock, work: impl FnOnce()) -> TimeDelta {
        let before = clock.now();
        work();
        clock.now() - before
    }

    #[test]
    fn a_fixed_clock_does_not_move_on_its_own() {
        let clock = FixedClock::at_epoch();
        assert_eq!(clock.now(), DateTime::UNIX_EPOCH);
        assert_eq!(
            elapsed_between_two_reads(&clock, || {}),
            TimeDelta::zero(),
            "no wall time may leak into an elapsed-time measurement"
        );
    }

    #[test]
    fn a_fixed_clock_moves_only_when_a_test_moves_it() {
        let clock = FixedClock::at_epoch();
        let elapsed = elapsed_between_two_reads(&clock, || {
            clock.advance(TimeDelta::seconds(30));
        });
        assert_eq!(elapsed, TimeDelta::seconds(30));

        clock.advance(TimeDelta::milliseconds(-500));
        assert_eq!(
            clock.now(),
            DateTime::UNIX_EPOCH + TimeDelta::seconds(29) + TimeDelta::milliseconds(500)
        );

        let pinned = DateTime::UNIX_EPOCH + TimeDelta::days(1);
        clock.set(pinned);
        assert_eq!(clock.now(), pinned);
    }

    #[test]
    fn a_fixed_clock_is_shareable_behind_a_trait_object() {
        let clock = Arc::new(FixedClock::at_epoch());
        let as_trait: Arc<dyn Clock> = clock.clone();
        clock.advance(TimeDelta::seconds(5));
        assert_eq!(
            as_trait.now(),
            DateTime::UNIX_EPOCH + TimeDelta::seconds(5),
            "a holder of the shared clock sees what the test did to it"
        );
    }

    #[test]
    fn the_default_fixed_clock_starts_at_the_epoch() {
        assert_eq!(FixedClock::default().now(), FixedClock::at_epoch().now());
    }

    #[test]
    fn the_system_clock_reads_wall_time_without_the_test_asserting_on_it() {
        // Deliberately asserts no instant and no ordering: the only claim is
        // that SystemClock satisfies the trait and can be used where one is
        // wanted. Any assertion about the value here would be the R8
        // violation this module exists to prevent.
        let clock: &dyn Clock = &SystemClock;
        let _: DateTime<Utc> = clock.now();
    }
}
