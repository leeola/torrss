//! Everything the application talks to the outside world through.
//!
//! A handler needs the feed source, the clock, and the database. Carrying
//! them as one value means the app context holds a single entry and a new
//! dependency reaches every handler without touching a signature.

use std::sync::Arc;

use sqlx::SqlitePool;

use crate::clock::Clock;
use crate::feed::FeedSource;

/// The outside world, as the application sees it.
///
/// Cloning is cheap and shares every dependency, which is what makes the
/// poll task and the router work against the same feed source and the same
/// database rather than against copies.
#[derive(Clone)]
pub struct Services {
    pub feeds: Arc<dyn FeedSource>,
    pub clock: Arc<dyn Clock>,
    pub db: SqlitePool,
}

#[cfg(any(test, feature = "fake"))]
pub use fake::Fakes;

#[cfg(any(test, feature = "fake"))]
mod fake {
    use std::sync::Arc;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::Services;
    use crate::clock::FakeClock;
    use crate::feed::FakeFeeds;

    /// The same fakes [`Services`] holds, at their concrete types.
    ///
    /// [`Services`] erases each dependency to a trait object, which hides
    /// the methods a test scripts with. These handles point at the very
    /// same values, so a reply installed here is the reply the code under
    /// test reads.
    pub struct Fakes {
        pub feeds: Arc<FakeFeeds>,
        pub clock: Arc<FakeClock>,
    }

    impl Services {
        /// Returns a set of services backed entirely by fakes.
        ///
        /// The clock starts at 2025-01-01T00:00:00Z. Anchor on
        /// `clock.now()` rather than repeating that instant, so a later
        /// change of start does not break a test that does not care.
        ///
        /// The database is a fresh empty in-memory one, so a caller that
        /// needs a schema runs its own migration.
        ///
        /// # Panics
        ///
        /// Panics when the in-memory database does not open, which means
        /// the sqlite driver itself is broken.
        pub async fn fake() -> (Self, Fakes) {
            let feeds = Arc::new(FakeFeeds::new());
            let clock = Arc::new(FakeClock::at(start()));

            // Each field coerces the concrete fake to its trait object here.
            // `Arc::clone` names the concrete type and blocks that, so the
            // method form is what keeps both structs pointed at one value.
            let services = Self {
                feeds: feeds.clone(),
                clock: clock.clone(),
                // One connection only. Every connection to `sqlite::memory:`
                // opens a database of its own, so a second one sees an empty
                // schema instead of what the first wrote.
                db: SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("an in-memory sqlite database always opens"),
            };

            (services, Fakes { feeds, clock })
        }
    }

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .expect("the fake start instant is unambiguous")
    }
}

#[cfg(test)]
mod tests {
    use super::Services;
    use crate::feed::{Feed, fake};

    #[tokio::test]
    async fn scripting_a_fake_reaches_the_services() {
        let (services, fakes) = Services::fake().await;
        let url = "https://tracker.invalid/rss";
        fakes
            .feeds
            .feed(url, vec![fake::item("Some.Release.1080p")]);

        assert_eq!(
            services
                .feeds
                .fetch(&url.parse().expect("the test URL parses"))
                .await,
            Ok(Feed {
                items: vec![fake::item("Some.Release.1080p")]
            }),
            "both structs hold the same fake"
        );
    }
}
