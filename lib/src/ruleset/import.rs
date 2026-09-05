//! Reading the torrents a client holds into the rulesets they imply.
//!
//! A client full of one subject is the reader saying they follow it. This
//! reads that statement into a ruleset the reader writes by hand otherwise.
//! The ruleset names the subject, and every other field those torrents agree
//! on.
//!
//! The grouping is by the parser's own subject rather than by a field called
//! `show`, because a parser names its subject as it likes. Only an episodic
//! parser takes part. A film the client holds suggests nothing, because the
//! reader already has it and no later release of it follows.
//!
//! Nothing here grabs a torrent. A ruleset puts titles on the wanted list,
//! and the client already holds these.

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};

use super::form::RulesetForm;
use super::{Condition, Op};
use crate::parser::{Field, FieldKind, Parser, TitleTest};
use crate::rules::{Engine, Reading};
use crate::torrent::{Torrent, TorrentId};

/// Fields a suggested condition never names.
///
/// A feed title carries no extension and no checksum, so a condition on
/// either claims nothing the feed announces. A condition on the episode name
/// claims one episode, where a suggestion is about a whole show.
const SKIPPED_FIELDS: &[&str] = &["extension", "checksum", "episodeName"];

/// One ruleset an import offers to create.
///
/// The torrents and the time are what the preview reports about a subject,
/// so the reader decides from them whether the suggestion is one they want.
///
/// No [`Eq`], because a member carries the client's report of a torrent and
/// that carries a progress fraction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Suggestion {
    /// [`crate::parser::Parser::id`] the suggested ruleset reads with.
    pub(crate) parser: String,

    /// The subject as [`FieldKind::normalize`] renders it.
    ///
    /// A checkbox posts this back, and the dedupe compares it, so it is the
    /// one spelling two torrents that named the subject differently share.
    pub(crate) key: String,

    /// The subject as the suggested ruleset names it.
    pub(crate) show: String,

    /// The client's torrents this subject accounts for, in the order the
    /// client listed them.
    pub(crate) members: Vec<Member>,

    /// When the client added the newest of them, or nothing when it named no
    /// time for any.
    pub(crate) newest: Option<DateTime<Utc>>,

    /// What the suggested ruleset compares, the subject first and the fields
    /// the torrents agree on after it, in the parser's own order.
    pub(crate) conditions: Vec<Condition>,

    /// The ruleset that already names this subject, when one does.
    pub(crate) collision: Option<Collision>,

    /// [`crate::parser::Parser::id`] of the earlier suggestion about this
    /// same subject, when one precedes this.
    ///
    /// A client holds one show in two shapes when one tracker names the
    /// episode and another does not. Each shape reads through its own
    /// parser, so both are offered and the later one says what it repeats.
    pub(crate) repeats: Option<String>,
}

/// One torrent a subject groups, and whether the reader kept it.
///
/// An excluded torrent stays listed so the reader takes it back, and it
/// feeds no condition while it is out. That is how a name the reader
/// rejects stops holding an agreement back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub(crate) torrent: Torrent,
    pub(crate) included: bool,
}

/// A ruleset that already names the subject a suggestion is about.
///
/// On the same parser the suggestion is one the reader has, so the preview
/// shows it and offers nothing. On another parser the preview still offers
/// it, because each ruleset claims the shape its own parser reads and the
/// reader wants both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Collision {
    /// [`crate::ruleset::Ruleset::id`] of the ruleset that names the subject.
    pub(crate) ruleset: String,

    pub(crate) same_parser: bool,
}

/// Every torrent of one show, and what each of them read.
#[derive(Default)]
struct Group {
    /// Every torrent of the group in the order the client listed them,
    /// excluded ones among them.
    members: Vec<Member>,

    /// One entry per included torrent, each field's raw capture beside its
    /// normalized form.
    readings: Vec<BTreeMap<String, (String, String)>>,

    newest: Option<DateTime<Utc>>,

    /// What the newest torrent read, which is where an agreed condition
    /// takes its value from.
    newest_values: BTreeMap<String, (String, String)>,
}

impl Group {
    /// Returns the newest torrent's raw capture for `field`, when every
    /// torrent in the group read the field and read the same value.
    ///
    /// The comparison is on the normalized readings, so two torrents that
    /// spell one value differently still agree. The raw capture is what the
    /// condition carries, because that is the spelling the reader sees.
    fn agreed(&self, field: &str) -> Option<String> {
        let (raw, normalized) = self.newest_values.get(field)?;

        self.readings
            .iter()
            .all(|read| read.get(field).is_some_and(|(_, read)| read == normalized))
            .then(|| raw.clone())
    }
}

/// Returns one suggestion per subject the client holds.
///
/// A torrent named in `excluded` stays listed among the suggestion's
/// members and feeds no condition, so a name the reader rejects stops
/// holding an agreement back. A group whose every torrent is excluded still
/// suggests, with the subject condition alone and no time, which the sort
/// places last.
///
/// A subject a ruleset already names carries that ruleset in its
/// [`Suggestion::collision`] rather than dropping out, so the reader sees
/// that the client holds a show they follow.
///
/// A torrent whose name no parser reads is ignored, as is one read by a
/// parser with no subject or with nothing episodic about it. Neither says
/// anything about a ruleset the reader wants.
///
/// One subject read by two parsers yields one suggestion per parser, and
/// each after the first carries the earlier one in
/// [`Suggestion::repeats`].
///
/// The suggestions come out newest first, so the subject the reader added
/// most recently is the one the preview leads with. A subject the client
/// named no time for comes last.
pub(crate) fn plan(
    engine: &Engine,
    torrents: &[Torrent],
    excluded: &HashSet<TorrentId>,
) -> Vec<Suggestion> {
    let mut groups: BTreeMap<(String, String), Group> = BTreeMap::new();

    for torrent in torrents {
        let Some(reading) = engine.read(&torrent.name) else {
            continue;
        };

        let Some(parser) = engine.parser(&reading.parser) else {
            continue;
        };

        let Some(subject) = parser.subject().filter(|_| parser.episodic()) else {
            continue;
        };

        let Some(show) = reading
            .values
            .iter()
            .find(|(field, _)| field == &subject.name)
            .map(|(_, raw)| raw)
        else {
            continue;
        };

        let key = subject.kind.normalize(show);
        let read = reading_of(parser, &reading);

        let group = groups.entry((reading.parser.clone(), key)).or_default();
        let included = !excluded.contains(&torrent.id);

        group.members.push(Member {
            torrent: torrent.clone(),
            included,
        });

        if !included {
            continue;
        }

        // A torrent the client named no time for counts as the oldest, and
        // `None` orders below every `Some`. The first torrent of a group
        // seeds the values even so, because a group of nothing but those
        // still suggests conditions.
        if group.readings.is_empty() || torrent.added_at > group.newest {
            group.newest = torrent.added_at;
            group.newest_values = read.clone();
        }

        group.readings.push(read);
    }

    let mut suggestions = groups
        .into_iter()
        .filter_map(|((parser_id, key), group)| {
            let parser = engine.parser(&parser_id)?;
            let subject = parser.subject()?;
            let show = titled(&key);
            let conditions = conditions(parser, subject, &show, &group);

            Some(Suggestion {
                collision: collision(engine, &parser_id, &subject.name, &key),
                parser: parser_id,
                members: group.members,
                newest: group.newest,
                repeats: None,
                key,
                show,
                conditions,
            })
        })
        .collect::<Vec<_>>();

    suggestions.sort_by(|one, other| {
        other
            .newest
            .cmp(&one.newest)
            .then_with(|| one.show.cmp(&other.show))
    });

    // After the sort, because the earlier suggestion is the one the reader
    // meets first rather than the one the grouping happened to build first.
    let mut seen: BTreeMap<String, String> = BTreeMap::new();

    for suggestion in &mut suggestions {
        match seen.get(&suggestion.key) {
            Some(parser) => suggestion.repeats = Some(parser.clone()),
            None => {
                seen.insert(suggestion.key.clone(), suggestion.parser.clone());
            }
        }
    }

    suggestions
}

/// What one reading captured, each field's raw value beside its normalized
/// form.
///
/// A capture no field of `parser` carries is dropped, which the composed
/// regex never produces.
fn reading_of(parser: &Parser, reading: &Reading) -> BTreeMap<String, (String, String)> {
    reading
        .values
        .iter()
        .filter_map(|(field, raw)| {
            let kind = parser.fields.iter().find(|one| &one.name == field)?.kind;

            Some((field.clone(), (raw.clone(), kind.normalize(raw))))
        })
        .collect()
}

/// What a suggested ruleset compares.
///
/// The subject leads, and every field the group agrees on follows it in the
/// parser's own order.
///
/// An identity field beyond the subject names one release rather than the
/// set the reader wants, so only the rest take part.
fn conditions(parser: &Parser, subject: &Field, show: &str, group: &Group) -> Vec<Condition> {
    let mut conditions = vec![Condition {
        field: subject.name.clone(),
        op: Op::Equals,
        value: show.to_owned(),
    }];

    conditions.extend(
        parser
            .fields
            .iter()
            .filter(|field| !field.identity && !SKIPPED_FIELDS.contains(&field.name.as_str()))
            .filter_map(|field| {
                Some(Condition {
                    field: field.name.clone(),
                    op: Op::Equals,
                    value: group.agreed(&field.name)?,
                })
            }),
    );

    conditions
}

/// Reads `title` into the form a ruleset editor starts from.
///
/// A feed title is a group of one, so every field it read agrees and each
/// becomes a condition. The reader removes the ones they do not want, which
/// is quicker than typing the ones they do.
///
/// Any parser serves here, unlike an import, because a reader who wants a
/// film says so from its title. A title no parser reads keeps its place as
/// the draft's one test, so the reader writes the fields against something.
pub(crate) fn seed(engine: &Engine, title: &str) -> RulesetForm {
    let read = engine.read(title).and_then(|reading| {
        let parser = engine.parser(&reading.parser)?;
        let subject = parser.subject()?;

        Some((parser, subject, reading_of(parser, &reading)))
    });

    let Some((parser, subject, read)) = read else {
        return RulesetForm {
            name: String::new(),
            parser: String::new(),
            conditions: Vec::new(),
            tests: vec![TitleTest {
                title: title.to_owned(),
                expected: BTreeMap::new(),
            }],
        };
    };

    let show = read
        .get(&subject.name)
        .map_or_else(String::new, |(raw, _)| raw.clone());

    let group = Group {
        newest_values: read.clone(),
        readings: vec![read.clone()],
        ..Group::default()
    };

    let conditions = conditions(parser, subject, &show, &group);

    let expected = conditions
        .iter()
        .filter_map(|condition| {
            let (_, normalized) = read.get(&condition.field)?;

            Some((condition.field.clone(), normalized.clone()))
        })
        .collect();

    RulesetForm {
        name: String::new(),
        parser: parser.id.clone(),
        conditions,
        tests: vec![TitleTest {
            title: title.to_owned(),
            expected,
        }],
    }
}

/// Finds the ruleset that already names `key` under the field `subject`.
///
/// The one on `parser` wins over one on another parser, because that is the
/// ruleset this suggestion repeats. Both sides normalize before they compare,
/// so a ruleset that spells the subject differently is still found.
fn collision(engine: &Engine, parser: &str, subject: &str, key: &str) -> Option<Collision> {
    let named = engine
        .rulesets()
        .filter(|ruleset| {
            ruleset.conditions.iter().any(|condition| {
                condition.field == subject
                    && condition.op == Op::Equals
                    && FieldKind::Text.normalize(&condition.value) == key
            })
        })
        .collect::<Vec<_>>();

    let found = named
        .iter()
        .find(|ruleset| ruleset.parser == parser)
        .or(named.first())?;

    Some(Collision {
        ruleset: found.id.clone(),
        same_parser: found.parser == parser,
    })
}

/// Returns `key` with the first letter of each word in upper case.
///
/// The key is normalized, so it reads `coastal ecology`. A ruleset carries
/// the name the reader types by hand, which is `Coastal Ecology`.
fn titled(key: &str) -> String {
    let mut show = String::with_capacity(key.len());

    for word in key.split(' ') {
        if !show.is_empty() {
            show.push(' ');
        }

        let mut letters = word.chars();

        if let Some(first) = letters.next() {
            show.extend(first.to_uppercase());
            show.push_str(letters.as_str());
        }
    }

    show
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use chrono::{TimeZone, Utc};

    use super::{Collision, Suggestion, plan, seed};
    use crate::parser::{Field, Parser, TitleTest};
    use crate::rules::Engine;
    use crate::ruleset::fixture::{self, ENGINE};
    use crate::ruleset::{Condition, Op, Ruleset};
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    fn torrent(name: &str, day: u32) -> Torrent {
        Torrent {
            id: TorrentId(name.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
            added_at: Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0).single(),
        }
    }

    /// Plans over `torrents` with nothing excluded, which is what every test
    /// but the exclusion one asks about.
    fn planned(engine: &Engine, torrents: &[Torrent]) -> Vec<Suggestion> {
        plan(engine, torrents, &HashSet::new())
    }

    fn equals(field: &str, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op: Op::Equals,
            value: value.to_owned(),
        }
    }

    #[test]
    fn unanimous_fields_become_conditions() {
        let planned = planned(
            &ENGINE,
            &[
                torrent(
                    "Coastal.Ecology.S01E01.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                    4,
                ),
                torrent(
                    "Coastal.Ecology.S01E02.1080p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv",
                    6,
                ),
            ],
        );

        let [suggestion] = planned.as_slice() else {
            panic!("one show, one suggestion, found {}", planned.len());
        };

        assert_eq!(suggestion.show, "Coastal Ecology");
        assert_eq!(suggestion.members.len(), 2);
        assert_eq!(
            suggestion.conditions,
            [
                equals("show", "Coastal Ecology"),
                equals("resolution", "1080p"),
                equals("source", "Broadcast"),
                equals("audio", "AAC.Stereo"),
                equals("codec", "H.264"),
            ],
            "the two names disagree on the publisher alone, and no condition names it"
        );
    }

    #[test]
    fn a_show_a_ruleset_names_is_listed_with_its_collision() {
        let planned = planned(
            &ENGINE,
            &[torrent(
                "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv",
                4,
            )],
        );

        let [suggestion] = &planned[..] else {
            panic!("one show, so one suggestion: {planned:?}");
        };

        assert_eq!(
            suggestion.collision,
            Some(Collision {
                ruleset: "series-hollow-meridian".to_owned(),
                same_parser: true,
            }),
            "the reader follows this show already, and the preview says so rather than hiding it"
        );
    }

    #[test]
    fn a_ruleset_on_another_parser_marks_the_collision() {
        let copy = Parser {
            id: "series-copy".to_owned(),
            ..fixture::parsers()
                .into_iter()
                .find(|parser| parser.id == "series-episodes")
                .expect("the fixture declares the episode parser")
        };

        let engine = Engine::new(
            fixture::parsers().into_iter().chain([copy]).collect(),
            vec![Ruleset {
                id: "hollow-copy".to_owned(),
                name: "Hollow copy".to_owned(),
                enabled: true,
                parser: "series-copy".to_owned(),
                conditions: vec![equals("show", "The Hollow Meridian")],
                tests: Vec::new(),
            }],
        )
        .expect("the fixture patterns compile");

        let planned = planned(
            &engine,
            &[torrent(
                "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv",
                4,
            )],
        );

        let [suggestion] = &planned[..] else {
            panic!("one show, so one suggestion: {planned:?}");
        };

        assert_eq!(
            suggestion.collision,
            Some(Collision {
                ruleset: "hollow-copy".to_owned(),
                same_parser: false,
            }),
            "the other parser claims a different shape, so the reader wants this one too"
        );
    }

    #[test]
    fn a_show_read_by_two_parsers_is_listed_twice_and_the_second_says_so() {
        let episodes = fixture::parsers()
            .into_iter()
            .find(|parser| parser.id == "series-episodes")
            .expect("the fixture declares the episode parser");

        let strict = Parser {
            id: "series-strict".to_owned(),
            fields: episodes
                .fields
                .iter()
                .map(|field| Field {
                    required: field.name == "resolution" || field.required,
                    ..field.clone()
                })
                .collect(),
            ..episodes.clone()
        };

        let engine =
            Engine::new(vec![strict, episodes], Vec::new()).expect("the fixture patterns compile");

        let planned = planned(
            &engine,
            &[
                torrent("Coastal.Ecology.S01E01.1080p.Broadcast-PublicWave.mkv", 6),
                torrent("Coastal.Ecology.S01E02.Broadcast-PublicWave.mkv", 4),
            ],
        );

        assert_eq!(
            planned
                .iter()
                .map(|suggestion| (&*suggestion.parser, suggestion.repeats.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("series-strict", None),
                ("series-episodes", Some("series-strict")),
            ],
            "one show in two shapes, and the later row names the row it repeats"
        );
    }

    #[test]
    fn an_excluded_torrent_stays_listed_and_leaves_the_agreement() {
        let torrents = [
            torrent(
                "Coastal.Ecology.S01E01.720p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                4,
            ),
            torrent(
                "Coastal.Ecology.S01E02.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                6,
            ),
        ];

        let whole = planned(&ENGINE, &torrents);

        let [whole] = whole.as_slice() else {
            panic!("one show, so one suggestion");
        };

        assert!(
            !whole.conditions.contains(&equals("resolution", "1080p")),
            "the two disagree on the resolution, so no condition names it"
        );

        let excluded = HashSet::from([torrents[0].id.clone()]);
        let planned = plan(&ENGINE, &torrents, &excluded);

        let [suggestion] = planned.as_slice() else {
            panic!("one show, so one suggestion");
        };

        assert!(
            suggestion
                .conditions
                .contains(&equals("resolution", "1080p")),
            "the one that disagreed is out, so the rest agree: {:?}",
            suggestion.conditions
        );
        assert_eq!(
            suggestion
                .members
                .iter()
                .map(|member| (&*member.torrent.name, member.included))
                .collect::<Vec<_>>(),
            [(&*torrents[0].name, false), (&*torrents[1].name, true),],
            "an excluded torrent stays listed, in the order the client gave"
        );
    }

    #[test]
    fn a_seed_reads_the_title_into_conditions_and_a_test() {
        const TITLE: &str =
            "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";

        let seeded = seed(&ENGINE, TITLE);

        assert_eq!(seeded.parser, "series-episodes");
        assert_eq!(
            seeded.conditions,
            [
                equals("show", "The.Hollow.Meridian"),
                equals("resolution", "720p"),
                equals("source", "Broadcast"),
                equals("audio", "AAC.Stereo"),
                equals("codec", "H.264"),
                equals("publisher", "OtherGroup"),
            ],
            "one title agrees with itself, so every field it read becomes a condition"
        );
        assert_eq!(
            seeded.tests,
            [TitleTest {
                title: TITLE.to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("resolution".to_owned(), "720p".to_owned()),
                    ("source".to_owned(), "broadcast".to_owned()),
                    ("audio".to_owned(), "aac stereo".to_owned()),
                    ("codec".to_owned(), "h 264".to_owned()),
                    ("publisher".to_owned(), "othergroup".to_owned()),
                ]),
            }],
            "the test expects what the title read, in the normalized form a verdict compares"
        );
    }

    #[test]
    fn a_seed_of_a_title_no_parser_reads_keeps_the_title_as_a_test() {
        let seeded = seed(&ENGINE, "just some words with no structure at all");

        assert_eq!(seeded.parser, "", "no parser read it, so none is named");
        assert_eq!(seeded.conditions, [], "and nothing was read to compare");
        assert_eq!(
            seeded.tests,
            [TitleTest {
                title: "just some words with no structure at all".to_owned(),
                expected: BTreeMap::new(),
            }],
            "the title stays, so the reader writes the fields against something"
        );
    }

    #[test]
    fn a_film_the_client_holds_suggests_nothing() {
        assert_eq!(
            planned(
                &ENGINE,
                &[torrent(
                    "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv",
                    4,
                )],
            ),
            Vec::new(),
            "a film arrives once, and the reader already has it"
        );
    }

    #[test]
    fn newest_torrent_leads() {
        let planned = planned(
            &ENGINE,
            &[
                torrent(
                    "Ridge.Runner.S02E03.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                    2,
                ),
                torrent(
                    "Coastal.Ecology.S01E01.1080p.Broadcast.AAC.Stereo.H.264-publicwave.mkv",
                    4,
                ),
                torrent(
                    "Coastal.Ecology.S01E02.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                    6,
                ),
            ],
        );

        assert_eq!(
            planned
                .iter()
                .map(|suggestion| suggestion.show.as_str())
                .collect::<Vec<_>>(),
            ["Coastal Ecology", "Ridge Runner"],
            "the show the client added most recently leads"
        );
        assert!(
            planned[0]
                .conditions
                .contains(&equals("publisher", "PublicWave")),
            "the two spellings agree once normalized, and the newest torrent's wins"
        );
    }

    #[test]
    fn a_name_no_parser_reads_is_ignored() {
        assert_eq!(
            planned(
                &ENGINE,
                &[torrent("just some words with no structure at all", 4)],
            ),
            Vec::new()
        );
    }
}
