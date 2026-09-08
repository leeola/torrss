//! Reading the torrents a client holds into the searches they imply.
//!
//! A client full of one subject is the reader saying they follow it. This
//! reads that statement into a search the reader writes by hand otherwise.
//! The search names the subject, and every other field those torrents read
//! is what the preview offers as a chip. A field they read several ways
//! carries every value they read, so a chip the reader turns on claims what
//! the client holds rather than any value at all.
//!
//! The grouping is by the parser's own subject rather than by a field called
//! `show`, because a parser names its subject as it likes. Only an episodic
//! parser takes part. A film the client holds suggests nothing, because the
//! reader already has it and no later release of it follows.
//!
//! Nothing here grabs a torrent. A search puts titles on the wanted list,
//! and the client already holds these.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};

use super::form::SearchForm;
use super::{Condition, Op};
use crate::engine::{Engine, Reading};
use crate::parser::{Field, FieldKind, Parser, TitleTest};
use crate::torrent::{Torrent, TorrentId};

/// Fields a suggested condition never names.
///
/// A feed title carries no extension and no checksum, so a condition on
/// either claims nothing the feed announces. A condition on the episode name
/// claims one episode, where a suggestion is about a whole show.
const SKIPPED_FIELDS: &[&str] = &["extension", "checksum", "episodeName"];

/// What one torrent's name read, each field's raw capture beside its
/// normalized form.
///
/// A condition carries the raw capture, because that is the spelling the
/// reader sees. The normalized form is what two readings compare on, so two
/// torrents that spell one value differently name it once.
type Captures = BTreeMap<String, (String, String)>;

/// One search an import offers to create.
///
/// The torrents and the time are what the preview reports about a subject,
/// so the reader decides from them whether the suggestion is one they want.
///
/// No [`Eq`], because a member carries the client's report of a torrent and
/// that carries a progress fraction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Suggestion {
    /// [`crate::parser::Parser::id`] the suggested search reads with.
    pub(crate) parser: String,

    /// The subject as [`FieldKind::normalize`] renders it.
    ///
    /// A checkbox posts this back, and the dedupe compares it, so it is the
    /// one spelling two torrents that named the subject differently share.
    pub(crate) key: String,

    /// The subject as the suggested search names it.
    pub(crate) show: String,

    /// The client's torrents this subject accounts for, in the order the
    /// client listed them.
    pub(crate) members: Vec<Member>,

    /// When the client added the newest of its members, or nothing when it
    /// named no time for any.
    ///
    /// An excluded member counts, so a suggestion keeps its place in the
    /// list while the reader unchecks torrents.
    pub(crate) newest: Option<DateTime<Utc>>,

    /// The subject first, then the fields the torrents read after it, in the
    /// parser's own order.
    ///
    /// The search compares the subject. The rest is what the preview offers
    /// as chips, and each one joins the search only where the reader turns it
    /// on.
    pub(crate) conditions: Vec<Condition>,

    /// The search that already names this subject, when one does.
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
/// rejects keeps its values out of the search.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub(crate) torrent: Torrent,
    pub(crate) included: bool,
}

/// A search that already names the subject a suggestion is about.
///
/// On the same parser the suggestion is one the reader has, so the preview
/// shows it and offers nothing. On another parser the preview still offers
/// it, because each search claims the shape its own parser reads and the
/// reader wants both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Collision {
    /// [`crate::search::Search::id`] of the search that names the subject.
    pub(crate) search: String,

    pub(crate) same_parser: bool,
}

/// Every torrent of one show, and what each of them read.
#[derive(Default)]
struct Group {
    /// Every torrent of the group in the order the client listed them,
    /// excluded ones among them.
    members: Vec<Member>,

    /// One entry per included torrent, when the client added it beside what
    /// its name read.
    readings: Vec<(Option<DateTime<Utc>>, Captures)>,

    /// When the client added the newest member, excluded ones among them,
    /// so a review moves no suggestion in the list.
    newest: Option<DateTime<Utc>>,
}

impl Group {
    /// Returns every value the group read for `field`, newest first.
    ///
    /// Two torrents that spell one value differently name it once. The
    /// comparison is on the normalized readings, and the spelling is the
    /// newest torrent's raw capture, because that is what the reader sees.
    /// A tie on the time keeps the spelling of the torrent the client
    /// listed first.
    ///
    /// A field one torrent of the group did not read gives nothing. A
    /// condition on a list of values fails on a name that carries no value,
    /// and the client holds such a name.
    ///
    /// A group that includes no torrent gives nothing too, because it read
    /// no value at all.
    fn values(&self, field: &str) -> Option<Vec<String>> {
        if self.readings.is_empty() {
            return None;
        }

        let mut values: Vec<(Option<DateTime<Utc>>, &str, &str)> = Vec::new();

        for (added_at, read) in &self.readings {
            let (raw, normalized) = read.get(field)?;

            match values.iter_mut().find(|(_, seen, _)| seen == normalized) {
                Some(value) if *added_at > value.0 => *value = (*added_at, normalized, raw),
                Some(_) => {}
                None => values.push((*added_at, normalized, raw)),
            }
        }

        values.sort_by_key(|value| Reverse(value.0));

        Some(
            values
                .into_iter()
                .map(|(_, _, raw)| raw.to_owned())
                .collect(),
        )
    }
}

/// Returns one suggestion per subject the client holds.
///
/// A torrent named in `excluded` stays listed among the suggestion's
/// members and feeds no condition, so a name the reader rejects keeps its
/// values out of the search. A group whose every torrent is excluded still
/// suggests, with the subject condition alone, and it keeps its place in the
/// list.
///
/// A subject a search already names carries that search in its
/// [`Suggestion::collision`] rather than dropping out, so the reader sees
/// that the client holds a show they follow.
///
/// A torrent whose name no parser reads is ignored, as is one read by a
/// parser with no subject or with nothing episodic about it. Neither says
/// anything about a search the reader wants.
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

        // Every member counts here, so unchecking a torrent leaves the
        // suggestion where the reader met it. `None` orders below every
        // `Some`, so a group the client named no time for stays `None`.
        group.newest = group.newest.max(torrent.added_at);

        if !included {
            continue;
        }

        group.readings.push((torrent.added_at, read));
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

/// Reads one [`Reading`] into the captures a condition takes its value
/// from.
///
/// A capture no field of `parser` carries is dropped, which the composed
/// regex never produces.
fn reading_of(parser: &Parser, reading: &Reading) -> Captures {
    reading
        .values
        .iter()
        .filter_map(|(field, raw)| {
            let kind = parser.fields.iter().find(|one| &one.name == field)?.kind;

            Some((field.clone(), (raw.clone(), kind.normalize(raw))))
        })
        .collect()
}

/// The subject a suggested search compares, and the fields the preview offers
/// as chips.
///
/// The subject leads, and every field each included torrent read follows it
/// in the parser's own order. A field they all read one way becomes an
/// `equals`. A field they read several ways becomes a `one of` naming every
/// value, the newest first. The list is what the client holds, so a chip the
/// reader turns on claims that rather than any value at all.
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
                let values = group.values(&field.name)?;

                Some(Condition {
                    field: field.name.clone(),
                    op: if values.len() == 1 {
                        Op::Equals
                    } else {
                        Op::OneOf
                    },
                    value: values.join(", "),
                })
            }),
    );

    conditions
}

/// Reads `title` into the form a search editor starts from.
///
/// A feed title is a group of one, so every field it read names one value
/// and each becomes a condition. The reader removes the ones they do not
/// want, which is quicker than typing the ones they do.
///
/// Any parser serves here, unlike an import, because a reader who wants a
/// film says so from its title. A title no parser reads keeps its place as
/// the draft's one test, so the reader writes the fields against something.
pub(crate) fn seed(engine: &Engine, title: &str) -> SearchForm {
    let read = engine.read(title).and_then(|reading| {
        let parser = engine.parser(&reading.parser)?;
        let subject = parser.subject()?;

        Some((parser, subject, reading_of(parser, &reading)))
    });

    let Some((parser, subject, read)) = read else {
        return SearchForm {
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
        readings: vec![(None, read.clone())],
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

    SearchForm {
        name: String::new(),
        parser: parser.id.clone(),
        conditions,
        tests: vec![TitleTest {
            title: title.to_owned(),
            expected,
        }],
    }
}

/// Finds the search that already names `key` under the field `subject`.
///
/// The one on `parser` wins over one on another parser, because that is the
/// search this suggestion repeats. Both sides normalize before they compare,
/// so a search that spells the subject differently is still found.
fn collision(engine: &Engine, parser: &str, subject: &str, key: &str) -> Option<Collision> {
    let named = engine
        .searches()
        .filter(|search| {
            search.conditions.iter().any(|condition| {
                condition.field == subject
                    && condition.op == Op::Equals
                    && FieldKind::Text.normalize(&condition.value) == key
            })
        })
        .collect::<Vec<_>>();

    let found = named
        .iter()
        .find(|search| search.parser == parser)
        .or(named.first())?;

    Some(Collision {
        search: found.id.clone(),
        same_parser: found.parser == parser,
    })
}

/// Returns `key` with the first letter of each word in upper case.
///
/// The key is normalized, so it reads `coastal ecology`. A search carries
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
    use crate::engine::Engine;
    use crate::parser::{Field, Parser, TitleTest};
    use crate::search::fixture::{self, ENGINE};
    use crate::search::{Condition, Op, Search};
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

    fn one_of(field: &str, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op: Op::OneOf,
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_field_read_one_way_equals_and_read_two_ways_lists_both() {
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
                one_of("publisher", "OtherGroup, PublicWave"),
            ],
            "the two names read one publisher each, and the condition lists both"
        );
    }

    #[test]
    fn a_show_a_search_names_is_listed_with_its_collision() {
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
                search: "series-hollow-meridian".to_owned(),
                same_parser: true,
            }),
            "the reader follows this show already, and the preview says so rather than hiding it"
        );
    }

    #[test]
    fn a_search_on_another_parser_marks_the_collision() {
        let copy = Parser {
            id: "series-copy".to_owned(),
            ..fixture::parsers()
                .into_iter()
                .find(|parser| parser.id == "series-episodes")
                .expect("the fixture declares the episode parser")
        };

        let engine = Engine::new(
            fixture::parsers().into_iter().chain([copy]).collect(),
            vec![Search {
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
                search: "hollow-copy".to_owned(),
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
    fn an_excluded_torrent_stays_listed_and_feeds_no_condition() {
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
            whole
                .conditions
                .contains(&one_of("resolution", "1080p, 720p"))
                && !whole.conditions.contains(&equals("resolution", "1080p")),
            "both resolutions are listed, the newest first"
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
            "the one that read 720p is out, so the list names 1080p alone: {:?}",
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
    fn a_field_one_torrent_did_not_read_names_no_condition() {
        let planned = planned(
            &ENGINE,
            &[
                torrent(
                    "Coastal.Ecology.S01E01.1080p.Broadcast.AAC.Stereo-PublicWave.mkv",
                    4,
                ),
                torrent(
                    "Coastal.Ecology.S01E02.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv",
                    6,
                ),
            ],
        );

        let [suggestion] = planned.as_slice() else {
            panic!("one show, one suggestion, found {}", planned.len());
        };

        assert_eq!(suggestion.members.len(), 2, "one show, both torrents");
        assert_eq!(
            suggestion.conditions,
            [
                equals("show", "Coastal Ecology"),
                equals("resolution", "1080p"),
                equals("source", "Broadcast"),
                equals("audio", "AAC.Stereo"),
                equals("publisher", "PublicWave"),
            ],
            "the first name carries no codec, and a list naming H.264 would reject it"
        );
    }

    #[test]
    fn an_excluded_torrent_keeps_the_suggestion_in_place() {
        let torrents = [
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
        ];

        let excluded = HashSet::from([torrents[1].id.clone(), torrents[2].id.clone()]);
        let planned = plan(&ENGINE, &torrents, &excluded);

        assert_eq!(
            planned
                .iter()
                .map(|suggestion| &*suggestion.show)
                .collect::<Vec<_>>(),
            ["Coastal Ecology", "Ridge Runner"],
            "an unchecked torrent moves no suggestion"
        );
        assert_eq!(
            planned[0].newest, torrents[2].added_at,
            "the age is the client's, not the review's"
        );
        assert_eq!(
            planned[0].conditions,
            [equals("show", "Coastal Ecology")],
            "nothing included, so the subject alone is a condition"
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
