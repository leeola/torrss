//! What the client holds, and which grab put each torrent there.
//!
//! A torrent and a grab meet on the identity their names parse to. The client
//! keeps the release name and the store records no hash, so that identity is
//! the one link the two share. It is the same key the wanted list already
//! sorts titles by.
//!
//! A torrent no search matches is listed too, with no parse of its own. A
//! reader starts a search from such a name, so hiding it hides the rows
//! most worth acting on.

use std::cmp::Reverse;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::engine::{Engine, Parsed};
use crate::store::grabs::Accepted;
use crate::torrent::Torrent;

/// One torrent the client holds, with the grab that moved it there.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Held {
    pub(super) torrent: Torrent,

    /// What the matched search made of the name, or nothing when no search
    /// matches it.
    ///
    /// Its search is the one that claimed the name, rather than the parser
    /// behind it, so the page names the rule the reader wrote.
    ///
    /// It carries the whole parse rather than rendered values, because
    /// `Standing` on the home page does the same and the handler resolves the
    /// values at render.
    pub(super) parsed: Option<Parsed>,

    /// When the client accepted the grab, or nothing for a torrent it held
    /// before the store recorded any.
    pub(super) grabbed_at: Option<DateTime<Utc>>,
}

/// Pairs each torrent with the grab that moved it, when one did.
///
/// Every torrent a grab moved leads, then the rest a search matches, then
/// the rest. Each group orders by the time the client added the torrent,
/// newest first.
///
/// A name that does not parse has no identity, so no grab can name it and
/// its time is always absent.
pub(super) fn held(engine: &Engine, torrents: Vec<Torrent>, accepted: &[Accepted]) -> Vec<Held> {
    let mut grabbed: HashMap<String, DateTime<Utc>> = HashMap::new();

    for grab in accepted {
        let Some(parsed) = engine.parse(&grab.title) else {
            continue;
        };

        // Two qualities of one episode are one identity, and the last grab
        // the client accepted is the one that moved what it holds now.
        grabbed
            .entry(parsed.identity.to_string())
            .and_modify(|kept| *kept = (*kept).max(grab.at))
            .or_insert(grab.at);
    }

    let mut held: Vec<Held> = torrents
        .into_iter()
        .map(|torrent| {
            let parsed = engine.parse(&torrent.name);

            Held {
                // Above `parsed`, because the lookup borrows the parse that
                // the next field moves.
                grabbed_at: parsed
                    .as_ref()
                    .and_then(|parsed| grabbed.get(&parsed.identity.to_string()).copied()),
                parsed,
                torrent,
            }
        })
        .collect();

    // `false` sorts before `true`, so a grabbed torrent leads whatever its
    // date and a matched one leads an unmatched one. `Reverse(None)` sorts
    // last and the sort is stable, so a torrent the client gave no added time
    // for trails its own group in the client's order.
    held.sort_by_key(|held| {
        (
            held.grabbed_at.is_none(),
            held.parsed.is_none(),
            Reverse(held.torrent.added_at),
        )
    });

    held
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::{Held, held};
    use crate::engine::Parsed;
    use crate::search::fixture::ENGINE;
    use crate::store::grabs::Accepted;
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    const HOLLOW_E06: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";

    /// The same episode from another group, which is the same identity.
    const HOLLOW_E06_OTHER: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";

    const HOLLOW_E07: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const HOLLOW_E08: &str =
        "The.Hollow.Meridian.S04E08.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const HOLLOW_E09: &str =
        "The.Hollow.Meridian.S04E09.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";

    const NONSENSE: &str = "just some words with no structure at all";

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    fn torrent(id: &str, name: &str, day: u32) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
            added_at: Some(at(day)),
        }
    }

    fn accepted(title: &str, day: u32) -> Accepted {
        Accepted {
            title: title.to_owned(),
            at: at(day),
        }
    }

    /// What the engine makes of `name`, which every matched row carries.
    fn parsed(name: &str) -> Option<Parsed> {
        Some(ENGINE.parse(name).expect("matched"))
    }

    fn held_of(torrents: Vec<Torrent>, accepted: &[Accepted]) -> Vec<Held> {
        held(&ENGINE, torrents, accepted)
    }

    #[test]
    fn unmatched_torrent_trails_the_matched() {
        assert_eq!(
            held_of(
                vec![
                    torrent("t1", NONSENSE, 9),
                    torrent("t2", HOLLOW_E07, 8),
                    torrent("t3", HOLLOW_E06, 7),
                ],
                &[accepted(HOLLOW_E06, 2)],
            ),
            vec![
                Held {
                    torrent: torrent("t3", HOLLOW_E06, 7),
                    parsed: parsed(HOLLOW_E06),
                    grabbed_at: Some(at(2)),
                },
                Held {
                    torrent: torrent("t2", HOLLOW_E07, 8),
                    parsed: parsed(HOLLOW_E07),
                    grabbed_at: None,
                },
                Held {
                    torrent: torrent("t1", NONSENSE, 9),
                    parsed: None,
                    grabbed_at: None,
                },
            ],
            "the newest added trails both, because a match outranks the date"
        );
    }

    #[test]
    fn torrent_carries_the_grab_of_its_identity() {
        assert_eq!(
            held_of(
                vec![torrent("t1", HOLLOW_E06, 1)],
                &[accepted(HOLLOW_E06, 2)]
            ),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06, 1),
                parsed: parsed(HOLLOW_E06),
                grabbed_at: Some(at(2)),
            }],
            "the two names parse to one identity, which is what pairs them"
        );
    }

    #[test]
    fn torrent_with_no_grab_carries_no_time() {
        assert_eq!(
            held_of(vec![torrent("t1", HOLLOW_E06, 1)], &[]),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06, 1),
                parsed: parsed(HOLLOW_E06),
                grabbed_at: None,
            }],
            "the client held it before the store recorded any grab"
        );
    }

    #[test]
    fn latest_grab_of_one_identity_wins() {
        assert_eq!(
            held_of(
                vec![torrent("t1", HOLLOW_E06, 1)],
                &[accepted(HOLLOW_E06, 2), accepted(HOLLOW_E06_OTHER, 3)],
            ),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06, 1),
                parsed: parsed(HOLLOW_E06),
                grabbed_at: Some(at(3)),
            }],
            "two qualities are one identity, and the last grab moved what is held"
        );
    }

    #[test]
    fn grabbed_torrents_lead_and_each_group_sorts_by_added_time() {
        assert_eq!(
            held_of(
                vec![
                    torrent("t1", HOLLOW_E07, 5),
                    torrent("t2", HOLLOW_E06, 4),
                    torrent("t3", HOLLOW_E08, 3),
                    torrent("t4", HOLLOW_E09, 6),
                ],
                &[accepted(HOLLOW_E06, 2), accepted(HOLLOW_E08, 4)],
            ),
            vec![
                Held {
                    torrent: torrent("t2", HOLLOW_E06, 4),
                    parsed: parsed(HOLLOW_E06),
                    grabbed_at: Some(at(2)),
                },
                Held {
                    torrent: torrent("t3", HOLLOW_E08, 3),
                    parsed: parsed(HOLLOW_E08),
                    grabbed_at: Some(at(4)),
                },
                Held {
                    torrent: torrent("t4", HOLLOW_E09, 6),
                    parsed: parsed(HOLLOW_E09),
                    grabbed_at: None,
                },
                Held {
                    torrent: torrent("t1", HOLLOW_E07, 5),
                    parsed: parsed(HOLLOW_E07),
                    grabbed_at: None,
                },
            ],
            "a grab leads whatever its date, and the added time orders each group"
        );
    }
}
