//! What the client holds that a ruleset claims, and which grab put it there.
//!
//! A torrent and a grab meet on the identity their names parse to. The client
//! keeps the release name and the store records no hash, so that identity is
//! the one link the two share. It is the same key the wanted list already
//! sorts titles by.
//!
//! A torrent no ruleset claims is left out, as the scan leaves it out of the
//! library.

use std::cmp::Reverse;
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::rules::Engine;
use crate::store::grabs::Accepted;
use crate::torrent::Torrent;

/// One torrent the client holds, with the grab that moved it there.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code, reason = "the held torrents the client page lists")]
pub(super) struct Held {
    pub(super) torrent: Torrent,

    /// The ruleset that claimed the name, rather than the template behind it,
    /// so the page names the rule the reader wrote.
    pub(super) ruleset: String,

    /// When the client accepted the grab, or nothing for a torrent it held
    /// before the store recorded any.
    pub(super) grabbed_at: Option<DateTime<Utc>>,
}

/// Pairs each claimed torrent with the grab that moved it.
///
/// The newest grab comes first, then every torrent with no grab in the order
/// the client reported them.
#[allow(dead_code, reason = "the held torrents the client page lists")]
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
        .filter_map(|torrent| {
            let parsed = engine.parse(&torrent.name)?;

            Some(Held {
                grabbed_at: grabbed.get(&parsed.identity.to_string()).copied(),
                ruleset: parsed.ruleset,
                torrent,
            })
        })
        .collect();

    // `None` sorts last under `Reverse`, and the sort is stable, so an
    // ungrabbed torrent keeps the client's own order behind the grabbed ones.
    held.sort_by_key(|held| Reverse(held.grabbed_at));

    held
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::{Held, held};
    use crate::ruleset::fixture::ENGINE;
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

    const NONSENSE: &str = "just some words with no structure at all";

    const CLAIMANT: &str = "series-hollow-meridian";

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    fn torrent(id: &str, name: &str) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
        }
    }

    fn accepted(title: &str, day: u32) -> Accepted {
        Accepted {
            title: title.to_owned(),
            at: at(day),
        }
    }

    fn held_of(torrents: Vec<Torrent>, accepted: &[Accepted]) -> Vec<Held> {
        held(&ENGINE, torrents, accepted)
    }

    #[test]
    fn unclaimed_torrent_is_left_out() {
        assert_eq!(
            held_of(vec![torrent("t1", NONSENSE)], &[]),
            Vec::new(),
            "no ruleset claims the name, so no rule put it there"
        );
    }

    #[test]
    fn torrent_carries_the_grab_of_its_identity() {
        assert_eq!(
            held_of(vec![torrent("t1", HOLLOW_E06)], &[accepted(HOLLOW_E06, 2)]),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06),
                ruleset: CLAIMANT.to_owned(),
                grabbed_at: Some(at(2)),
            }],
            "the two names parse to one identity, which is what pairs them"
        );
    }

    #[test]
    fn torrent_with_no_grab_carries_no_time() {
        assert_eq!(
            held_of(vec![torrent("t1", HOLLOW_E06)], &[]),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06),
                ruleset: CLAIMANT.to_owned(),
                grabbed_at: None,
            }],
            "the client held it before the store recorded any grab"
        );
    }

    #[test]
    fn latest_grab_of_one_identity_wins() {
        assert_eq!(
            held_of(
                vec![torrent("t1", HOLLOW_E06)],
                &[accepted(HOLLOW_E06, 2), accepted(HOLLOW_E06_OTHER, 3)],
            ),
            vec![Held {
                torrent: torrent("t1", HOLLOW_E06),
                ruleset: CLAIMANT.to_owned(),
                grabbed_at: Some(at(3)),
            }],
            "two qualities are one identity, and the last grab moved what is held"
        );
    }

    #[test]
    fn grabbed_torrents_sort_newest_first_then_the_rest() {
        assert_eq!(
            held_of(
                vec![
                    torrent("t1", HOLLOW_E07),
                    torrent("t2", HOLLOW_E06),
                    torrent("t3", HOLLOW_E08),
                ],
                &[accepted(HOLLOW_E06, 2), accepted(HOLLOW_E08, 4)],
            ),
            vec![
                Held {
                    torrent: torrent("t3", HOLLOW_E08),
                    ruleset: CLAIMANT.to_owned(),
                    grabbed_at: Some(at(4)),
                },
                Held {
                    torrent: torrent("t2", HOLLOW_E06),
                    ruleset: CLAIMANT.to_owned(),
                    grabbed_at: Some(at(2)),
                },
                Held {
                    torrent: torrent("t1", HOLLOW_E07),
                    ruleset: CLAIMANT.to_owned(),
                    grabbed_at: None,
                },
            ],
            "the newest grab leads, and an ungrabbed torrent follows them all"
        );
    }
}
