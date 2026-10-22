//! Names the episodes a show's numbering implies that no search claims.
//!
//! An announced S04E10 beside a held S04E01 and S04E02 says that S04E03
//! through S04E09 exist. When no search claims a copy of them, from the feed
//! or from the client, the Results page names them. The reader then relaxes a
//! search or looks for them elsewhere.

use std::collections::{BTreeMap, BTreeSet};

use crate::engine::{Engine, Parsed};
use crate::parser::FieldKind;

/// One run of consecutive missing episodes of a season, from `first` to
/// `last` inclusive.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Gap {
    /// [`None`] for a show whose parser numbers episodes with no season.
    pub(super) season: Option<u64>,

    pub(super) first: u64,
    pub(super) last: u64,
}

impl Gap {
    /// Names the run by its ends as a release name writes them, such as
    /// `S04E03` and `E09` joined by an en dash.
    ///
    /// A run of one episode reads as that episode alone, such as `S04E05`.
    pub(super) fn label(&self) -> String {
        let season = self
            .season
            .map(|season| format!("S{season:0>2}"))
            .unwrap_or_default();

        if self.first == self.last {
            format!("{season}E{:0>2}", self.first)
        } else {
            format!("{season}E{:0>2}\u{2013}E{:0>2}", self.first, self.last)
        }
    }
}

/// Where one release sits in the numbering of its show.
struct Slot {
    subject: String,
    season: Option<u64>,

    /// [`None`] for a season pack, whose key leaves the episode empty.
    episode: Option<u64>,
}

/// What the listing and the client carry of one season.
#[derive(Default)]
struct Season {
    episodes: BTreeSet<u64>,

    /// Whether a season pack carries the season, which leaves no episode of
    /// it missing.
    packed: bool,
}

/// Returns the runs of missing episodes of each subject, keyed by its
/// [`Identity::subject`](crate::engine::Identity::subject) key.
///
/// A run holds each episode of a season below the highest known one that
/// neither `announced` nor `held` carries. The runs of a subject read low to
/// high, and a subject with no run has no entry.
///
/// A season pack, announced or held, covers its whole season, because the
/// reader grabs or holds every episode through it.
///
/// `announced` and `held` are what searches claim, so a copy that no search
/// wants fills no gap. `announced` is what the feed serves, so a caller leaves
/// out a release the tracker deleted.
pub(super) fn gaps<'a>(
    engine: &Engine,
    announced: impl IntoIterator<Item = &'a Parsed>,
    held: &[Parsed],
) -> BTreeMap<String, Vec<Gap>> {
    let announced = announced
        .into_iter()
        .filter_map(|parsed| slot(engine, parsed));
    let held = held.iter().filter_map(|parsed| slot(engine, parsed));

    // Keyed by subject and then season, so the runs of one subject come out
    // lowest season first.
    let mut seasons: BTreeMap<(String, Option<u64>), Season> = BTreeMap::new();
    for placed in announced.chain(held) {
        let known = seasons.entry((placed.subject, placed.season)).or_default();

        match placed.episode {
            Some(episode) => {
                known.episodes.insert(episode);
            }
            None => known.packed = true,
        }
    }

    let mut gaps: BTreeMap<String, Vec<Gap>> = BTreeMap::new();
    for ((subject, season), known) in seasons {
        if known.packed {
            continue;
        }

        let runs = missing(season, &known.episodes);
        if !runs.is_empty() {
            gaps.entry(subject).or_default().extend(runs);
        }
    }

    gaps
}

/// Places `parsed` in the numbering of its show.
///
/// Answers [`None`] when its parser has no episode among its identity
/// fields, as for a film or a dated show. Such a release implies no other.
fn slot(engine: &Engine, parsed: &Parsed) -> Option<Slot> {
    let parser = engine.parser(&parsed.parser)?;
    let part = |kind: FieldKind| {
        parser
            .fields
            .iter()
            .filter(|field| field.identity)
            .zip(&parsed.identity.key)
            .find_map(|(field, part)| (field.kind == kind).then_some(part))
    };

    let episode = part(FieldKind::Episode)?;

    Some(Slot {
        subject: parsed.identity.subject(),
        season: part(FieldKind::Season).and_then(|season| season.parse().ok()),
        episode: episode.parse().ok(),
    })
}

/// Returns the runs of numbers that `episodes` skips below its highest one,
/// lowest first.
///
/// Numbering starts at 1, so an episode 0 never opens a run.
fn missing(season: Option<u64>, episodes: &BTreeSet<u64>) -> Vec<Gap> {
    let mut runs = Vec::new();
    let mut below = 0;

    for &episode in episodes {
        if episode > below + 1 {
            runs.push(Gap {
                season,
                first: below + 1,
                last: episode - 1,
            });
        }

        below = episode;
    }

    runs
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Gap, gaps};
    use crate::engine::Parsed;
    use crate::search::fixture::ENGINE;

    /// What every invented Hollow Meridian name carries past its number.
    const TAIL: &str = ".1080p.Broadcast.AAC.Stereo.H.264-PublicWave";
    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";

    const HOLLOW: &str = "show|the hollow meridian";

    fn episode(season: u64, number: u64) -> String {
        format!("The.Hollow.Meridian.S{season:0>2}E{number:0>2}{TAIL}.mkv")
    }

    fn pack(season: u64) -> String {
        format!("The.Hollow.Meridian.S{season:0>2}{TAIL}")
    }

    fn parsed(titles: &[&str]) -> Vec<Parsed> {
        titles
            .iter()
            .map(|title| ENGINE.parse(title).expect("a fixture search claims it"))
            .collect()
    }

    fn gaps_of(announced: &[&str], held: &[&str]) -> BTreeMap<String, Vec<Gap>> {
        gaps(&ENGINE, &parsed(announced), &parsed(held))
    }

    fn run(season: u64, first: u64, last: u64) -> Gap {
        Gap {
            season: Some(season),
            first,
            last,
        }
    }

    fn hollow(runs: Vec<Gap>) -> BTreeMap<String, Vec<Gap>> {
        BTreeMap::from([(HOLLOW.to_owned(), runs)])
    }

    #[test]
    fn an_announced_episode_implies_the_ones_below_it() {
        assert_eq!(
            gaps_of(&[&episode(4, 10)], &[&episode(4, 1), &episode(4, 2)]),
            hollow(vec![run(4, 3, 9)]),
        );
    }

    #[test]
    fn a_held_episode_splits_a_run() {
        assert_eq!(
            gaps_of(&[&episode(4, 10)], &[&episode(4, 1), &episode(4, 5)]),
            hollow(vec![run(4, 2, 4), run(4, 6, 9)]),
        );
    }

    #[test]
    fn a_pack_covers_its_whole_season() {
        assert_eq!(
            gaps_of(&[&episode(4, 10)], &[&pack(4)]),
            BTreeMap::new(),
            "a held pack"
        );
        assert_eq!(
            gaps_of(&[&pack(4), &episode(4, 10)], &[]),
            BTreeMap::new(),
            "an announced pack"
        );
    }

    #[test]
    fn each_season_counts_on_its_own() {
        assert_eq!(
            gaps_of(&[&episode(4, 2)], &[&episode(3, 1)]),
            hollow(vec![run(4, 1, 1)]),
            "the held S03E01 fills nothing in season 4"
        );
    }

    #[test]
    fn an_unbroken_run_has_no_entry() {
        assert_eq!(
            gaps_of(&[&episode(4, 1), &episode(4, 2), &episode(4, 3)], &[]),
            BTreeMap::new(),
        );
    }

    #[test]
    fn a_film_has_no_entry() {
        assert_eq!(gaps_of(&[FILM], &[]), BTreeMap::new());
    }

    #[test]
    fn a_label_names_the_ends_of_a_run() {
        assert_eq!(
            [
                run(4, 3, 9).label(),
                run(4, 5, 5).label(),
                Gap {
                    season: None,
                    first: 3,
                    last: 9
                }
                .label(),
            ],
            ["S04E03\u{2013}E09", "S04E05", "E03\u{2013}E09"],
        );
    }
}
