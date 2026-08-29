//! The span every request runs inside.
//!
//! One line per request, written when the span closes, carrying the method,
//! the path, the status, and how long the request was busy. That line is the
//! access log, and every line a handler writes nests under it.

use topcoat::context::Cx;
use topcoat::router::response::{IntoResponse, Response};
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, request};
use topcoat::{Error, Result};
use tracing::field::Empty;
use tracing::{Instrument, Span, error, info_span};

/// Wraps every request in a `request` span.
pub(super) struct RequestSpan;

impl Layer for RequestSpan {
    /// Wraps every request rather than one path's.
    ///
    /// A request that matches no route reaches this layer as the error from
    /// [`Next::run`], and it is still a request worth a line.
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        let span = info_span!(
            "request",
            http.method = %request::method(cx),
            http.path = %request::uri(cx).path(),
            http.status = Empty,
        );

        Box::pin(
            async move {
                let response = match next.run(cx, body).await {
                    Ok(response) => response,
                    Err(error) => failed(cx, error)?,
                };

                // Recorded before the span closes, so the close line carries
                // the status rather than leaving it empty.
                Span::current().record("http.status", response.status().as_u16());

                Ok(response)
            }
            .instrument(span),
        )
    }
}

/// Renders the error the chain returned, and reports a server fault.
///
/// The conversion here is the one the router applies after the last layer, so
/// the client sees the same response. Doing it inside the span is what makes
/// the status known to the line that closes it.
///
/// Only a 5xx is logged. topcoat renders a 500 and reports the cause nowhere,
/// so without this the one line an operator needs never appears. A 4xx is the
/// client's business rather than a fault to investigate.
fn failed(cx: &Cx, error: Error) -> Result<Response> {
    let text = error.to_string();
    let response = error.into_response(cx)?;

    if response.status().is_server_error() {
        error!(error = %text, "request failed");
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex, MutexGuard};

    use topcoat::context::Cx;
    use topcoat::router::request::Request;
    use topcoat::router::response::IntoResponse;
    use topcoat::router::{Body, Method, RouteFn, RouteFuture, Router};
    use tracing::subscriber::DefaultGuard;
    use tracing_subscriber::fmt::format::FmtSpan;

    use super::RequestSpan;

    /// A writer a test reads back, shared with the subscriber writing to it.
    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Vec::new())))
        }

        /// Returns everything written so far.
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.lock()).into_owned()
        }

        fn lock(&self) -> MutexGuard<'_, Vec<u8>> {
            // Nothing panics while the guard is held, so the lock never poisons.
            self.0.lock().expect("the sink lock is never poisoned")
        }
    }

    impl Write for Sink {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.lock().extend_from_slice(buffer);

            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Installs a subscriber for this thread and hands back what it writes.
    ///
    /// `set_default` is thread-local and `#[tokio::test]` runs on the current
    /// thread, so the sink sees the layer's lines and no other test's. Hold
    /// the guard for as long as the assertions need the subscriber.
    fn capture() -> (Sink, DefaultGuard) {
        let sink = Sink::new();

        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let sink = sink.clone();
                move || sink.clone()
            })
            .with_ansi(false)
            .without_time()
            .with_span_events(FmtSpan::CLOSE)
            .finish();

        (sink, tracing::subscriber::set_default(subscriber))
    }

    fn ok(_cx: &Cx, _body: Body) -> RouteFuture<'_> {
        Box::pin(async move { "ok".into_response(_cx) })
    }

    fn fail(_cx: &Cx, _body: Body) -> RouteFuture<'_> {
        Box::pin(async move { Err(io::Error::other("boom").into()) })
    }

    fn router() -> Router {
        Router::builder()
            .layer(RequestSpan)
            .route(RouteFn::new(Method::GET, "/ok", ok))
            .route(RouteFn::new(Method::GET, "/fail", fail))
            .build()
    }

    async fn get(path: &str) -> String {
        let (sink, _guard) = capture();

        router()
            .handle(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("a valid request"),
            )
            .await;

        sink.text()
    }

    #[tokio::test]
    async fn ok_request_closes_with_status() {
        let logged = get("/ok").await;

        assert!(
            logged.contains("http.method=GET")
                && logged.contains("http.path=/ok")
                && logged.contains("http.status=200"),
            "the close line is the access log, got: {logged}"
        );
    }

    #[tokio::test]
    async fn missing_route_closes_with_404() {
        let logged = get("/nothing-here").await;

        assert!(
            logged.contains("http.status=404"),
            "a request that matched nothing still closes a span, got: {logged}"
        );
        assert!(
            !logged.contains("request failed"),
            "a 404 is the client's business, not a fault, got: {logged}"
        );
    }

    #[tokio::test]
    async fn handler_error_logs_the_error_text() {
        let logged = get("/fail").await;

        assert!(
            logged.contains("http.status=500"),
            "the converted error carries its status into the span, got: {logged}"
        );
        assert!(
            logged.contains("error=boom"),
            "topcoat reports the cause nowhere, so the layer does, got: {logged}"
        );
    }
}
