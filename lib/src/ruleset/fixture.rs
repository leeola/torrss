//! The rulesets the tests parse titles with.
//!
//! The application ships none, so a test that needs a claimed title
//! supplies its own configuration. [`ENGINE`] here is what a test asserts
//! against, and [`crate::rules::ENGINE`] is the empty shipped one.
//!
//! Five rulesets, every name invented:
//!
//! - `series-episodes`, a base that claims any episode name.
//! - `series-hollow-meridian`, a child narrowing it to one show at 1080p.
//! - `series-ashfall-county`, a second child, narrowed by publisher.
//! - `feature-films`, a title followed by a production year.
//! - `archive-talks`, a publisher-prefixed session number.
//!
//! [`super::FieldKind::Season`] and [`super::FieldKind::Episode`] arrive under
//! an alias, because [`super::Part`] names those two parts as well and the
//! parts are used far more often in this file.

use std::sync::LazyLock;

use super::{
    Field,
    FieldKind::{Enum, Episode as EpisodeKind, Number, Season as SeasonKind, Text},
    Part::{
        Audio, Checksum, Codec, Episode, Extension, Movie, Publisher, Resolution, Season, Show,
        Source, Year,
    },
    Ruleset,
};
use crate::rules::Engine;

/// The fixture rulesets, compiled once.
///
/// # Panics
///
/// Panics when a pattern fails to compile, which makes a bad fixture
/// pattern a failure of the test run rather than a silent miss.
pub(crate) static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    Engine::from_rulesets(RULESETS).expect("every fixture pattern is a valid regex")
});

pub(crate) const RULESETS: &[Ruleset] = &[
    Ruleset {
        id: "series-episodes",
        name: "Series Episodes",
        enabled: false,
        inherits: None,
        fields: &[
            Field {
                name: "show",
                part: Show,
                kind: Text,
                pattern: Some(r"^(?<show>[\w.]+?)\.S\d"),
                required: true,
                identity: true,
            },
            Field {
                name: "season",
                part: Season,
                kind: SeasonKind,
                pattern: None,
                required: true,
                identity: true,
            },
            Field {
                name: "episode",
                part: Episode,
                kind: EpisodeKind,
                pattern: None,
                required: false,
                identity: true,
            },
            Field {
                name: "resolution",
                part: Resolution,
                kind: Enum,
                pattern: Some(r"(?<resolution>480p|720p|1080p|2160p)"),
                required: false,
                identity: false,
            },
            Field {
                name: "source",
                part: Source,
                kind: Enum,
                pattern: Some(r"(?<source>[A-Z]{3}\.\w+|Broadcast|Webcast|Telecast)"),
                required: false,
                identity: false,
            },
            Field {
                name: "audio",
                part: Audio,
                kind: Text,
                pattern: Some(r"(?<audio>AAC\.(Mono|Stereo|5\.1)|PCM\.[\w.]+)"),
                required: false,
                identity: false,
            },
            Field {
                name: "codec",
                part: Codec,
                kind: Enum,
                pattern: Some(r"(?<codec>H\.26[45]|AV1|VP9)"),
                required: false,
                identity: false,
            },
            Field {
                name: "publisher",
                part: Publisher,
                kind: Text,
                pattern: Some(r"-(?<publisher>[A-Za-z0-9]+)\.\w+$"),
                required: false,
                identity: false,
            },
            Field {
                name: "extension",
                part: Extension,
                kind: Enum,
                pattern: Some(r"(?<extension>\.mkv|\.mp4|\.avi)$"),
                required: false,
                identity: false,
            },
        ],
    },
    Ruleset {
        id: "feature-films",
        name: "Feature Films",
        enabled: false,
        inherits: None,
        fields: &[
            Field {
                name: "title",
                part: Movie,
                kind: Text,
                pattern: Some(r"^(?<title>[\w.]+?)\.(?:19|20)\d{2}\."),
                required: true,
                identity: true,
            },
            Field {
                name: "year",
                part: Year,
                kind: Number,
                pattern: Some(r"\.(?<year>(19|20)\d{2})\."),
                required: true,
                identity: true,
            },
            Field {
                name: "resolution",
                part: Resolution,
                kind: Enum,
                pattern: Some(r"(?<resolution>720p|1080p|2160p)"),
                required: false,
                identity: false,
            },
            Field {
                name: "source",
                part: Source,
                kind: Enum,
                pattern: Some(r"(?<source>Studio\.Master|Remaster|Restoration|Archive)"),
                required: true,
                identity: false,
            },
            Field {
                name: "codec",
                part: Codec,
                kind: Enum,
                pattern: Some(r"(?<codec>H\.26[45]|AV1|VP9)"),
                required: false,
                identity: false,
            },
            Field {
                name: "audio",
                part: Audio,
                kind: Text,
                pattern: Some(r"(?<audio>PCM\.[\w.]+|AAC\.[\w.]+)"),
                required: false,
                identity: false,
            },
            Field {
                name: "publisher",
                part: Publisher,
                kind: Text,
                pattern: Some(r"-(?<publisher>[A-Za-z0-9]+)\.\w+$"),
                required: false,
                identity: false,
            },
            Field {
                name: "extension",
                part: Extension,
                kind: Enum,
                pattern: Some(r"(?<extension>\.mkv|\.mp4)$"),
                required: false,
                identity: false,
            },
        ],
    },
    Ruleset {
        id: "archive-talks",
        name: "Archive Talks",
        enabled: false,
        inherits: None,
        fields: &[
            Field {
                name: "publisher",
                part: Publisher,
                kind: Text,
                pattern: Some(r"^\[(?<publisher>[^\]]+)\]"),
                required: true,
                identity: false,
            },
            Field {
                name: "show",
                part: Show,
                kind: Text,
                pattern: Some(r"\]\s(?<show>.+?)\s-\s\d"),
                required: true,
                identity: true,
            },
            Field {
                name: "episode",
                part: Episode,
                kind: Number,
                pattern: Some(r"\s-\s(?<episode>\d{2,3})"),
                required: true,
                identity: true,
            },
            Field {
                name: "resolution",
                part: Resolution,
                kind: Enum,
                pattern: Some(r"[(\[](?<resolution>480p|720p|1080p)[)\]]"),
                required: false,
                identity: false,
            },
            Field {
                name: "checksum",
                part: Checksum,
                kind: Text,
                pattern: Some(r"\[(?<checksum>[0-9A-F]{8})\]"),
                required: false,
                identity: false,
            },
            Field {
                name: "extension",
                part: Extension,
                kind: Enum,
                pattern: Some(r"(?<extension>\.mkv|\.mp4)$"),
                required: false,
                identity: false,
            },
        ],
    },
    Ruleset {
        id: "series-hollow-meridian",
        name: "The Hollow Meridian",
        enabled: false,
        inherits: Some("series-episodes"),
        fields: &[
            Field {
                name: "show",
                part: Show,
                kind: Text,
                pattern: Some(r"^(?<show>The\.Hollow\.Meridian)\.S\d"),
                required: true,
                identity: true,
            },
            Field {
                name: "resolution",
                part: Resolution,
                kind: Enum,
                pattern: Some(r"(?<resolution>1080p)"),
                required: true,
                identity: false,
            },
        ],
    },
    Ruleset {
        id: "series-ashfall-county",
        name: "Ashfall County",
        enabled: false,
        inherits: Some("series-episodes"),
        fields: &[
            Field {
                name: "show",
                part: Show,
                kind: Text,
                pattern: Some(r"^(?<show>Ashfall\.County)\.S\d"),
                required: true,
                identity: true,
            },
            Field {
                name: "publisher",
                part: Publisher,
                kind: Text,
                pattern: Some(r"-(?<publisher>PublicWave)\.\w+$"),
                required: true,
                identity: false,
            },
        ],
    },
];
