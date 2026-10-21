//! What the client's torrents say about the values a reader takes.
//!
//! The reader chose every torrent the client holds, so the library is the
//! record of what they accepted. The ranking reads it to place a value that
//! no stated list names, and to fill a list that the reader never stated.

// FIXME: `Library` has no caller outside the tests. The ranking of a value
// that no stated list names, and the order that a field with no list falls
// back on, are what it waits on.
#![allow(dead_code)]

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::engine::Engine;
use crate::parser::{Field, Parser};
use crate::torrent::Torrent;

/// The values the client's torrents carry, built in one pass over the
/// torrent index.
///
/// The held values are kept for each subject, because a group the reader
/// took for one show says nothing about another show. The counts are kept
/// over the whole client, because a field with no stated list needs one
/// order for every show.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Library {
    /// The normalized values of the claimed torrents, by subject and then by
    /// field.
    held: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,

    /// The values of the completed torrents, by field, in the order the
    /// index first lists them.
    seen: BTreeMap<String, Vec<Seen>>,
}

/// One value of one field, with the number of completed torrents that carry
/// it.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    /// The value as the first torrent to carry it wrote it.
    raw: String,

    normalized: String,
    torrents: usize,
}

impl Library {
    /// Builds the library from `torrents`, newest first as
    /// [`index::all`](crate::torrent::index::all) returns them.
    ///
    /// The order decides the spelling and the tie order of a suggested value.
    /// The engine reads every name, and a completed one twice, so build one
    /// library per request and share it.
    pub(crate) fn new(engine: &Engine, torrents: &[Torrent]) -> Self {
        let mut library = Self::default();

        for torrent in torrents {
            if let Some(parsed) = engine.parse(&torrent.name)
                && let Some(parser) = engine.parser(&parsed.parser)
            {
                let fields = library.held.entry(parsed.identity.subject()).or_default();

                for (field, raw) in ranked(parser, &parsed.values) {
                    fields
                        .entry(field.name.clone())
                        .or_default()
                        .insert(field.kind.normalize(raw));
                }
            }

            if torrent.progress >= 1.0
                && let Some(reading) = engine.read(&torrent.name)
                && let Some(parser) = engine.parser(&reading.parser)
            {
                for (field, raw) in ranked(parser, &reading.values) {
                    let normalized = field.kind.normalize(raw);
                    let seen = library.seen.entry(field.name.clone()).or_default();

                    match seen.iter_mut().find(|one| one.normalized == normalized) {
                        Some(one) => one.torrents += 1,
                        None => seen.push(Seen {
                            raw: raw.to_owned(),
                            normalized,
                            torrents: 1,
                        }),
                    }
                }
            }
        }

        library
    }

    /// Returns whether a claimed torrent of `subject` carries `normalized`
    /// for `field`.
    ///
    /// `subject` is an [`Identity::subject`](crate::engine::Identity::subject)
    /// key, and `normalized` is the value as its field's kind normalizes it.
    ///
    /// A torrent counts at any progress, because a grab still in flight is
    /// already a group the reader took for that show. A name that no search
    /// claims files under no subject.
    pub(crate) fn holds(&self, subject: &str, field: &str, normalized: &str) -> bool {
        self.held
            .get(subject)
            .and_then(|fields| fields.get(field))
            .is_some_and(|values| values.contains(normalized))
    }

    /// Returns each value that two or more completed torrents carry for
    /// `field`, with its count, most carried first.
    ///
    /// Every name a parser reads counts, whether or not a search claims it,
    /// so a show with no search still says which groups the reader takes.
    /// Only a completed torrent counts, so a download in progress or stopped
    /// part way does not move the order.
    ///
    /// A value only one torrent carries is left out, because a field whose
    /// every value appears once, such as a checksum or an episode name,
    /// names nothing to prefer.
    ///
    /// Each value keeps the spelling of the newest torrent that carries it,
    /// and two values with one count keep the order the index lists them in.
    pub(crate) fn suggested(&self, field: &str) -> Vec<(String, usize)> {
        let mut suggested: Vec<(String, usize)> = self
            .seen
            .get(field)
            .into_iter()
            .flatten()
            .filter(|seen| seen.torrents >= 2)
            .map(|seen| (seen.raw.clone(), seen.torrents))
            .collect();

        suggested.sort_by_key(|(_, torrents)| Reverse(*torrents));
        suggested
    }
}

/// Pairs each value `parser` read outside its identity with the field that
/// read it.
///
/// Every copy of one identity carries the same identity values, so those say
/// nothing about which copy the reader takes.
fn ranked<'a>(
    parser: &'a Parser,
    values: &'a [(String, String)],
) -> impl Iterator<Item = (&'a Field, &'a str)> {
    values.iter().filter_map(|(name, raw)| {
        parser
            .fields
            .iter()
            .find(|field| field.name == *name && !field.identity)
            .map(|field| (field, raw.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::Library;
    use crate::search::fixture::ENGINE;
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    /// A show the fixture's `series-hollow-meridian` search claims at 1080p.
    const HOLLOW: &str = "The.Hollow.Meridian";

    /// A show the `series-episodes` parser reads and no fixture search claims.
    const COASTAL: &str = "Coastal.Ecology";

    /// An invented 1080p episode name of `show` from `publisher`.
    fn episode(show: &str, number: u32, publisher: &str) -> String {
        format!("{show}.S04E{number:02}.1080p.Broadcast.AAC.Stereo.H.264-{publisher}.mkv")
    }

    fn torrent(name: &str, progress: f32) -> Torrent {
        Torrent {
            id: TorrentId(name.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Paused,
            size: 734_003_200,
            progress,
            added_at: None,
        }
    }

    fn suggestion(raw: &str, torrents: usize) -> (String, usize) {
        (raw.to_owned(), torrents)
    }

    #[test]
    fn holds_files_a_claimed_torrent_under_its_subject() {
        let library = Library::new(&ENGINE, &[torrent(&episode(HOLLOW, 6, "PublicWave"), 0.5)]);

        assert!(
            library.holds("show|the hollow meridian", "publisher", "publicwave"),
            "a claimed torrent counts before it completes"
        );
        assert!(
            !library.holds("show|the hollow meridian", "resolution", "2160p"),
            "the torrent reads 1080p"
        );
        assert!(
            !library.holds("title|the hollow meridian", "publisher", "publicwave"),
            "a film of the same name is another subject"
        );
    }

    #[test]
    fn suggested_counts_completed_torrents_most_first() {
        let library = Library::new(
            &ENGINE,
            &[
                torrent(&episode(HOLLOW, 6, "PublicWave"), 0.5),
                torrent(&episode(HOLLOW, 5, "PublicWave"), 1.0),
                torrent(&episode(HOLLOW, 4, "OtherGroup"), 1.0),
                torrent(&episode(HOLLOW, 3, "PublicWave"), 1.0),
                torrent(&episode(HOLLOW, 2, "OtherGroup"), 1.0),
                torrent(&episode(HOLLOW, 1, "OtherGroup"), 1.0),
            ],
        );

        assert_eq!(
            library.suggested("publisher"),
            [suggestion("OtherGroup", 3), suggestion("PublicWave", 2)],
            "the download in progress leaves PublicWave one behind"
        );
    }

    #[test]
    fn a_value_one_torrent_holds_is_no_suggestion() {
        let library = Library::new(
            &ENGINE,
            &[
                torrent(&episode(HOLLOW, 2, "PublicWave"), 1.0),
                torrent(&episode(HOLLOW, 1, "OtherGroup"), 1.0),
            ],
        );

        assert_eq!(
            library.suggested("publisher"),
            Vec::<(String, usize)>::new()
        );
        assert_eq!(
            library.suggested("resolution"),
            [suggestion("1080p", 2)],
            "a value both torrents carry is one"
        );
    }

    #[test]
    fn a_name_no_search_claims_still_counts() {
        let library = Library::new(
            &ENGINE,
            &[
                torrent(&episode(COASTAL, 3, "PublicWave"), 1.0),
                torrent(&episode(COASTAL, 2, "PublicWave"), 1.0),
            ],
        );

        assert_eq!(
            library.suggested("publisher"),
            [suggestion("PublicWave", 2)],
            "a show with no search still says which groups the reader takes"
        );
        assert!(
            !library.holds("show|coastal ecology", "publisher", "publicwave"),
            "only a claimed torrent files under a subject"
        );
    }
}
