//! Names the episodes a show's numbering implies that no search claims.
//!
//! An announced S04E10 beside a held S04E01 and S04E02 says that S04E03
//! through S04E09 exist. When no search claims a copy of them, from the feed
//! or from the client, the home page lists each as a greyed row of the card.
//! The reader then relaxes a search or looks for them elsewhere.

use std::collections::{BTreeMap, BTreeSet};

use crate::engine::{Engine, Parsed};
use crate::parser::FieldKind;

/// Where a release sits in its show's numbering, as season and episode.
///
/// [`None`] marks a part the parser does not number. It sorts below any
/// number, as a pack sorts below the episodes of its season in
/// [`groups::tree`](super::groups::tree).
pub(super) type Numbering = (Option<u64>, Option<u64>);

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

    /// Returns each episode of the run as a run of one, highest first, so
    /// [`Self::label`] names each as `S04E03`.
    pub(super) fn episodes(&self) -> impl Iterator<Item = Gap> {
        let season = self.season;

        (self.first..=self.last).rev().map(move |number| Gap {
            season,
            first: number,
            last: number,
        })
    }
}

/// Where one release sits in the numbering of its show.
pub(super) struct Slot {
    subject: String,
    season: Option<u64>,

    /// [`None`] for a season pack, whose key leaves the episode empty.
    episode: Option<u64>,
}

impl Slot {
    pub(super) fn numbering(&self) -> Numbering {
        (self.season, self.episode)
    }
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

/// Places each missing episode of `runs` among `rows`, highest first.
///
/// The subject card lists a missing episode as a row beside the ones around
/// it, so it reads where the reader expects it. `rows` read highest first,
/// as [`groups::tree`](super::groups::tree) orders them, and `runs` read low
/// to high, as [`gaps`] returns them. Each missing episode lands before the
/// first row whose numbering sorts below it, and `missing` builds its row
/// from its label.
///
/// A row with no numbering, such as a film year or a dated episode, stops no
/// missing episode and keeps its place. The missing episodes below every
/// numbered row follow the last row.
pub(super) fn interleave<T>(
    rows: Vec<(Option<Numbering>, T)>,
    runs: &[Gap],
    missing: impl Fn(String) -> T,
) -> Vec<T> {
    let mut gaps = runs.iter().rev().flat_map(Gap::episodes).peekable();
    let mut placed = Vec::with_capacity(rows.len());

    for (numbering, row) in rows {
        if let Some(numbering) = numbering {
            while let Some(gap) = gaps.next_if(|gap| (gap.season, Some(gap.first)) > numbering) {
                placed.push(missing(gap.label()));
            }
        }

        placed.push(row);
    }

    placed.extend(gaps.map(|gap| missing(gap.label())));
    placed
}

/// Places `parsed` in the numbering of its show.
///
/// Answers [`None`] when its parser has no episode among its identity
/// fields, as for a film or a dated show. Such a release implies no other.
pub(super) fn slot(engine: &Engine, parsed: &Parsed) -> Option<Slot> {
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

    use super::{Gap, Numbering, gaps, interleave};
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

    /// One numbered row of the card, labeled as a release name writes it.
    fn row(season: u64, number: u64) -> (Option<Numbering>, String) {
        (
            Some((Some(season), Some(number))),
            format!("S{season:0>2}E{number:0>2}"),
        )
    }

    fn interleaved(rows: Vec<(Option<Numbering>, String)>, runs: &[Gap]) -> Vec<String> {
        interleave(rows, runs, |label| label)
    }

    #[test]
    fn a_missing_episode_sits_between_the_rows_around_it() {
        assert_eq!(
            interleaved(vec![row(4, 10), row(4, 2)], &[run(4, 3, 9)]),
            [
                "S04E10", "S04E09", "S04E08", "S04E07", "S04E06", "S04E05", "S04E04", "S04E03",
                "S04E02"
            ],
        );
    }

    #[test]
    fn a_later_season_leads_the_missing_episodes_of_an_earlier_one() {
        assert_eq!(
            interleaved(vec![row(5, 1), row(4, 10)], &[run(4, 3, 9)]),
            [
                "S05E01", "S04E10", "S04E09", "S04E08", "S04E07", "S04E06", "S04E05", "S04E04",
                "S04E03"
            ],
        );
    }

    #[test]
    fn a_run_below_every_row_follows_the_last_row() {
        assert_eq!(
            interleaved(vec![row(4, 10)], &[run(4, 3, 5)]),
            ["S04E10", "S04E05", "S04E04", "S04E03"],
        );
    }

    #[test]
    fn a_row_with_no_numbering_keeps_its_place() {
        assert_eq!(
            interleaved(
                vec![row(4, 10), (None, "2024".to_owned()), row(4, 8)],
                &[run(4, 9, 9)]
            ),
            ["S04E10", "2024", "S04E09", "S04E08"],
            "the film year stops no missing episode, so S04E09 waits for the next numbered row"
        );
    }

    #[test]
    fn episodes_name_each_episode_of_a_run_highest_first() {
        assert_eq!(
            run(4, 3, 5)
                .episodes()
                .map(|gap| gap.label())
                .collect::<Vec<_>>(),
            ["S04E05", "S04E04", "S04E03"],
        );
    }
}
