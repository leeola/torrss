//! A [`Downloader`] that answers from a script instead of the network.
//!
//! A test installs one reply per URL. The downloader also records the order
//! it was asked, so a caller that grabs several releases is asserted on what
//! it fetched as well as on what it did with the bytes.
//!
//! One reply per URL, rather than the queue [`crate::feed::FakeFeeds`] keeps.
//! A poll fetches one feed again and again, so a test scripts it growing. A
//! release is downloaded once.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use url::Url;

use super::{DownloadError, Downloader};
use crate::feed::FeedAuth;

/// A scripted [`Downloader`].
///
/// Every method takes `&self`, because a test holds this through an
/// `Arc<FakeDownloader>` and scripts it after the services it lives in are
/// built.
#[derive(Debug, Default)]
pub struct FakeDownloader {
    replies: Mutex<HashMap<Url, Result<Vec<u8>, DownloadError>>>,
    downloaded: Mutex<Vec<(Url, FeedAuth)>>,
}

impl FakeDownloader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers `url` with `bytes`.
    pub fn file(&self, url: &str, bytes: impl Into<Vec<u8>>) {
        self.lock_replies().insert(parse(url), Ok(bytes.into()));
    }

    /// Answers `url` with `error`.
    pub fn failing(&self, url: &str, error: DownloadError) {
        self.lock_replies().insert(parse(url), Err(error));
    }

    /// Returns every URL downloaded so far, in the order it was asked for.
    pub fn downloaded(&self) -> Vec<Url> {
        self.lock_downloaded()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Returns every download as the URL and the auth it was sent with.
    ///
    /// Use this where the credentials are the point. [`Self::downloaded`]
    /// drops them, which keeps the common assertion short.
    pub fn downloaded_auth(&self) -> Vec<(Url, FeedAuth)> {
        self.lock_downloaded().clone()
    }

    fn lock_replies(&self) -> MutexGuard<'_, HashMap<Url, Result<Vec<u8>, DownloadError>>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.replies
            .lock()
            .expect("the fake download reply lock is never poisoned")
    }

    fn lock_downloaded(&self) -> MutexGuard<'_, Vec<(Url, FeedAuth)>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.downloaded
            .lock()
            .expect("the fake download record lock is never poisoned")
    }
}

#[async_trait]
impl Downloader for FakeDownloader {
    /// Answers a URL with no scripted reply as unreachable.
    ///
    /// The attempt is recorded first, so a test still sees that the caller
    /// asked for a URL it never scripted.
    async fn download(&self, url: &Url, auth: &FeedAuth) -> Result<Vec<u8>, DownloadError> {
        self.lock_downloaded().push((url.clone(), auth.clone()));

        // The reply is cloned rather than removed, so a caller that retries
        // keeps seeing the state a test left it in instead of running dry.
        self.lock_replies().get(url).cloned().unwrap_or_else(|| {
            Err(DownloadError::Unreachable {
                message: format!("no reply is scripted for {url}"),
            })
        })
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

    use super::{DownloadError, Downloader, FakeDownloader, FeedAuth, parse};

    const TORRENT: &str = "https://tracker.invalid/get/1.torrent";
    const OTHER: &str = "https://other.invalid/get/2.torrent";

    #[tokio::test]
    async fn scripted_bytes_return() {
        let downloads = FakeDownloader::new();
        downloads.file(TORRENT, b"d8:announce4:teste");

        assert_eq!(
            downloads
                .download(&parse(TORRENT), &FeedAuth::default())
                .await,
            Ok(b"d8:announce4:teste".to_vec())
        );
    }

    #[tokio::test]
    async fn scripted_error_returns() {
        let downloads = FakeDownloader::new();
        downloads.failing(TORRENT, DownloadError::Status { code: 403 });

        assert_eq!(
            downloads
                .download(&parse(TORRENT), &FeedAuth::default())
                .await,
            Err(DownloadError::Status { code: 403 })
        );
    }

    #[tokio::test]
    async fn unknown_url_is_unreachable() {
        let downloads = FakeDownloader::new();
        downloads.file(TORRENT, "bytes");

        assert_eq!(
            downloads
                .download(&parse(OTHER), &FeedAuth::default())
                .await,
            Err(DownloadError::Unreachable {
                message: format!("no reply is scripted for {OTHER}"),
            })
        );
    }

    #[tokio::test]
    async fn downloaded_records_order() {
        let downloads = FakeDownloader::new();
        downloads.file(TORRENT, "bytes");
        downloads.failing(OTHER, DownloadError::Status { code: 404 });

        let _ = downloads
            .download(&parse(OTHER), &FeedAuth::default())
            .await;
        let _ = downloads
            .download(&parse(TORRENT), &FeedAuth::default())
            .await;
        let _ = downloads
            .download(&parse(OTHER), &FeedAuth::default())
            .await;

        assert_eq!(
            downloads.downloaded(),
            vec![parse(OTHER), parse(TORRENT), parse(OTHER)],
            "an unscripted or failing URL still records the attempt"
        );
    }

    #[tokio::test]
    async fn downloaded_auth_records_the_pair() {
        let downloads = FakeDownloader::new();
        downloads.file(TORRENT, "bytes");

        let auth = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("Cookie".to_owned(), "session=abc".to_owned())]),
        };
        let _ = downloads.download(&parse(TORRENT), &auth).await;

        assert_eq!(
            downloads.downloaded_auth(),
            vec![(parse(TORRENT), auth)],
            "the credentials reach the downloader, not only the URL"
        );
    }
}
