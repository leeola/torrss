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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use snafu::Snafu;
use url::Url;

use crate::torrent::TorrentSource;

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

    /// Where to get the torrent data, ready to hand to a torrent client.
    pub link: TorrentSource,

    /// When the tracker announced the release. A feed that omits a date
    /// leaves this empty, so an age is never inferred from the fetch time.
    pub published: Option<DateTime<Utc>>,

    /// Total size in bytes, when the feed states one.
    pub size: Option<u64>,

    /// Current seeder count, when the feed states one. Only a torznab feed
    /// reports this, so an ordinary RSS feed leaves it empty.
    pub seeders: Option<u32>,
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
    /// Fetches `url` and returns the releases it announces.
    ///
    /// Each call makes one request. Nothing is cached here, so a caller that
    /// polls decides its own interval.
    async fn fetch(&self, url: &Url) -> Result<Feed, FeedError>;
}
