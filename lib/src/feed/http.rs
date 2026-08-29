//! The [`FeedSource`] that reads a tracker feed over HTTP.

use async_trait::async_trait;
use reqwest::Client;
use tracing::{debug, instrument};
use url::Url;

use super::{Feed, FeedAuth, FeedError, FeedSource, parse, redacted};

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
    #[instrument(name = "fetch_feed", level = "debug", skip_all, fields(feed.url = %redacted(url)))]
    async fn fetch(&self, url: &Url, auth: &FeedAuth) -> Result<Feed, FeedError> {
        let mut request = self.client.get(url.clone());

        // A malformed name or value defers to `send`, which reports it as the
        // same `reqwest::Error` any other request failure takes, so nothing
        // is validated here.
        for (name, value) in &auth.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        if let Some(basic) = &auth.basic {
            request = request.basic_auth(&basic.username, Some(&basic.password));
        }

        let response = request
            .send()
            .await
            .map_err(unreachable)?
            .error_for_status()
            .map_err(status)?;

        // Named `code` rather than `status`, which is the fallible mapping
        // three lines above and would shadow it for the rest of the body.
        let code = response.status().as_u16();
        let body = response.bytes().await.map_err(unreachable)?;

        debug!(http.status = code, bytes = body.len(), "fetched");

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
