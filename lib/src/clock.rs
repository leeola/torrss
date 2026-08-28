//! The passage of time, behind a trait so a test controls it.
//!
//! Release age, poll intervals, and the pause between two polling passes
//! all read the clock. Reading it through [`Clock`] lets a test drive a
//! whole polling run in no real time, and assert the exact intervals the
//! run slept for.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// The source of the current time and of every delay.
///
/// [`Duration`] is the standard library type rather than a chrono one,
/// because the real implementation hands it straight to the tokio timer.
#[async_trait]
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;

    /// Waits for `duration` to pass.
    async fn sleep(&self, duration: Duration);
}

/// The real clock, reading wall time and the tokio timer.
pub struct SystemClock;

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[cfg(any(test, feature = "fake"))]
pub use fake::FakeClock;

#[cfg(any(test, feature = "fake"))]
mod fake {
    use std::sync::{Mutex, MutexGuard};
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::{DateTime, TimeDelta, Utc};

    use super::Clock;

    /// A clock a test moves by hand.
    ///
    /// Sleeping returns at once and moves the stored time forward instead,
    /// so a loop under test observes hours passing within a single test run.
    /// Every duration slept for is kept, which is how a test asserts the
    /// schedule a loop actually kept rather than only its final state.
    pub struct FakeClock {
        now: Mutex<DateTime<Utc>>,
        slept: Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        /// Returns a clock that starts at `now`.
        pub fn at(now: DateTime<Utc>) -> Self {
            Self {
                now: Mutex::new(now),
                slept: Mutex::new(Vec::new()),
            }
        }

        /// Moves the clock forward without recording a sleep.
        ///
        /// Use this to age what a test already stored. A sleep the code
        /// under test makes shows up in [`Self::slept`]. A jump made here
        /// does not.
        pub fn advance(&self, duration: Duration) {
            let mut now = self.lock_now();
            *now = advanced(*now, duration);
        }

        /// Returns every duration slept for, in the order it was slept.
        pub fn slept(&self) -> Vec<Duration> {
            self.lock_slept().clone()
        }

        fn lock_now(&self) -> MutexGuard<'_, DateTime<Utc>> {
            // Nothing panics while the guard is held, so the lock never poisons.
            self.now
                .lock()
                .expect("the fake clock time lock is never poisoned")
        }

        fn lock_slept(&self) -> MutexGuard<'_, Vec<Duration>> {
            // Nothing panics while the guard is held, so the lock never poisons.
            self.slept
                .lock()
                .expect("the fake clock sleep lock is never poisoned")
        }
    }

    #[async_trait]
    impl Clock for FakeClock {
        fn now(&self) -> DateTime<Utc> {
            *self.lock_now()
        }

        async fn sleep(&self, duration: Duration) {
            self.lock_slept().push(duration);
            self.advance(duration);
        }
    }

    /// Returns `now` moved forward by `duration`.
    ///
    /// Both steps fail only for a span of hundreds of millions of years,
    /// which no interval a test scripts reaches, so each names the
    /// condition rather than returning a result a caller must handle.
    fn advanced(now: DateTime<Utc>, duration: Duration) -> DateTime<Utc> {
        let delta = TimeDelta::from_std(duration)
            .expect("a test never scripts a duration chrono cannot measure");

        now.checked_add_signed(delta)
            .expect("a test never advances the clock past the end of time")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{DateTime, TimeZone, Utc};

    use super::{Clock, FakeClock};

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, 4, 9, 15, 0)
            .single()
            .expect("the start time is unambiguous")
    }

    #[tokio::test]
    async fn sleep_advances_now() {
        let clock = FakeClock::at(start());
        clock.sleep(Duration::from_secs(900)).await;

        assert_eq!(
            clock.now(),
            Utc.with_ymd_and_hms(2025, 3, 4, 9, 30, 0)
                .single()
                .expect("the expected time is unambiguous")
        );
    }

    #[tokio::test]
    async fn slept_records_order() {
        let clock = FakeClock::at(start());
        clock.sleep(Duration::from_secs(30)).await;
        clock.advance(Duration::from_secs(5));
        clock.sleep(Duration::from_secs(900)).await;
        clock.sleep(Duration::from_secs(60)).await;

        assert_eq!(
            clock.slept(),
            vec![
                Duration::from_secs(30),
                Duration::from_secs(900),
                Duration::from_secs(60),
            ],
            "advance never counts as a sleep"
        );
    }
}
