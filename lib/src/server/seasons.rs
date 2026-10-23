//! What the client holds of one subject, placed in its seasons.
//!
//! A Results card names a show by what the feed announces. The subject page
//! answers what the reader already has of it, so it lists the held torrents
//! of the show season by season.

use super::held::Held;
use super::{gaps, groups};
use crate::engine::Engine;

/// One season of a subject, with the torrents the client holds of it.
#[derive(Debug, PartialEq)]
pub(super) struct Season {
    /// [`None`] for a parser that numbers no season, such as a film's or a
    /// talk series'.
    pub(super) number: Option<u64>,

    /// A pack first, then the episodes low to high.
    ///
    /// Two torrents of one episode both list, in the order
    /// [`held`](super::held::held) gives them.
    pub(super) rows: Vec<Row>,
}

/// One held torrent, as its season lists it.
#[derive(Debug, PartialEq)]
pub(super) struct Row {
    /// The identity past its subject, such as `S04E06`, `S04`, or `2024`, as
    /// [`group_label`](super::groups::group_label) renders it.
    pub(super) label: String,

    /// Whether the identity is a span over several releases, such as a
    /// season pack.
    pub(super) pack: bool,

    pub(super) held: Held,
}

/// Places what the client holds of `subject` into its seasons, lowest first.
///
/// `subject` is an [`Identity::subject`](crate::engine::Identity::subject)
/// key. A name that no search claims parses to no subject, so it never
/// lists here.
pub(super) fn seasons(engine: &Engine, held: Vec<Held>, subject: &str) -> Vec<Season> {
    let mut placed: Vec<(Option<u64>, Option<u64>, Row)> = held
        .into_iter()
        .filter_map(|held| {
            let parsed = held.parsed.as_ref()?;
            if parsed.identity.subject() != subject {
                return None;
            }

            let (season, episode) =
                gaps::slot(engine, parsed).map_or((None, None), |slot| (slot.season, slot.episode));

            Some((
                season,
                episode,
                Row {
                    label: groups::group_label(engine, parsed),
                    pack: parsed.identity.is_span(),
                    held,
                },
            ))
        })
        .collect();

    // The sort is stable, so two torrents of one episode keep the held
    // order. `None` sorts first, so a pack leads the episodes of its season.
    placed.sort_by_key(|(season, episode, _)| (*season, *episode));

    let mut seasons: Vec<Season> = Vec::new();
    for (season, _, row) in placed {
        match seasons.last_mut() {
            Some(current) if current.number == season => current.rows.push(row),
            _ => seasons.push(Season {
                number: season,
                rows: vec![row],
            }),
        }
    }

    seasons
}

/// The subject as a held release writes it, with its capitals, or nothing
/// when the client holds nothing of it.
pub(super) fn label(seasons: &[Season]) -> Option<String> {
    seasons
        .first()?
        .rows
        .first()?
        .held
        .parsed
        .as_ref()
        .map(groups::subject_label)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{Row, Season, label, seasons};
    use crate::search::fixture::ENGINE;
    use crate::server::held::{self, Held};
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    const HOLLOW: &str = "show|the hollow meridian";

    const S03E01: &str =
        "The.Hollow.Meridian.S03E01.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const S03E02: &str =
        "The.Hollow.Meridian.S03E02.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const S04E07: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";

    /// The same episode from another group, which is the same identity.
    const S04E07_OTHER: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";

    /// The fourth season as one release, named after its folder.
    const S04_PACK: &str = "The.Hollow.Meridian.S04.1080p.Broadcast.AAC.Stereo.H.264-PublicWave";

    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const NONSENSE: &str = "just some words with no structure at all";

    /// A torrent the client added on `day`, so a later day lists first.
    fn torrent(id: &str, name: &str, day: u32) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
            added_at: Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0).single(),
        }
    }

    fn held_of(torrents: Vec<Torrent>) -> Vec<Held> {
        held::held(&ENGINE, torrents, &[])
    }

    fn row(label: &str, pack: bool, torrent: Torrent) -> Row {
        Row {
            label: label.to_owned(),
            pack,
            held: Held {
                parsed: ENGINE.parse(&torrent.name),
                torrent,
                grabbed_at: None,
            },
        }
    }

    #[test]
    fn seasons_ascend_and_a_pack_leads_its_season() {
        assert_eq!(
            seasons(
                &ENGINE,
                held_of(vec![
                    torrent("t1", S04E07, 4),
                    torrent("t2", S03E02, 3),
                    torrent("t3", S04_PACK, 2),
                    torrent("t4", S03E01, 1),
                ]),
                HOLLOW,
            ),
            vec![
                Season {
                    number: Some(3),
                    rows: vec![
                        row("S03E01", false, torrent("t4", S03E01, 1)),
                        row("S03E02", false, torrent("t2", S03E02, 3)),
                    ],
                },
                Season {
                    number: Some(4),
                    rows: vec![
                        row("S04", true, torrent("t3", S04_PACK, 2)),
                        row("S04E07", false, torrent("t1", S04E07, 4)),
                    ],
                },
            ],
            "the lower season leads, and a pack opens the season it carries"
        );
    }

    #[test]
    fn two_copies_of_one_episode_keep_the_held_order() {
        assert_eq!(
            seasons(
                &ENGINE,
                held_of(vec![
                    torrent("t1", S04E07, 1),
                    torrent("t2", S04E07_OTHER, 2),
                ]),
                HOLLOW,
            ),
            vec![Season {
                number: Some(4),
                rows: vec![
                    row("S04E07", false, torrent("t2", S04E07_OTHER, 2)),
                    row("S04E07", false, torrent("t1", S04E07, 1)),
                ],
            }],
            "the copy the client added later lists first, as the held order has it"
        );
    }

    #[test]
    fn another_subject_and_an_unmatched_name_are_left_out() {
        assert_eq!(
            seasons(
                &ENGINE,
                held_of(vec![
                    torrent("t1", S04E07, 3),
                    torrent("t2", FILM, 2),
                    torrent("t3", NONSENSE, 1),
                ]),
                HOLLOW,
            ),
            vec![Season {
                number: Some(4),
                rows: vec![row("S04E07", false, torrent("t1", S04E07, 3))],
            }],
            "the film is another subject, and the nonsense name parses to none"
        );
    }

    #[test]
    fn a_film_lists_under_no_season() {
        assert_eq!(
            seasons(
                &ENGINE,
                held_of(vec![torrent("t1", FILM, 1)]),
                "title|coastal drift"
            ),
            vec![Season {
                number: None,
                rows: vec![row("2024", false, torrent("t1", FILM, 1))],
            }],
            "a film's parser numbers no season"
        );
    }

    #[test]
    fn label_names_the_subject_as_a_held_release_writes_it() {
        assert_eq!(
            label(&seasons(
                &ENGINE,
                held_of(vec![torrent("t1", S04E07, 1)]),
                HOLLOW
            )),
            Some("The Hollow Meridian".to_owned())
        );
        assert_eq!(label(&[]), None, "the client holds nothing of the subject");
    }
}
