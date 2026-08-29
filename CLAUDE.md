# torrss

## Tracing

- Open one span per unit of work: `request`, `check_feed`, `fetch_feed`. Name a
  span as a snake_case noun or verb phrase. Give it only the fields that find
  the work again.
- Give a field a dotted namespace when it belongs to a domain the OpenTelemetry
  semantic conventions name: `http.method`, `http.path`, `http.status`,
  `feed.id`, `feed.url`. A field that belongs to no domain keeps a bare name,
  such as `error` or `bytes`.
- Report an outcome as an event with a short, static, lowercase message. Put
  every value in a field. Never format a value into the message.
- Put an error in a field named `error`, rendered with `%`.
- Pick the level by what the line records:
  - `error`: work that failed and an operator has to see, such as a 5xx or a
    panic.
  - `warn`: a failure the next pass retries.
  - `info`: one line per unit of work, and the process lifecycle.
  - `debug`: internal detail.
  - `trace`: per-item detail.
- Never hold a `Span::enter` guard across an `.await`. Use `#[instrument]` or
  `.instrument(span)` instead.
- Give a spawned task its own span. `tokio::spawn` does not carry the caller's
  span.
- Never log a whole feed URL. A private tracker puts its passkey in the query.
  Use `feed::redacted`.
- To read spans, set `TORRSS_LOG=info,torrss=debug` and `TORRSS_LOG_FORMAT=json`.
