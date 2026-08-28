//! Mock candidate filenames the ruleset editor shows as a diff.
//!
//! Each list stands for one ruleset run against a feed's backlog, with every
//! entry already carrying the state it lands in under an unsaved edit. No
//! pattern runs, so the states are authored rather than computed.
//!
//! Every title, release group, and feed name is invented.

use super::{
    Candidate,
    Diff::{Added, Excluded, Kept, Removed},
    Part::{
        Audio, Checksum, Codec, Episode, Extension, Movie, Publisher, Resolution, Season, Show,
        Source, Year,
    },
    gap, hit,
};

/// Segments for a dot-separated broadcast episode name.
macro_rules! episode {
    ($show:literal, $season:literal, $episode:literal, $resolution:literal,
     $source:literal, $audio:literal, $codec:literal, $group:literal) => {
        &[
            hit($show, Show),
            gap("."),
            hit($season, Season),
            hit($episode, Episode),
            gap("."),
            hit($resolution, Resolution),
            gap("."),
            hit($source, Source),
            gap("."),
            hit($audio, Audio),
            gap("."),
            hit($codec, Codec),
            gap("-"),
            hit($group, Publisher),
            hit(".mkv", Extension),
        ]
    };
}

/// Segments for a dot-separated feature name carrying a release year.
macro_rules! feature {
    ($title:literal, $year:literal, $resolution:literal, $source:literal,
     $codec:literal, $audio:literal, $group:literal) => {
        &[
            hit($title, Movie),
            gap("."),
            hit($year, Year),
            gap("."),
            hit($resolution, Resolution),
            gap("."),
            hit($source, Source),
            gap("."),
            hit($codec, Codec),
            gap("."),
            hit($audio, Audio),
            gap("-"),
            hit($group, Publisher),
            hit(".mkv", Extension),
        ]
    };
}

/// Segments for a publisher-prefixed session name ending in a checksum.
macro_rules! session_checksum {
    ($group:literal, $show:literal, $episode:literal, $resolution:literal, $checksum:literal) => {
        &[
            hit($group, Publisher),
            gap(" "),
            hit($show, Show),
            gap(" - "),
            hit($episode, Episode),
            gap(" ("),
            hit($resolution, Resolution),
            gap(") ["),
            hit($checksum, Checksum),
            gap("]"),
            hit(".mkv", Extension),
        ]
    };
}

/// Segments for a publisher-prefixed session name ending in format tags.
macro_rules! session_tags {
    ($group:literal, $show:literal, $episode:literal, $resolution:literal, $codec:literal) => {
        &[
            hit($group, Publisher),
            gap(" "),
            hit($show, Show),
            gap(" - "),
            hit($episode, Episode),
            gap(" ["),
            hit($resolution, Resolution),
            gap("]["),
            hit($codec, Codec),
            gap("]"),
            hit(".mkv", Extension),
        ]
    };
}

/// Segments for a filename no rule claimed.
///
/// This is a macro rather than a `const fn`. A function returning the slice
/// borrows an array it builds itself, and const promotion keeps the array
/// alive only when every element is a constant.
macro_rules! unclaimed {
    ($name:literal) => {
        &[gap($name)]
    };
}

pub(crate) const SERIES_EPISODES: &[Candidate] = &[
    Candidate {
        id: "ep-01",
        segments: episode!(
            "The.Hollow.Meridian",
            "S05",
            "E01",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Added,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ep-02",
        segments: episode!(
            "Glasswing",
            "S02",
            "E04",
            "2160p",
            "PBW.Broadcast",
            "AAC.5.1",
            "H.265",
            "Northlight"
        ),
        diff: Added,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ep-03",
        segments: episode!(
            "Ashfall.County",
            "S02",
            "E01",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Added,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ep-04",
        segments: episode!(
            "Nine.Lantern.Road",
            "S03",
            "E02",
            "720p",
            "Telecast",
            "AAC.5.1",
            "H.264",
            "CivicArchive"
        ),
        diff: Removed,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ep-05",
        segments: episode!(
            "Glasswing",
            "S01",
            "E09",
            "1080p",
            "Webcast",
            "AAC.Mono",
            "H.264",
            "OpenReel"
        ),
        diff: Removed,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ep-06",
        segments: episode!(
            "The.Hollow.Meridian",
            "S04",
            "E06",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ep-07",
        segments: episode!(
            "The.Hollow.Meridian",
            "S04",
            "E07",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ep-08",
        segments: episode!(
            "Glasswing",
            "S02",
            "E03",
            "2160p",
            "PBW.Broadcast",
            "AAC.5.1",
            "H.265",
            "Northlight"
        ),
        diff: Kept,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ep-09",
        segments: episode!(
            "Nine.Lantern.Road",
            "S03",
            "E01",
            "1080p",
            "CVA.Archive",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ep-10",
        segments: episode!(
            "Ashfall.County",
            "S01",
            "E10",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ep-11",
        segments: unclaimed!(
            "Paper.Continents.2024.1080p.Remaster.H.264.PCM.Stereo-MeridianPress.mkv"
        ),
        diff: Excluded,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "ep-12",
        segments: unclaimed!("[OpenReel] Coastal.Ecology - 18 (1080p) [A1B2C3D4].mkv"),
        diff: Excluded,
        feed: "openreel-talks",
    },
];

pub(crate) const FEATURE_FILMS: &[Candidate] = &[
    Candidate {
        id: "fm-01",
        segments: feature!(
            "Neon.Harbor.2049",
            "2017",
            "1080p",
            "Remaster",
            "H.264",
            "AAC.5.1",
            "MeridianPress"
        ),
        diff: Added,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "fm-02",
        segments: feature!(
            "Salt.Glass.Winter",
            "2021",
            "2160p",
            "Studio.Master",
            "H.265",
            "PCM.5.1",
            "AtlasReel"
        ),
        diff: Added,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "fm-03",
        segments: feature!(
            "Paper.Continents",
            "2024",
            "1080p",
            "Broadcast",
            "H.264",
            "AAC.Stereo",
            "Northlight"
        ),
        diff: Removed,
        feed: "northlight-hd",
    },
    Candidate {
        id: "fm-04",
        segments: feature!(
            "Paper.Continents",
            "2024",
            "1080p",
            "Remaster",
            "H.264",
            "PCM.Stereo",
            "MeridianPress"
        ),
        diff: Kept,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "fm-05",
        segments: feature!(
            "Neon.Harbor.2049",
            "2017",
            "2160p",
            "Studio.Master",
            "H.265",
            "PCM.5.1",
            "AtlasReel"
        ),
        diff: Kept,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "fm-06",
        segments: feature!(
            "Hollow.Tide",
            "1998",
            "1080p",
            "Remaster",
            "H.264",
            "AAC.5.1",
            "MeridianPress"
        ),
        diff: Kept,
        feed: "civicarchive-films",
    },
    Candidate {
        id: "fm-07",
        segments: unclaimed!(
            "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv"
        ),
        diff: Excluded,
        feed: "publicwave-series",
    },
    Candidate {
        id: "fm-08",
        segments: unclaimed!("Coastal.Ecology.Recap.1080p.Broadcast.H.264-PublicWave.mkv"),
        diff: Excluded,
        feed: "openreel-talks",
    },
];

pub(crate) const ARCHIVE_TALKS: &[Candidate] = &[
    Candidate {
        id: "tk-01",
        segments: session_tags!("[Northlight]", "Tidal.Systems", "08", "1080p", "AV1"),
        diff: Added,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-02",
        segments: session_checksum!("[OpenReel]", "Glass Chemistry", "03", "720p", "E5F6A7B8"),
        diff: Added,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-03",
        segments: session_checksum!("[AtlasReel]", "Coastal.Ecology", "12", "480p", "11223344"),
        diff: Removed,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-04",
        segments: session_checksum!("[OpenReel]", "Coastal.Ecology", "18", "1080p", "A1B2C3D4"),
        diff: Kept,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-05",
        segments: session_checksum!("[OpenReel]", "Coastal.Ecology", "19", "1080p", "B2C3D4E5"),
        diff: Kept,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-06",
        segments: session_tags!("[Northlight]", "Tidal.Systems", "07", "1080p", "AV1"),
        diff: Kept,
        feed: "openreel-talks",
    },
    Candidate {
        id: "tk-07",
        segments: unclaimed!(
            "Ashfall.County.S01E10.1080p.NLT.Broadcast.AAC.Stereo.H.264-PublicWave.mkv"
        ),
        diff: Excluded,
        feed: "publicwave-series",
    },
    Candidate {
        id: "tk-08",
        segments: unclaimed!(
            "Neon.Harbor.2049.2017.1080p.Remaster.H.264.AAC.5.1-MeridianPress.mkv"
        ),
        diff: Excluded,
        feed: "civicarchive-films",
    },
];

/// Candidates for a child ruleset narrowed to one series.
///
/// A narrowed ruleset rejects the sibling series the parent accepts, so those
/// filenames sit here as removals rather than vanishing from the list. That
/// is the point of the preview: it shows what the narrowing costs.
pub(crate) const HOLLOW_MERIDIAN: &[Candidate] = &[
    Candidate {
        id: "hm-01",
        segments: episode!(
            "The.Hollow.Meridian",
            "S05",
            "E01",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Added,
        feed: "publicwave-series",
    },
    Candidate {
        id: "hm-02",
        segments: episode!(
            "The.Hollow.Meridian",
            "S04",
            "E06",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "hm-03",
        segments: episode!(
            "The.Hollow.Meridian",
            "S04",
            "E07",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "hm-04",
        segments: episode!(
            "The.Hollow.Meridian",
            "S03",
            "E02",
            "720p",
            "Telecast",
            "AAC.5.1",
            "H.264",
            "CivicArchive"
        ),
        diff: Removed,
        feed: "northlight-hd",
    },
    Candidate {
        id: "hm-05",
        segments: episode!(
            "Glasswing",
            "S02",
            "E03",
            "2160p",
            "PBW.Broadcast",
            "AAC.5.1",
            "H.265",
            "Northlight"
        ),
        diff: Excluded,
        feed: "northlight-hd",
    },
    Candidate {
        id: "hm-06",
        segments: episode!(
            "Ashfall.County",
            "S01",
            "E10",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Excluded,
        feed: "publicwave-series",
    },
];

/// Candidates for a child ruleset narrowed to one series and one publisher.
pub(crate) const ASHFALL_COUNTY: &[Candidate] = &[
    Candidate {
        id: "ac-01",
        segments: episode!(
            "Ashfall.County",
            "S02",
            "E01",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Added,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ac-02",
        segments: episode!(
            "Ashfall.County",
            "S01",
            "E10",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Kept,
        feed: "publicwave-series",
    },
    Candidate {
        id: "ac-03",
        segments: episode!(
            "Ashfall.County",
            "S01",
            "E09",
            "1080p",
            "NLT.Broadcast",
            "AAC.Stereo",
            "H.264",
            "MeridianPress"
        ),
        diff: Removed,
        feed: "northlight-hd",
    },
    Candidate {
        id: "ac-04",
        segments: episode!(
            "The.Hollow.Meridian",
            "S04",
            "E06",
            "1080p",
            "Broadcast",
            "AAC.Stereo",
            "H.264",
            "PublicWave"
        ),
        diff: Excluded,
        feed: "publicwave-series",
    },
];
