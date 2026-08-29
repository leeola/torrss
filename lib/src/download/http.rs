//! The [`Downloader`] that fetches a torrent file over HTTP.

use async_trait::async_trait;
use reqwest::Client;
use url::Url;

use super::{DownloadError, Downloader};
use crate::feed::FeedAuth;
use crate::feed::http::authorize;

/// A torrent file fetched over HTTP.
///
/// One value serves every tracker. The client it holds pools connections and
/// is cheap to clone, so a caller that grabs from several trackers builds
/// this once at startup rather than per download.
pub struct HttpDownloader {
    client: Client,
}

impl HttpDownloader {
    /// Returns a downloader that identifies itself as `torrss`.
    ///
    /// Trackers rate-limit and sometimes block by user agent, so the name is
    /// fixed rather than left as the library default.
    ///
    /// # Panics
    ///
    /// Panics when the TLS backend fails to start, which happens at process
    /// start or not at all.
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("torrss")
                .build()
                .expect("the rustls backend always builds"),
        }
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn download(&self, url: &Url, auth: &FeedAuth) -> Result<Vec<u8>, DownloadError> {
        let response = authorize(self.client.get(url.clone()), auth)
            .send()
            .await
            .map_err(unreachable)?
            .error_for_status()
            .map_err(status)?;

        Ok(response.bytes().await.map_err(unreachable)?.to_vec())
    }
}

fn unreachable(error: reqwest::Error) -> DownloadError {
    DownloadError::Unreachable {
        message: error.to_string(),
    }
}

/// Turns a rejected status into [`DownloadError::Status`].
///
/// An error from `error_for_status` always carries a status. The fallback
/// keeps that assumption out of the panic path, because a wrong guess here
/// fails a grab over one odd response.
fn status(error: reqwest::Error) -> DownloadError {
    match error.status() {
        Some(code) => DownloadError::Status {
            code: code.as_u16(),
        },
        None => unreachable(error),
    }
}
