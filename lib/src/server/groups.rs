//! Folds the listed releases into the subjects and identities the Results
//! page shows.
//!
//! A tracker announces one episode many times, in several resolutions, from
//! several groups, and inside a season pack. A reader picks one copy, so the
//! page lists every release of one identity together, and every identity of
//! one show or film under that subject.
//!
//! The tree keys on the identity the engine files a release under, not on
//! the title. Two releases share a group exactly when the library counts
//! them as one item, so a grab of either one owns the whole group.
//!
//! The input order is the listing order, newest first. It decides which
//! subject leads, and which release leads when the preference lists rank two
//! alike.

use crate::engine::{Engine, Identity, Parsed};
use crate::parser::FieldKind;
use crate::preference::Preferences;

/// One listed release, as the tree reads it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Release<'a> {
    pub(super) parsed: &'a Parsed,

    /// Whether the listing wants this release.
    ///
    /// An unwanted release still takes its place in its group, but never
    /// leads it.
    pub(super) wanted: bool,
}

/// A show or a film, with every identity the listing holds of it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Subject {
    /// The subject as its newest release names it, with `.` and `_` read as
    /// spaces.
    pub(super) label: String,

    /// Highest first, so the latest episode leads and a season pack follows
    /// the episodes of its season.
    pub(super) groups: Vec<Group>,
}

/// One identity, such as an episode, a season pack, or a film year, with
/// every listed release of it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Group {
    /// The identity past its subject, such as `S04E06`, `S01`, or `2024`.
    pub(super) label: String,

    /// Whether the identity is a span over several releases, such as a
    /// season pack.
    pub(super) pack: bool,

    /// Indices into the input, best first.
    ///
    /// A wanted release comes before an unwanted one, and the preference
    /// lists order each part. Releases the lists rank alike keep the input
    /// order.
    pub(super) releases: Vec<usize>,

    /// The first release, when the listing wants it.
    pub(super) preferred: Option<usize>,

    /// Whether the two best releases are both wanted and the stated lists
    /// rank them alike.
    ///
    /// Only the order they arrived in separates such a pair, and the reader
    /// never chose that order. A reader who stated no list asked for no
    /// ranking, so a pair that no list ranks is no tie.
    pub(super) tied: bool,
}

/// A subject while the walk fills it.
struct SubjectDraft<'a> {
    key: String,
    label: String,
    groups: Vec<GroupDraft<'a>>,
}

/// An identity while the walk fills it.
struct GroupDraft<'a> {
    key: String,

    /// The key parts past the subject, which the groups of one subject sort
    /// by.
    order: Vec<Part<'a>>,

    label: String,
    pack: bool,
    releases: Vec<Ranked>,
}

impl GroupDraft<'_> {
    /// Orders the releases best first and settles which one leads.
    fn finish(mut self) -> Group {
        self.releases
            .sort_by(|a, b| (!a.wanted, &a.rank).cmp(&(!b.wanted, &b.rank)));

        let tied = matches!(
            self.releases.as_slice(),
            [first, second, ..] if first.wanted
                && second.wanted
                && !first.rank.is_empty()
                && first.rank == second.rank
        );

        Group {
            label: self.label,
            pack: self.pack,
            preferred: self
                .releases
                .first()
                .filter(|first| first.wanted)
                .map(|first| first.index),
            releases: self.releases.iter().map(|ranked| ranked.index).collect(),
            tied,
        }
    }
}

/// One release of a group, with what ranks it.
struct Ranked {
    index: usize,
    rank: Vec<usize>,
    wanted: bool,
}

/// Where one key part sorts among the parts at its position.
///
/// The variant order is the sort order. A number sorts above an empty part,
/// so an episode sorts above the pack of its season. Numbers compare as
/// numbers, so episode 10 sorts above episode 9.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part<'a> {
    Empty,
    Number(u64),
    Text(&'a str),
}

impl<'a> From<&'a str> for Part<'a> {
    fn from(part: &'a str) -> Self {
        if part.is_empty() {
            return Self::Empty;
        }

        part.parse().map_or(Self::Text(part), Self::Number)
    }
}

/// Folds `releases` into subjects, each holding its identities.
///
/// Subjects follow the input order of their first release. Within one, the
/// identities sort highest first, with numbers compared as numbers.
pub(super) fn tree(
    engine: &Engine,
    preferences: &Preferences,
    releases: &[Release<'_>],
) -> Vec<Subject> {
    let mut drafts: Vec<SubjectDraft<'_>> = Vec::new();

    for (index, release) in releases.iter().enumerate() {
        let parsed = release.parsed;
        let identity = &parsed.identity;

        let subject_key = format!(
            "{}|{}",
            subject_field(identity),
            identity.key.first().map_or("", String::as_str)
        );
        let subject = find_or_push(
            &mut drafts,
            |draft| draft.key == subject_key,
            || SubjectDraft {
                key: subject_key.clone(),
                label: subject_label(parsed),
                groups: Vec::new(),
            },
        );

        let group_key = identity.to_string();
        let group = find_or_push(
            &mut subject.groups,
            |draft| draft.key == group_key,
            || GroupDraft {
                key: group_key.clone(),
                order: past_subject(identity)
                    .iter()
                    .map(|part| Part::from(part.as_str()))
                    .collect(),
                label: group_label(engine, parsed),
                pack: identity.key.last().is_some_and(String::is_empty),
                releases: Vec::new(),
            },
        );

        group.releases.push(Ranked {
            index,
            rank: preferences.rank(engine, parsed),
            wanted: release.wanted,
        });
    }

    drafts
        .into_iter()
        .map(|mut draft| {
            draft.groups.sort_by(|a, b| b.order.cmp(&a.order));

            Subject {
                label: draft.label,
                groups: draft.groups.into_iter().map(GroupDraft::finish).collect(),
            }
        })
        .collect()
}

/// Returns the item of `items` that `found` picks, and pushes the one `make`
/// builds when none matches.
fn find_or_push<T>(
    items: &mut Vec<T>,
    found: impl Fn(&T) -> bool,
    make: impl FnOnce() -> T,
) -> &mut T {
    let at = match items.iter().position(found) {
        Some(at) => at,
        None => {
            items.push(make());
            items.len() - 1
        }
    };

    &mut items[at]
}

/// The name of the field that names the subject, such as `show` or `title`.
///
/// The name takes part in the subject key, so a show and a film that
/// normalize alike stay two subjects. Two parsers that name the field alike
/// fold one show into one subject.
fn subject_field(identity: &Identity) -> &str {
    identity.namespace.split('+').next().unwrap_or_default()
}

/// The subject as `parsed` names it, with `.` and `_` read as spaces.
///
/// The raw value keeps the capitals the release wrote, which the normalized
/// key drops.
fn subject_label(parsed: &Parsed) -> String {
    raw(parsed, subject_field(&parsed.identity))
        .unwrap_or_else(|| parsed.identity.key.first().map_or("", String::as_str))
        .replace(['.', '_'], " ")
}

/// Names an identity past its subject, such as `S04E06`, `S01`, or `2024`.
///
/// A season and an episode read as a release name writes them, and two in a
/// row run together. Every other part reads as the release wrote it, set off
/// with `-`. An empty part is the open end of a span, which the label leaves
/// out.
fn group_label(engine: &Engine, parsed: &Parsed) -> String {
    let parts = past_subject(&parsed.identity);

    // Without the parser nothing says which part is a season, so only the
    // normalized parts remain.
    let Some(parser) = engine.parser(&parsed.parser) else {
        return parts
            .iter()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("-");
    };

    let fields = parser.fields.iter().filter(|field| field.identity).skip(1);
    let mut label = String::new();
    let mut after_numbered = false;

    for (field, part) in fields.zip(parts).filter(|(_, part)| !part.is_empty()) {
        let numbered = matches!(field.kind, FieldKind::Season | FieldKind::Episode);

        if !label.is_empty() && !(numbered && after_numbered) {
            label.push('-');
        }

        match field.kind {
            FieldKind::Season => label.push_str(&format!("S{part:0>2}")),
            FieldKind::Episode => label.push_str(&format!("E{part:0>2}")),
            _ => label.push_str(raw(parsed, &field.name).unwrap_or(part)),
        }

        after_numbered = numbered;
    }

    label
}

/// The key parts after the one that names the subject.
fn past_subject(identity: &Identity) -> &[String] {
    identity.key.get(1..).unwrap_or_default()
}

/// The value `parsed` read for the field `name`, as the release wrote it.
fn raw<'a>(parsed: &'a Parsed, name: &str) -> Option<&'a str> {
    parsed
        .values
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Group, Release, Subject, tree};
    use crate::engine::Parsed;
    use crate::preference::Preferences;
    use crate::search::fixture::ENGINE;

    const HOLLOW_1080: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    /// The same episode from another group.
    const HOLLOW_OTHER_GROUP: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
    const HOLLOW_NEXT: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    /// The first season as one release, named after its folder.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S01.1080p.Broadcast.AAC.Stereo.H.264-PublicWave";
    /// An episode of the season the pack carries.
    const HOLLOW_S01E03: &str =
        "The.Hollow.Meridian.S01E03.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const FILM_UHD: &str = "Coastal.Drift.2024.2160p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";

    fn parsed(titles: &[&str]) -> Vec<Parsed> {
        titles
            .iter()
            .map(|title| ENGINE.parse(title).expect("a fixture search claims it"))
            .collect()
    }

    fn wanted(parsed: &[Parsed]) -> Vec<Release<'_>> {
        parsed
            .iter()
            .map(|parsed| Release {
                parsed,
                wanted: true,
            })
            .collect()
    }

    fn prefers(field: &str, values: &[&str]) -> Preferences {
        Preferences::new(BTreeMap::from([(
            field.to_owned(),
            values.iter().map(|one| (*one).to_owned()).collect(),
        )]))
    }

    /// The tree of `titles`, every one of them wanted.
    fn tree_of(preferences: &Preferences, titles: &[&str]) -> Vec<Subject> {
        tree(&ENGINE, preferences, &wanted(&parsed(titles)))
    }

    fn subject(label: &str, groups: Vec<Group>) -> Subject {
        Subject {
            label: label.to_owned(),
            groups,
        }
    }

    /// A group that is no pack, led by its first release, with no tie.
    fn group(label: &str, releases: &[usize]) -> Group {
        Group {
            label: label.to_owned(),
            pack: false,
            releases: releases.to_vec(),
            preferred: releases.first().copied(),
            tied: false,
        }
    }

    #[test]
    fn one_show_holds_its_episodes_high_first() {
        assert_eq!(
            tree_of(
                &Preferences::default(),
                &[HOLLOW_1080, HOLLOW_OTHER_GROUP, HOLLOW_NEXT]
            ),
            vec![subject(
                "The Hollow Meridian",
                vec![group("S04E07", &[2]), group("S04E06", &[0, 1])],
            )],
        );
    }

    #[test]
    fn a_season_pack_follows_the_episodes_of_its_season() {
        assert_eq!(
            tree_of(&Preferences::default(), &[HOLLOW_PACK, HOLLOW_S01E03]),
            vec![subject(
                "The Hollow Meridian",
                vec![
                    group("S01E03", &[1]),
                    Group {
                        pack: true,
                        ..group("S01", &[0])
                    },
                ],
            )],
            "the pack arrived first, and the episode it carries still leads"
        );
    }

    #[test]
    fn a_stated_list_leads_with_the_best() {
        assert_eq!(
            tree_of(
                &prefers("publisher", &["OtherGroup", "PublicWave"]),
                &[HOLLOW_1080, HOLLOW_OTHER_GROUP]
            ),
            vec![subject(
                "The Hollow Meridian",
                vec![group("S04E06", &[1, 0])]
            )],
            "the older release comes from the group the list names first"
        );
    }

    #[test]
    fn two_releases_a_list_ranks_alike_tie() {
        assert_eq!(
            tree_of(
                &prefers("resolution", &["1080p"]),
                &[HOLLOW_1080, HOLLOW_OTHER_GROUP]
            ),
            vec![subject(
                "The Hollow Meridian",
                vec![Group {
                    tied: true,
                    ..group("S04E06", &[0, 1])
                }],
            )],
            "both read 1080p, so only their order separates them"
        );
    }

    #[test]
    fn no_stated_list_ties_nothing() {
        assert_eq!(
            tree_of(&Preferences::default(), &[HOLLOW_1080, HOLLOW_OTHER_GROUP]),
            vec![subject(
                "The Hollow Meridian",
                vec![group("S04E06", &[0, 1])]
            )],
            "a reader who stated no list asked for no ranking"
        );
    }

    #[test]
    fn an_unwanted_release_never_leads() {
        let claimed = parsed(&[HOLLOW_1080, HOLLOW_OTHER_GROUP]);
        let releases = |first: bool, second: bool| {
            vec![
                Release {
                    parsed: &claimed[0],
                    wanted: first,
                },
                Release {
                    parsed: &claimed[1],
                    wanted: second,
                },
            ]
        };

        assert_eq!(
            tree(&ENGINE, &Preferences::default(), &releases(false, true)),
            vec![subject(
                "The Hollow Meridian",
                vec![group("S04E06", &[1, 0])]
            )],
            "the wanted release leads although it is older"
        );
        assert_eq!(
            tree(&ENGINE, &Preferences::default(), &releases(false, false)),
            vec![subject(
                "The Hollow Meridian",
                vec![Group {
                    preferred: None,
                    ..group("S04E06", &[0, 1])
                }],
            )],
            "no release is wanted, so none is preferred"
        );
    }

    #[test]
    fn subjects_follow_their_newest_release() {
        let film = |at| subject("Coastal Drift", vec![group("2024", &[at])]);
        let show = |at| subject("The Hollow Meridian", vec![group("S04E06", &[at])]);

        assert_eq!(
            tree_of(&Preferences::default(), &[FILM, HOLLOW_1080]),
            vec![film(0), show(1)],
        );
        assert_eq!(
            tree_of(&Preferences::default(), &[HOLLOW_1080, FILM]),
            vec![show(0), film(1)],
        );
    }

    #[test]
    fn a_film_is_labeled_by_its_year() {
        assert_eq!(
            tree_of(&Preferences::default(), &[FILM, FILM_UHD]),
            vec![subject("Coastal Drift", vec![group("2024", &[0, 1])])],
        );
    }
}
