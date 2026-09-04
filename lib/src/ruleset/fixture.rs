//! The rulesets the tests parse titles with.
//!
//! The application ships none, so a test that needs a claimed title
//! supplies its own configuration. [`ENGINE`] here is what a test asserts
//! against, and a running process reads its own set from the store.
//!
//! Five rulesets, every name invented:
//!
//! - `series-episodes`, a template that claims nothing and holds the fields
//!   an episode name breaks into.
//! - `series-hollow-meridian`, a ruleset on it for one show at 1080p.
//! - `series-ashfall-county`, a second, told apart by publisher.
//! - `feature-films`, a title followed by a production year.
//! - `archive-talks`, a publisher-prefixed session number.

use std::sync::LazyLock;

use super::Ruleset;
use crate::parser::{
    Field, FieldKind,
    FieldKind::{Enum, Episode, Number, Season, Text},
};
use crate::rules::Engine;

/// Builds one field, so a ruleset below reads as a list of rules rather
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

/// The fixture rulesets, compiled once.
///
/// # Panics
///
/// Panics when a pattern fails to compile, which makes a bad fixture
/// pattern a failure of the test run rather than a silent miss.
pub(crate) static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    Engine::new(Vec::new(), rulesets()).expect("every fixture pattern is a valid regex")
});

/// The five rulesets the tests parse against.
pub(crate) fn rulesets() -> Vec<Ruleset> {
    vec![
        Ruleset {
            id: "series-episodes".to_owned(),
            name: "Series Episodes".to_owned(),
            enabled: false,
            template: true,
            based_on: None,
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
            conditions: Vec::new(),
            tests: Vec::new(),
        },
        Ruleset {
            id: "feature-films".to_owned(),
            name: "Feature Films".to_owned(),
            enabled: false,
            template: false,
            based_on: None,
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
            conditions: Vec::new(),
            tests: Vec::new(),
        },
        Ruleset {
            id: "archive-talks".to_owned(),
            name: "Archive Talks".to_owned(),
            enabled: false,
            template: false,
            based_on: None,
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
            conditions: Vec::new(),
            tests: Vec::new(),
        },
        Ruleset {
            id: "series-hollow-meridian".to_owned(),
            name: "The Hollow Meridian".to_owned(),
            enabled: false,
            template: false,
            based_on: Some("series-episodes".to_owned()),
            fields: vec![
                field(
                    "show",
                    Text,
                    Some(r"^(?<show>The\.Hollow\.Meridian)"),
                    true,
                    false,
                    true,
                ),
                field(
                    "resolution",
                    Enum,
                    Some(r"\.(?<resolution>1080p)"),
                    true,
                    false,
                    false,
                ),
            ],
            conditions: Vec::new(),
            tests: Vec::new(),
        },
        Ruleset {
            id: "series-ashfall-county".to_owned(),
            name: "Ashfall County".to_owned(),
            enabled: false,
            template: false,
            based_on: Some("series-episodes".to_owned()),
            fields: vec![
                field(
                    "show",
                    Text,
                    Some(r"^(?<show>Ashfall\.County)"),
                    true,
                    false,
                    true,
                ),
                field(
                    "publisher",
                    Text,
                    Some(r"-(?<publisher>PublicWave)"),
                    true,
                    false,
                    false,
                ),
            ],
            conditions: Vec::new(),
            tests: Vec::new(),
        },
    ]
}
