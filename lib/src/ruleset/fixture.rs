//! The parsers and rulesets the tests parse titles with.
//!
//! The application ships none, so a test that needs a claimed title
//! supplies its own configuration. [`ENGINE`] here is what a test asserts
//! against, and a running process reads its own set from the store.
//!
//! Three parsers, every name invented:
//!
//! - `series-episodes`, the fields an episode name breaks into.
//! - `feature-films`, a title followed by a production year.
//! - `archive-talks`, a publisher-prefixed session number.
//!
//! Four rulesets on them: one for each film and talk parser claiming
//! everything it reads, and two on `series-episodes` narrowed by conditions
//! to one show each.

use std::sync::LazyLock;

use super::{Condition, Op, Ruleset};
use crate::parser::{
    Field, FieldKind,
    FieldKind::{Enum, Episode, Number, Season, Text},
    Parser,
};
use crate::rules::Engine;

/// Builds one field, so a parser below reads as a list of rules rather
/// than a page of struct literals.
fn field(
    name: &str,
    kind: FieldKind,
    pattern: Option<&str>,
    required: bool,
    tight: bool,
    identity: bool,
) -> Field {
    Field {
        name: name.to_owned(),
        kind,
        pattern: pattern.map(ToOwned::to_owned),
        required,
        tight,
        identity,
    }
}

/// Names one condition, so a ruleset below reads as a list of comparisons.
fn equals(field: &str, value: &str) -> Condition {
    Condition {
        field: field.to_owned(),
        op: Op::Equals,
        value: value.to_owned(),
    }
}

/// Builds one ruleset on `parser`, claiming what its conditions admit.
fn on(id: &str, name: &str, parser: &str, conditions: Vec<Condition>) -> Ruleset {
    Ruleset {
        id: id.to_owned(),
        name: name.to_owned(),
        enabled: false,
        parser: parser.to_owned(),
        conditions,
        tests: Vec::new(),
    }
}

/// The fixture parsers and rulesets, compiled once.
///
/// # Panics
///
/// Panics when a pattern fails to compile, which makes a bad fixture
/// pattern a failure of the test run rather than a silent miss.
pub(crate) static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    Engine::new(parsers(), rulesets()).expect("every fixture pattern is a valid regex")
});

/// The three parsers the tests read titles with.
pub(crate) fn parsers() -> Vec<Parser> {
    vec![
        Parser {
            id: "series-episodes".to_owned(),
            name: "Series Episodes".to_owned(),
            fields: vec![
                field("show", Text, Some(r"^(?<show>[\w.]+?)"), true, true, true),
                field("season", Season, None, true, true, true),
                field("episodeNumber", Episode, None, false, false, true),
                field(
                    "resolution",
                    Enum,
                    Some(r"\.(?<resolution>480p|720p|1080p|2160p)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "source",
                    Enum,
                    Some(r"\.(?<source>[A-Z]{3}\.\w+|Broadcast|Webcast|Telecast)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "audio",
                    Text,
                    Some(r"\.(?<audio>AAC\.(?:Mono|Stereo|5\.1)|PCM\.(?:Mono|Stereo|5\.1))"),
                    false,
                    false,
                    false,
                ),
                field(
                    "codec",
                    Enum,
                    Some(r"\.(?<codec>H\.26[45]|AV1|VP9)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "publisher",
                    Text,
                    Some(r"-(?<publisher>[A-Za-z0-9]+)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "extension",
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4|\.avi)$"),
                    false,
                    false,
                    false,
                ),
            ],
            tests: Vec::new(),
        },
        Parser {
            id: "feature-films".to_owned(),
            name: "Feature Films".to_owned(),
            fields: vec![
                field("title", Text, Some(r"^(?<title>[\w.]+?)"), true, true, true),
                field(
                    "year",
                    Number,
                    Some(r"\.(?<year>(?:19|20)\d{2})"),
                    true,
                    false,
                    true,
                ),
                field(
                    "resolution",
                    Enum,
                    Some(r"\.(?<resolution>720p|1080p|2160p)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "source",
                    Enum,
                    Some(r"\.(?<source>Studio\.Master|Remaster|Restoration|Archive)"),
                    true,
                    false,
                    false,
                ),
                // Audio precedes the codec, because a film name carries
                // `AAC.Stereo.H.264` in that order and a component reads the
                // run that follows the one before it.
                field(
                    "audio",
                    Text,
                    Some(r"\.(?<audio>AAC\.(?:Mono|Stereo|5\.1)|PCM\.(?:Mono|Stereo|5\.1))"),
                    false,
                    false,
                    false,
                ),
                field(
                    "codec",
                    Enum,
                    Some(r"\.(?<codec>H\.26[45]|AV1|VP9)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "publisher",
                    Text,
                    Some(r"-(?<publisher>[A-Za-z0-9]+)"),
                    false,
                    false,
                    false,
                ),
                field(
                    "extension",
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4)$"),
                    false,
                    false,
                    false,
                ),
            ],
            tests: Vec::new(),
        },
        Parser {
            id: "archive-talks".to_owned(),
            name: "Archive Talks".to_owned(),
            fields: vec![
                field(
                    "publisher",
                    Text,
                    Some(r"^\[(?<publisher>[^\]]+)\]"),
                    true,
                    false,
                    false,
                ),
                field("show", Text, Some(r"\s(?<show>.+?)"), true, true, true),
                field(
                    "episodeNumber",
                    Number,
                    Some(r"\s-\s(?<episodeNumber>\d{2,3})"),
                    true,
                    false,
                    true,
                ),
                field(
                    "resolution",
                    Enum,
                    Some(r"\s[(\[](?<resolution>480p|720p|1080p)[)\]]"),
                    false,
                    false,
                    false,
                ),
                field(
                    "checksum",
                    Text,
                    Some(r"\s\[(?<checksum>[0-9A-F]{8})\]"),
                    false,
                    false,
                    false,
                ),
                field(
                    "extension",
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4)$"),
                    false,
                    false,
                    false,
                ),
            ],
            tests: Vec::new(),
        },
    ]
}

/// The four rulesets the tests claim titles with.
///
/// The film and talk rulesets write no condition, so each claims every name
/// its parser reads. The two episode rulesets share one parser and name one
/// show each, which is what two rulesets on one parser are for.
pub(crate) fn rulesets() -> Vec<Ruleset> {
    vec![
        on(
            "feature-films",
            "Feature Films",
            "feature-films",
            Vec::new(),
        ),
        on(
            "archive-talks",
            "Archive Talks",
            "archive-talks",
            Vec::new(),
        ),
        on(
            "series-hollow-meridian",
            "The Hollow Meridian",
            "series-episodes",
            vec![
                equals("show", "The Hollow Meridian"),
                equals("resolution", "1080p"),
            ],
        ),
        on(
            "series-ashfall-county",
            "Ashfall County",
            "series-episodes",
            vec![
                equals("show", "Ashfall County"),
                equals("publisher", "PublicWave"),
            ],
        ),
    ]
}
