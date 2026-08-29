//! The tracker feed the application watches for new releases.
//!
//! A tracker publishes its releases as an RSS or Atom feed. [`FeedSource`] is
//! the whole surface the application reads one through, so no rule and no
//! handler touches HTTP or XML.
//!
//! The types here are what survives that translation. A tracker feed carries
//! far more than this, and the rest is dropped, because a rule matches on a
//! filename and a listing shows a size and an age.
//!
//! [`FeedError`] carries plain data rather than a [`reqwest::Error`]. A test
//! fake then produces any failure the application handles, without a live
//! request to fail against.

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use snafu::Snafu;
use url::Url;

mod http;
mod parse;

pub mod registry;

#[cfg(any(test, feature = "fake"))]
pub mod fake;

pub use http::HttpFeedSource;

#[cfg(any(test, feature = "fake"))]
pub use fake::FakeFeeds;

/// One fetch of one tracker feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub items: Vec<FeedItem>,
}

/// One release a tracker announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    /// The tracker's own identifier for this release.
    ///
    /// A feed republishes the same release on every fetch, so this is what
    /// separates a release already seen from a new one.
    pub guid: String,

    /// The release filename, which is what a rule matches against.
    pub title: String,

    /// Where to get the torrent data.
    ///
    /// A `magnet:` scheme marks a magnet link, and any other scheme marks a
    /// torrent file to download. The scheme carries that distinction, so an
    /// item stays one URL and stores as one text column.
    pub link: Url,

    /// When the tracker announced the release. A feed that omits a date
    /// leaves this empty, so an age is never inferred from the fetch time.
    pub published: Option<DateTime<Utc>>,

    /// Total size in bytes, when the feed states one.
    pub size: Option<u64>,

    /// Current seeder count, when the feed states one. Only a torznab feed
    /// reports this, so an ordinary RSS feed leaves it empty.
    pub seeders: Option<u32>,
}

/// The credentials a fetch sends with a feed.
///
/// Most private trackers key a feed by a passkey in the URL, which needs
/// nothing here. Some hand the feed over only for a session cookie, an
/// API-key header, or HTTP basic auth, so a feed carries these beside its
/// URL.
///
/// The headers are a [`BTreeMap`] rather than a hash map, because the
/// serialized form has to be stable: two equal auth values store as equal
/// text in the database and in the configuration file.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedAuth {
    pub basic: Option<BasicAuth>,

    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// A username and password sent as HTTP basic auth.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasicAuth {
    pub username: String,
    pub password: String,
}

impl Debug for FeedAuth {
    /// Names the headers that are set, and prints none of their values.
    ///
    /// A header carries an API key as often as it carries anything else, and
    /// a derived `Debug` would put it in every line that formats a feed. The
    /// same reasoning drops the query from a feed URL before it is logged.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FeedAuth")
            .field("basic", &self.basic)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Debug for BasicAuth {
    /// Prints the username, and redacts the password.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("BasicAuth")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Why a feed fetch did not produce items.
#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
pub enum FeedError {
    /// The request never reached the tracker, or never came back. A dead
    /// host, a timeout, and a TLS failure all land here.
    #[snafu(display("the feed is unreachable: {message}"))]
    Unreachable { message: String },

    /// The tracker answered, and refused. An expired API key reaches the
    /// application as this rather than as an empty feed.
    #[snafu(display("the feed answered with status {code}"))]
    Status { code: u16 },

    /// The body arrived and is not a feed this application reads.
    #[snafu(display("the feed did not parse: {message}"))]
    Parse { message: String },
}

/// A tracker feed the application reads.
///
/// The trait is the whole contract. One implementation fetches over HTTP.
/// Another replies from a script for a test.
#[async_trait]
pub trait FeedSource: Send + Sync {
    /// Fetches `url` with `auth` and returns the releases it announces.
    ///
    /// Each call makes one request. Nothing is cached here, so a caller that
    /// polls decides its own interval.
    async fn fetch(&self, url: &Url, auth: &FeedAuth) -> Result<Feed, FeedError>;
}

/// Renders `url` as the feed it names, without the credentials it carries.
///
/// A private tracker puts the passkey in the query, so a whole feed URL in a
/// log line hands the account to anyone who reads the log. The host and the
/// path identify the feed; nothing else in the URL does.
///
/// A URL with no host renders as the path alone. A feed URL always has one,
/// but a redaction that panics on the exception is worse than one that
/// returns less.
pub(crate) fn redacted(url: &Url) -> String {
    match url.host_str() {
        Some(host) => format!("{host}{}", url.path()),
        None => url.path().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use std::collections::BTreeMap;

    use super::{BasicAuth, FeedAuth, redacted};

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    #[test]
    fn redacted_drops_the_query() {
        assert_eq!(
            redacted(&url("https://t.example/rss?passkey=abc")),
            "t.example/rss",
            "the passkey rides in the query, so the query goes"
        );
    }

    #[test]
    fn redacted_keeps_the_path_that_names_the_feed() {
        assert_eq!(
            redacted(&url("https://t.example:8443/torrents/rss#top")),
            "t.example/torrents/rss",
            "the port and the fragment name nothing a reader needs"
        );
    }

    #[test]
    fn redacted_hostless_url_is_the_path() {
        assert_eq!(
            redacted(&url("file:///var/feeds/local.xml")),
            "/var/feeds/local.xml"
        );
    }

    #[test]
    fn debug_names_the_headers_and_keeps_no_secret() {
        let auth = FeedAuth {
            basic: Some(BasicAuth {
                username: "reader".to_owned(),
                password: "hunter2".to_owned(),
            }),
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "an-api-key".to_owned())]),
        };

        let rendered = format!("{auth:?}");

        assert!(
            !rendered.contains("hunter2") && !rendered.contains("an-api-key"),
            "a formatted feed reaches the log, so no value in it does: {rendered}"
        );
        assert!(
            rendered.contains("reader") && rendered.contains("X-Api-Key"),
            "the username and the header names say which credentials are set: {rendered}"
        );
    }
}
