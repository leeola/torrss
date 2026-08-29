//! A [`FeedSource`] that answers from a script instead of the network.
//!
//! A test installs one reply, or a sequence of them, per URL. The source
//! also records the order it was asked, so a caller that polls several feeds
//! is asserted on what it fetched as well as on what it did with the result.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use url::Url;

use super::{Feed, FeedAuth, FeedError, FeedItem, FeedSource};

/// The host every [`item`] link points at.
///
/// The `.invalid` TLD is reserved and never resolves, so a test that reaches
/// the network by mistake fails rather than fetching something real.
const FAKE_HOST: &str = "https://fake.invalid";

/// A scripted [`FeedSource`].
///
/// Every method takes `&self`, because a test holds this through an
/// `Arc<FakeFeeds>` and scripts it after the services it lives in are built.
#[derive(Debug, Default)]
pub struct FakeFeeds {
    replies: Mutex<HashMap<Url, VecDeque<Result<Feed, FeedError>>>>,
    fetched: Mutex<Vec<(Url, FeedAuth)>>,
}

impl FakeFeeds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `url` with a feed of `items`.
    pub fn feed(&self, url: &str, items: Vec<FeedItem>) {
        self.replies(url, [Ok(Feed { items })]);
    }

    /// Answers `url` with `error`.
    pub fn failing(&self, url: &str, error: FeedError) {
        self.replies(url, [Err(error)]);
    }

    /// Answers `url` with each reply in turn, and repeats the last one.
    pub fn replies(&self, url: &str, replies: impl IntoIterator<Item = Result<Feed, FeedError>>) {
        self.lock_replies()
            .insert(parse(url), replies.into_iter().collect());
    }

    /// Adds `item` to the last successful reply scripted for `url`.
    ///
    /// A URL with no successful reply gets a fresh one-item feed, so a test
    /// that grows a feed over several fetches never scripts an empty one
    /// first.
    pub fn push(&self, url: &str, item: FeedItem) {
        let mut replies = self.lock_replies();
        let queue = replies.entry(parse(url)).or_default();

        match queue.iter_mut().rev().find_map(|reply| reply.as_mut().ok()) {
            Some(feed) => feed.items.push(item),
            None => queue.push_back(Ok(Feed { items: vec![item] })),
        }
    }

    /// Returns every URL fetched so far, in the order it was fetched.
    pub fn fetched(&self) -> Vec<Url> {
        self.lock_fetched()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Returns every fetch as the URL and the auth it was sent with.
    ///
    /// Use this where the credentials are the point. [`Self::fetched`] drops
    /// them, which keeps the common assertion short.
    pub fn fetched_auth(&self) -> Vec<(Url, FeedAuth)> {
        self.lock_fetched().clone()
    }

    fn lock_replies(&self) -> MutexGuard<'_, HashMap<Url, VecDeque<Result<Feed, FeedError>>>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.replies
            .lock()
            .expect("the fake feed reply lock is never poisoned")
    }

    fn lock_fetched(&self) -> MutexGuard<'_, Vec<(Url, FeedAuth)>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.fetched
            .lock()
            .expect("the fake feed fetch lock is never poisoned")
    }
}

#[async_trait]
impl FeedSource for FakeFeeds {
    async fn fetch(&self, url: &Url, auth: &FeedAuth) -> Result<Feed, FeedError> {
        self.lock_fetched().push((url.clone(), auth.clone()));

        let mut replies = self.lock_replies();
        let Some(queue) = replies.get_mut(url) else {
            return Err(FeedError::Unreachable {
                message: format!("no reply is scripted for {url}"),
            });
        };

        // The final reply is cloned rather than popped, so a caller that polls
        // keeps seeing the state a test left it in instead of running dry.
        match queue.len() {
            0 => Err(FeedError::Unreachable {
                message: format!("every scripted reply for {url} is used up"),
            }),
            1 => queue[0].clone(),
            _ => queue
                .pop_front()
                .unwrap_or_else(|| unreachable!("the queue holds more than one reply")),
        }
    }
}

/// Returns a feed item for a release named `title`.
///
/// The guid, the title, and the link all come from `title`, so a test names
/// only the release and still gets a whole item. Chain the setters below to
/// fill in what the test actually asserts on.
pub fn item(title: &str) -> FeedItem {
    FeedItem {
        guid: title.to_owned(),
        title: title.to_owned(),
        link: parse(&format!("{FAKE_HOST}/{title}.torrent")),
        published: None,
        size: None,
        seeders: None,
    }
}

impl FeedItem {
    pub fn size(mut self, bytes: u64) -> Self {
        self.size = Some(bytes);
        self
    }

    pub fn seeders(mut self, count: u32) -> Self {
        self.seeders = Some(count);
        self
    }

    /// Replaces the link with the magnet URL `magnet`.
    pub fn magnet(mut self, magnet: &str) -> Self {
        self.link = parse(magnet);
        self
    }

    pub fn published(mut self, at: DateTime<Utc>) -> Self {
        self.published = Some(at);
        self
    }
}

/// Parses a URL a test wrote, panicking when it does not parse.
///
/// A malformed URL here is a bug in the test rather than a condition the
/// application handles, so it fails loudly instead of returning a result the
/// test has to unwrap.
fn parse(url: &str) -> Url {
    Url::parse(url).unwrap_or_else(|error| panic!("{url} is not a valid test URL: {error}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FakeFeeds, FeedAuth, FeedSource, item, parse};
    use crate::feed::{Feed, FeedError};

    const FEED: &str = "https://tracker.invalid/rss";
    const OTHER: &str = "https://other.invalid/rss";

    #[tokio::test]
    async fn last_reply_repeats() {
        let feeds = FakeFeeds::new();
        feeds.replies(
            FEED,
            [
                Ok(Feed {
                    items: vec![item("First.Release.1080p")],
                }),
                Ok(Feed {
                    items: vec![item("Second.Release.1080p")],
                }),
            ],
        );

        let url = parse(FEED);
        let first = feeds.fetch(&url, &FeedAuth::default()).await;
        let second = feeds.fetch(&url, &FeedAuth::default()).await;
        let third = feeds.fetch(&url, &FeedAuth::default()).await;

        assert_eq!(
            first,
            Ok(Feed {
                items: vec![item("First.Release.1080p")]
            }),
            "first fetch"
        );
        assert_eq!(second, third, "the last reply repeats");
        assert_eq!(
            third,
            Ok(Feed {
                items: vec![item("Second.Release.1080p")]
            }),
            "third fetch"
        );
    }

    #[tokio::test]
    async fn unknown_url_is_unreachable() {
        let feeds = FakeFeeds::new();
        feeds.feed(FEED, vec![item("Known.Release.1080p")]);

        assert_eq!(
            feeds.fetch(&parse(OTHER), &FeedAuth::default()).await,
            Err(FeedError::Unreachable {
                message: format!("no reply is scripted for {OTHER}"),
            })
        );
    }

    #[tokio::test]
    async fn push_extends_next_reply() {
        let feeds = FakeFeeds::new();
        feeds.push(FEED, item("Some Show S01E01 1080p").size(2048));
        feeds.push(FEED, item("Some Show S01E02 1080p").seeders(9));

        assert_eq!(
            feeds.fetch(&parse(FEED), &FeedAuth::default()).await,
            Ok(Feed {
                items: vec![
                    item("Some Show S01E01 1080p").size(2048),
                    item("Some Show S01E02 1080p").seeders(9),
                ]
            })
        );
    }

    #[tokio::test]
    async fn fetched_records_order() {
        let feeds = FakeFeeds::new();
        feeds.feed(FEED, vec![item("Some.Release.1080p")]);
        feeds.failing(OTHER, FeedError::Status { code: 503 });

        let _ = feeds.fetch(&parse(OTHER), &FeedAuth::default()).await;
        let _ = feeds.fetch(&parse(FEED), &FeedAuth::default()).await;
        let _ = feeds.fetch(&parse(OTHER), &FeedAuth::default()).await;

        assert_eq!(
            feeds.fetched(),
            vec![parse(OTHER), parse(FEED), parse(OTHER)]
        );
    }

    #[tokio::test]
    async fn fetched_auth_records_the_pair() {
        let feeds = FakeFeeds::new();
        feeds.feed(FEED, vec![item("Some.Release.1080p")]);

        let auth = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]),
        };
        let _ = feeds.fetch(&parse(FEED), &auth).await;

        assert_eq!(
            feeds.fetched_auth(),
            vec![(parse(FEED), auth)],
            "the credentials reach the source, not only the URL"
        );
    }
}
