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
//!
//! [`super::FieldKind::Season`] and [`super::FieldKind::Episode`] arrive under
//! an alias, because [`super::Part`] names those two parts as well and the
//! parts are used far more often in this file.

use std::sync::LazyLock;

use super::{
    Field, FieldKind,
    FieldKind::{Enum, Episode as EpisodeKind, Number, Season as SeasonKind, Text},
    Part,
    Part::{
        Audio, Checksum, Codec, Episode, Extension, Movie, Publisher, Resolution, Season, Show,
        Source, Year,
    },
    Ruleset,
};
use crate::rules::Engine;

/// Builds one field, so a ruleset below reads as a list of rules rather
/// than a page of struct literals.
fn field(
    name: &str,
    part: Part,
    kind: FieldKind,
    pattern: Option<&str>,
    required: bool,
    identity: bool,
) -> Field {
    Field {
        name: name.to_owned(),
        part,
        kind,
        pattern: pattern.map(ToOwned::to_owned),
        required,
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
    Engine::from_rulesets(rulesets()).expect("every fixture pattern is a valid regex")
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
                field(
                    "show",
                    Show,
                    Text,
                    Some(r"^(?<show>[\w.]+?)\.S\d"),
                    true,
                    true,
                ),
                field("season", Season, SeasonKind, None, true, true),
                field("episode", Episode, EpisodeKind, None, false, true),
                field(
                    "resolution",
                    Resolution,
                    Enum,
                    Some(r"(?<resolution>480p|720p|1080p|2160p)"),
                    false,
                    false,
                ),
                field(
                    "source",
                    Source,
                    Enum,
                    Some(r"(?<source>[A-Z]{3}\.\w+|Broadcast|Webcast|Telecast)"),
                    false,
                    false,
                ),
                field(
                    "audio",
                    Audio,
                    Text,
                    Some(r"(?<audio>AAC\.(Mono|Stereo|5\.1)|PCM\.[\w.]+)"),
                    false,
                    false,
                ),
                field(
                    "codec",
                    Codec,
                    Enum,
                    Some(r"(?<codec>H\.26[45]|AV1|VP9)"),
                    false,
                    false,
                ),
                field(
                    "publisher",
                    Publisher,
                    Text,
                    Some(r"-(?<publisher>[A-Za-z0-9]+)\.\w+$"),
                    false,
                    false,
                ),
                field(
                    "extension",
                    Extension,
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4|\.avi)$"),
                    false,
                    false,
                ),
            ],
            tests: Vec::new(),
        },
        Ruleset {
            id: "feature-films".to_owned(),
            name: "Feature Films".to_owned(),
            enabled: false,
            template: false,
            based_on: None,
            fields: vec![
                field(
                    "title",
                    Movie,
                    Text,
                    Some(r"^(?<title>[\w.]+?)\.(?:19|20)\d{2}\."),
                    true,
                    true,
                ),
                field(
                    "year",
                    Year,
                    Number,
                    Some(r"\.(?<year>(19|20)\d{2})\."),
                    true,
                    true,
                ),
                field(
                    "resolution",
                    Resolution,
                    Enum,
                    Some(r"(?<resolution>720p|1080p|2160p)"),
                    false,
                    false,
                ),
                field(
                    "source",
                    Source,
                    Enum,
                    Some(r"(?<source>Studio\.Master|Remaster|Restoration|Archive)"),
                    true,
                    false,
                ),
                field(
                    "codec",
                    Codec,
                    Enum,
                    Some(r"(?<codec>H\.26[45]|AV1|VP9)"),
                    false,
                    false,
                ),
                field(
                    "audio",
                    Audio,
                    Text,
                    Some(r"(?<audio>PCM\.[\w.]+|AAC\.[\w.]+)"),
                    false,
                    false,
                ),
                field(
                    "publisher",
                    Publisher,
                    Text,
                    Some(r"-(?<publisher>[A-Za-z0-9]+)\.\w+$"),
                    false,
                    false,
                ),
                field(
                    "extension",
                    Extension,
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4)$"),
                    false,
                    false,
                ),
            ],
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
                    Publisher,
                    Text,
                    Some(r"^\[(?<publisher>[^\]]+)\]"),
                    true,
                    false,
                ),
                field(
                    "show",
                    Show,
                    Text,
                    Some(r"\]\s(?<show>.+?)\s-\s\d"),
                    true,
                    true,
                ),
                field(
                    "episode",
                    Episode,
                    Number,
                    Some(r"\s-\s(?<episode>\d{2,3})"),
                    true,
                    true,
                ),
                field(
                    "resolution",
                    Resolution,
                    Enum,
                    Some(r"[(\[](?<resolution>480p|720p|1080p)[)\]]"),
                    false,
                    false,
                ),
                field(
                    "checksum",
                    Checksum,
                    Text,
                    Some(r"\[(?<checksum>[0-9A-F]{8})\]"),
                    false,
                    false,
                ),
                field(
                    "extension",
                    Extension,
                    Enum,
                    Some(r"(?<extension>\.mkv|\.mp4)$"),
                    false,
                    false,
                ),
            ],
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
                    Show,
                    Text,
                    Some(r"^(?<show>The\.Hollow\.Meridian)\.S\d"),
                    true,
                    true,
                ),
                field(
                    "resolution",
                    Resolution,
                    Enum,
                    Some(r"(?<resolution>1080p)"),
                    true,
                    false,
                ),
            ],
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
                    Show,
                    Text,
                    Some(r"^(?<show>Ashfall\.County)\.S\d"),
                    true,
                    true,
                ),
                field(
                    "publisher",
                    Publisher,
                    Text,
                    Some(r"-(?<publisher>PublicWave)\.\w+$"),
                    true,
                    false,
                ),
            ],
            tests: Vec::new(),
        },
    ]
}
