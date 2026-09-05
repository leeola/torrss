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

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::{Condition, Op};
use crate::parser::FieldKind;
use crate::rules::Engine;
use crate::torrent::Torrent;

/// Fields a suggested condition never names.
///
/// A feed title carries no extension and no checksum, so a condition on
/// either claims nothing the feed announces. A condition on the episode name
/// claims one episode, where a suggestion is about a whole show.
const SKIPPED_FIELDS: &[&str] = &["extension", "checksum", "episodeName"];

/// One ruleset an import offers to create.
///
/// The count and the time are what the preview reports about a subject, so
/// the reader decides from them whether the suggestion is one they want.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// How many of the client's torrents this subject accounts for.
    pub(crate) torrents: usize,

    /// When the client added the newest of them, or nothing when it named no
    /// time for any.
    pub(crate) newest: Option<DateTime<Utc>>,

    /// What the suggested ruleset compares, the subject first and the fields
    /// the torrents agree on after it, in the parser's own order.
    pub(crate) conditions: Vec<Condition>,

    /// The ruleset that already names this subject, when one does.
    pub(crate) collision: Option<Collision>,
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
    /// One entry per torrent, each field's raw capture beside its normalized
    /// form.
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
/// A subject a ruleset already names carries that ruleset in its
/// [`Suggestion::collision`] rather than dropping out, so the reader sees
/// that the client holds a show they follow.
///
/// A torrent whose name no parser reads is ignored, as is one read by a
/// parser with no subject or with nothing episodic about it. Neither says
/// anything about a ruleset the reader wants.
///
/// The suggestions come out newest first, so the subject the reader added
/// most recently is the one the preview leads with. A subject the client
/// named no time for comes last.
pub(crate) fn plan(engine: &Engine, torrents: &[Torrent]) -> Vec<Suggestion> {
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

        let read = reading
            .values
            .iter()
            .filter_map(|(field, raw)| {
                let kind = parser.fields.iter().find(|one| &one.name == field)?.kind;

                Some((field.clone(), (raw.clone(), kind.normalize(raw))))
            })
            .collect::<BTreeMap<_, _>>();

        let group = groups.entry((reading.parser.clone(), key)).or_default();

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

            let mut conditions = vec![Condition {
                field: subject.name.clone(),
                op: Op::Equals,
                value: show.clone(),
            }];

            conditions.extend(
                parser
                    .fields
                    .iter()
                    .filter(|field| {
                        !field.identity && !SKIPPED_FIELDS.contains(&field.name.as_str())
                    })
                    .filter_map(|field| {
                        Some(Condition {
                            field: field.name.clone(),
                            op: Op::Equals,
                            value: group.agreed(&field.name)?,
                        })
                    }),
            );

            Some(Suggestion {
                collision: collision(engine, &parser_id, &subject.name, &key),
                parser: parser_id,
                torrents: group.readings.len(),
                newest: group.newest,
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

    suggestions
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
    use chrono::{TimeZone, Utc};

    use super::{Collision, plan};
    use crate::parser::Parser;
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

    fn equals(field: &str, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op: Op::Equals,
            value: value.to_owned(),
        }
    }

    #[test]
    fn unanimous_fields_become_conditions() {
        let planned = plan(
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
        assert_eq!(suggestion.torrents, 2);
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
        let planned = plan(
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

        let planned = plan(
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
    fn a_film_the_client_holds_suggests_nothing() {
        assert_eq!(
            plan(
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
        let planned = plan(
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
            plan(
                &ENGINE,
                &[torrent("just some words with no structure at all", 4)],
            ),
            Vec::new()
        );
    }
}
