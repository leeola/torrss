//! The [`FeedSource`] that reads a tracker feed over HTTP.

use async_trait::async_trait;
use reqwest::Client;
use url::Url;

use super::{Feed, FeedError, FeedSource, parse};

/// A tracker feed read over HTTP.
///
/// One value serves every feed. The client it holds pools connections and
/// is cheap to clone, so a caller that polls dozens of trackers builds this
/// once at startup rather than per fetch.
pub struct HttpFeedSource {
    client: Client,
}

impl HttpFeedSource {
    /// Returns a source that identifies itself as `torrss`.
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

impl Default for HttpFeedSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FeedSource for HttpFeedSource {
    async fn fetch(&self, url: &Url) -> Result<Feed, FeedError> {
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(unreachable)?
            .error_for_status()
            .map_err(status)?;

        let body = response.bytes().await.map_err(unreachable)?;

        parse::parse(&body)
    }
}

fn unreachable(error: reqwest::Error) -> FeedError {
    FeedError::Unreachable {
        message: error.to_string(),
    }
}

/// Turns a rejected status into [`FeedError::Status`].
///
/// An error from `error_for_status` always carries a status. The fallback
/// keeps that assumption out of the panic path, because a wrong guess here
/// takes down a poll loop over one odd response.
fn status(error: reqwest::Error) -> FeedError {
    match error.status() {
        Some(code) => FeedError::Status {
            code: code.as_u16(),
        },
        None => unreachable(error),
    }
}
