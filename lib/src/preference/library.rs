//! What the client's torrents say about the values a reader takes.
//!
//! The reader chose every torrent the client holds, so the library is the
//! record of what they accepted. The ranking reads it to place a value that
//! no stated list names, and to fill a list that the reader never stated.
//! The tie breaker reads it to order two copies that the lists rank alike.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::engine::Engine;
use crate::parser::{Field, Parser, SEPARATORS};
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

    /// The title tokens of the claimed torrents by subject, each with the
    /// number of torrents that carry it.
    affinity: BTreeMap<String, BTreeMap<String, usize>>,

    /// The values of the completed torrents, by field, in the order the
    /// index first lists them.
    seen: BTreeMap<String, Vec<Seen>>,

    /// The title tokens of the completed torrents, each with the number of
    /// torrents that carry it.
    habit: BTreeMap<String, usize>,
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
                let subject = parsed.identity.subject();
                let fields = library.held.entry(subject.clone()).or_default();

                for (field, raw) in ranked(parser, &parsed.values) {
                    fields
                        .entry(field.name.clone())
                        .or_default()
                        .insert(field.kind.normalize(raw));
                }

                let affinity = library.affinity.entry(subject).or_default();
                for token in tokens(&torrent.name) {
                    *affinity.entry(token).or_default() += 1;
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

                for token in tokens(&torrent.name) {
                    *library.habit.entry(token).or_default() += 1;
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

    /// Returns how similar `title` is to the torrents the reader took of
    /// `subject`.
    ///
    /// Each distinct token of `title` adds the number of claimed torrents of
    /// `subject` that carry it. A token is a run between separators, and a
    /// token that no field reads counts too. A streaming service or an HDR
    /// tag therefore separates two copies that read alike. A claimed torrent
    /// counts at any progress, as it does for [`Self::holds`].
    ///
    /// When no claimed torrent files under `subject`, every completed torrent
    /// that a parser reads counts instead. A show that the reader holds
    /// nothing of then follows their habits across the other shows.
    ///
    /// Compare two sums only between copies that
    /// [`Preferences::rank`](crate::preference::Preferences::rank) places
    /// alike. A token states nothing the reader chose, so it never outranks
    /// a list.
    pub(crate) fn affinity(&self, subject: &str, title: &str) -> usize {
        let counts = self
            .affinity
            .get(subject)
            .filter(|counts| !counts.is_empty())
            .unwrap_or(&self.habit);

        tokens(title)
            .iter()
            .filter_map(|token| counts.get(token))
            .sum()
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

/// The distinct words of `name`, lowercased.
///
/// A tracker and a client capitalize one name differently, so `ATV` and
/// `atv` are one token.
fn tokens(name: &str) -> BTreeSet<String> {
    name.to_lowercase()
        .split(SEPARATORS)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
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

    #[test]
    fn affinity_sums_the_tokens_the_subjects_torrents_carry() {
        let library = Library::new(
            &ENGINE,
            &[
                torrent(&episode(HOLLOW, 5, "PublicWave"), 1.0),
                torrent(&episode(HOLLOW, 3, "PublicWave"), 1.0),
            ],
        );

        assert_eq!(
            library.affinity(
                "show|the hollow meridian",
                &episode(HOLLOW, 9, "PublicWave")
            ),
            22,
            "both torrents carry every token but the episode"
        );
        assert_eq!(
            library.affinity(
                "show|the hollow meridian",
                &episode(HOLLOW, 9, "OtherGroup")
            ),
            20,
            "neither torrent carries the episode or the group"
        );
    }

    #[test]
    fn a_subject_with_nothing_held_follows_every_completed_torrent() {
        let title = episode(COASTAL, 9, "PublicWave");
        let affinity = |torrents: &[Torrent]| {
            Library::new(&ENGINE, torrents).affinity("show|coastal ecology", &title)
        };

        assert_eq!(
            affinity(&[
                torrent(&episode(HOLLOW, 5, "PublicWave"), 1.0),
                torrent(&episode(COASTAL, 3, "OtherGroup"), 1.0),
            ]),
            17,
            "both completed torrents count, claimed or not"
        );
        assert_eq!(
            affinity(&[torrent(&episode(HOLLOW, 5, "PublicWave"), 0.5)]),
            0,
            "a download in progress is no habit"
        );
    }
}
