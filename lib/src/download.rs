//! Fetching the torrent file a feed item points at.
//!
//! A tracker's `.torrent` link carries a passkey and needs the same cookies
//! as the feed it came from. Handing that URL to qBittorrent asks the client
//! to authenticate as this application, which it cannot do. So the
//! application fetches the file itself and hands the client the bytes.
//!
//! A magnet link needs none of this and never reaches here. Only a link the
//! client cannot resolve on its own is downloaded.
//!
//! [`DownloadError`] carries plain data rather than a [`reqwest::Error`]. A
//! test fake then produces any failure the application handles, without a
//! live request to fail against.

use async_trait::async_trait;
use snafu::Snafu;
use url::Url;

#[cfg(any(test, feature = "fake"))]
pub mod fake;

#[cfg(any(test, feature = "fake"))]
pub use fake::FakeDownloader;

/// Why a download did not produce a file.
#[derive(Debug, Clone, PartialEq, Eq, Snafu)]
pub enum DownloadError {
    /// The request never reached the tracker, or never came back. A dead
    /// host, a timeout, and a TLS failure all land here.
    #[snafu(display("the download is unreachable: {message}"))]
    Unreachable { message: String },

    /// The tracker answered, and refused. An expired passkey reaches the
    /// application as this rather than as an empty file.
    #[snafu(display("the download answered with status {code}"))]
    Status { code: u16 },
}

/// A source of torrent files the application downloads.
///
/// The trait is the whole contract. One implementation fetches over HTTP.
/// Another replies from a script for a test.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Fetches `url` and returns the bytes it answered with.
    ///
    /// The bytes are not checked for being a torrent file. The client
    /// rejects a body it cannot read, and that rejection says more about the
    /// tracker than a parse here would.
    async fn download(&self, url: &Url) -> Result<Vec<u8>, DownloadError>;
}
