//! Routes under `/api`, which answer a script rather than a browser.
//!
//! Everything here returns JSON and never redirects, so a caller reads a
//! result instead of following a page. The pages elsewhere in this module do
//! the opposite.
//!
//! Nothing here authenticates, which is true of the admin pages too. The
//! listener defaults to a loopback address for that reason.

use std::sync::Arc;

use serde::Serialize;
use topcoat::{Result, context::Cx, context::app_context, router::content::Json, router::route};

use crate::feed::registry::{self, FeedEntry, FeedRegistry};
use crate::services::Services;

/// What one pass over every feed produced.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct CheckReport {
    feeds: Vec<FeedReport>,
}

/// One feed's place in the report.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct FeedReport {
    id: String,
    name: String,

    /// Flattened, so a reader gets the outcome and its fields side by side
    /// rather than nested under a variant name.
    #[serde(flatten)]
    outcome: Outcome,
}

/// How one feed's check went.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
enum Outcome {
    Ok {
        items: usize,
        added: usize,
    },
    Failed {
        error: String,
    },

    /// A feed registered while the pass was already running.
    ///
    /// It is reported rather than omitted, because leaving it out reads as
    /// though the feed does not exist.
    Unchecked,
}

/// Checks every registered feed, then reports what each one answered.
///
/// Runs the same pass as the feed page's Fetch button and the poll task, so
/// a script and a reader never see different behavior.
///
/// The status is 200 whatever the feeds returned. The request itself
/// succeeded, and a tracker being down is a fact the body carries rather
/// than a failure of this call.
#[route(POST "/api/feeds/check")]
async fn check(cx: &Cx) -> Result<Json<CheckReport>> {
    let registry = app_context::<Arc<FeedRegistry>>(cx);
    let services = app_context::<Services>(cx);

    registry::check_all(
        registry,
        &services.db,
        services.feeds.as_ref(),
        services.clock.as_ref(),
    )
    .await;

    Ok(Json(report(registry.entries())))
}

fn report(entries: Vec<FeedEntry>) -> CheckReport {
    CheckReport {
        feeds: entries
            .into_iter()
            .map(|entry| FeedReport {
                id: entry.id,
                name: entry.name,
                // Bound as `last` rather than `check`, because the `#[route]`
                // attribute above puts a unit struct named `check` in scope.
                outcome: match entry.check {
                    Some(last) => match last.outcome {
                        Ok(ingest) => Outcome::Ok {
                            items: ingest.items,
                            added: ingest.added,
                        },
                        Err(error) => Outcome::Failed { error },
                    },
                    None => Outcome::Unchecked,
                },
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use url::Url;

    use super::report;
    use crate::feed::registry::{FeedCheck, FeedEntry};
    use crate::store::Ingest;

    fn entry(id: &str, name: &str, check: Option<FeedCheck>) -> FeedEntry {
        FeedEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            url: Url::parse(&format!("https://{id}.invalid/rss")).expect("the test URL parses"),
            check,
        }
    }

    fn checked(outcome: Result<Ingest, String>) -> Option<FeedCheck> {
        Some(FeedCheck {
            at: Utc
                .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
                .single()
                .expect("the test instant is unambiguous"),
            outcome,
        })
    }

    #[test]
    fn report_maps_each_check_outcome() {
        let entries = vec![
            entry(
                "f1",
                "Working",
                checked(Ok(Ingest {
                    items: 12,
                    added: 3,
                })),
            ),
            entry("f2", "Broken", checked(Err("timed out".to_owned()))),
            entry("f3", "Fresh", None),
        ];

        assert_eq!(
            serde_json::to_value(report(entries)).expect("the report serializes"),
            json!({
                "feeds": [
                    {"id": "f1", "name": "Working", "outcome": "ok", "items": 12, "added": 3},
                    {"id": "f2", "name": "Broken", "outcome": "failed", "error": "timed out"},
                    {"id": "f3", "name": "Fresh", "outcome": "unchecked"},
                ]
            }),
            "the outcome tags the object and its fields sit beside it"
        );
    }
}
