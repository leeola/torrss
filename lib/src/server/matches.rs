//! What an in-progress ruleset edit does to the titles the feeds carry.
//!
//! An authored example teaches nothing about a reader's own feeds. The
//! editor instead runs the saved rules and the edited rules over the stored
//! titles and reports the difference, so a pattern is judged against the
//! names it will actually meet.
//!
//! An edit rides as form-encoded text rather than as a typed struct. That is
//! what the browser sends from the editor's GET form, and what a live
//! re-render carries, so one parser serves both and a link round-trips
//! through [`Edits::pairs`] unchanged.
//!
//! A rule with no usable pattern claims nothing rather than everything. A
//! reader who breaks a regex mid-edit sees every title fall out of the set,
//! which reads as the mistake it is. The opposite quietly claims the whole
//! feed and tells the reader nothing.

use std::collections::BTreeMap;
use std::ops::Range;

use regex::Regex;
use url::form_urlencoded;

use crate::ruleset::{Diff, Field, FieldKind, Part, Segment};

/// Every field attribute the editor's form carries, keyed by field name.
///
/// The editor's own control (`diff`) shares the query string with the field
/// inputs. A key with no `.` in it is that one, so it passes through
/// untouched rather than becoming a field named after a control.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Edits(BTreeMap<String, FieldEdit>);

/// One field's attributes as the form posted them.
///
/// Every attribute is optional because a disabled input posts nothing. An
/// absent value means "keep what the field declares", never "clear it".
#[derive(Debug, Default, PartialEq, Eq)]
struct FieldEdit {
    name: Option<String>,
    kind: Option<FieldKind>,
    pattern: Option<String>,

    /// A checkbox posts its name only when checked, so absence is `false`.
    required: bool,
}

/// One field resolved into something that runs against a title.
///
/// `regex` is [`None`] when the pattern failed to compile or none was
/// available at all. Such a rule claims nothing, and a required one turns
/// every title away.
#[derive(Debug)]
pub(super) struct Rule {
    name: String,
    part: Part,
    required: bool,
    regex: Option<Regex>,
}

/// Why one field's pattern did not compile.
///
/// Named by field rather than by index, because the editor shows the message
/// beside the input the reader edits.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct PatternError {
    pub(super) field: String,
    pub(super) message: String,
}

/// One stored title as the Matches section shows it.
///
/// The segments borrow the title, so a match costs no copy of the name it
/// describes.
#[derive(Debug)]
pub(super) struct Match<'a> {
    pub(super) id: i64,
    pub(super) segments: Vec<Segment<'a>>,
    pub(super) diff: Diff,
    pub(super) feed: String,
}

#[allow(
    dead_code,
    reason = "the live re-render parses an edit out of the form and writes it back"
)]
impl Edits {
    /// Reads every `field.attribute` pair out of a form-encoded query.
    pub(super) fn parse(query: &str) -> Self {
        let mut edits: BTreeMap<String, FieldEdit> = BTreeMap::new();

        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            let Some((field, attribute)) = key.rsplit_once('.') else {
                continue;
            };

            let edit = edits.entry(field.to_owned()).or_default();

            match attribute {
                "name" => edit.name = Some(value.into_owned()),
                "kind" => edit.kind = FieldKind::from_label(&value),
                "pattern" => edit.pattern = Some(value.into_owned()),
                "required" => edit.required = true,
                _ => {}
            }
        }

        Self(edits)
    }

    /// The same pairs the form posted, for writing back into a link.
    pub(super) fn pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();

        for (field, edit) in &self.0 {
            if let Some(name) = &edit.name {
                pairs.push((format!("{field}.name"), name.clone()));
            }

            if let Some(kind) = edit.kind {
                pairs.push((format!("{field}.kind"), kind.label().to_owned()));
            }

            if let Some(pattern) = &edit.pattern {
                pairs.push((format!("{field}.pattern"), pattern.clone()));
            }

            if edit.required {
                pairs.push((format!("{field}.required"), "on".to_owned()));
            }
        }

        pairs
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Resolves `fields` under `edits` into rules, and reports every bad pattern.
///
/// An edit that names a field but carries no pattern means the input was
/// disabled, which is how a premade kind shows its built-in regex read-only.
/// The field's own pattern stands in, then the kind's.
///
/// Any attribute at all makes the edit answer for the field's required flag,
/// because the form posts every input of a row together and an unchecked box
/// posts nothing. A caller that builds an edit by hand supplies the checkbox
/// too, or the field turns optional.
pub(super) fn rules(fields: &[&Field], edits: &Edits) -> (Vec<Rule>, Vec<PatternError>) {
    let mut rules = Vec::with_capacity(fields.len());
    let mut errors = Vec::new();

    for field in fields {
        let edit = edits.0.get(&field.name);

        let kind = edit.and_then(|edit| edit.kind).unwrap_or(field.kind);

        let pattern = edit
            .and_then(|edit| edit.pattern.clone())
            .or_else(|| field.pattern.clone())
            .or_else(|| kind.pattern().map(str::to_owned));

        let required = edit.map_or(field.required, |edit| edit.required);

        let name = edit
            .and_then(|edit| edit.name.clone())
            .unwrap_or_else(|| field.name.clone());

        let regex = match pattern {
            Some(pattern) => match Regex::new(&pattern) {
                Ok(regex) => Some(regex),
                Err(error) => {
                    errors.push(PatternError {
                        field: field.name.to_owned(),
                        message: error.to_string(),
                    });

                    None
                }
            },
            None => {
                errors.push(PatternError {
                    field: field.name.to_owned(),
                    message: "no pattern".to_owned(),
                });

                None
            }
        };

        rules.push(Rule {
            name,
            part: field.part,
            required,
            regex,
        });
    }

    (rules, errors)
}

/// Reports how `title` moved between the saved rules and the edited ones.
///
/// The segments come from `after` when the edit claims the title, and from
/// `before` otherwise, so a removed title keeps the highlighting the edit
/// gives up. That is what [`Diff::Removed`] promises the reader.
pub(super) fn diff<'a>(
    before: &[Rule],
    after: &[Rule],
    title: &'a str,
) -> (Diff, Vec<Segment<'a>>) {
    let was = spans(before, title);
    let now = spans(after, title);

    let diff = match (was.is_some(), now.is_some()) {
        (false, true) => Diff::Added,
        (true, false) => Diff::Removed,
        (true, true) => Diff::Kept,
        (false, false) => Diff::Excluded,
    };

    let claimed = now.or(was).unwrap_or_default();

    (diff, segments(title, claimed))
}

/// Returns where each rule matched in `title`, or nothing when a required
/// rule missed.
fn spans(rules: &[Rule], title: &str) -> Option<Vec<(Range<usize>, Part)>> {
    let mut spans = Vec::new();

    for rule in rules {
        // The capture group carries the rule's name by convention, but a
        // pattern written without one still works through group 1.
        let matched = rule.regex.as_ref().and_then(|regex| {
            regex
                .captures(title)
                .and_then(|caps| caps.name(&rule.name).or_else(|| caps.get(1)))
        });

        match matched {
            Some(found) => spans.push((found.range(), rule.part)),
            None if rule.required => return None,
            None => {}
        }
    }

    Some(spans)
}

/// Cuts `title` into claimed and unclaimed runs.
///
/// Two rules sometimes claim overlapping text. The one that starts earlier
/// keeps its run and the later one drops out, because a character belongs to
/// one part and a twice-rendered character breaks the title.
fn segments(title: &str, mut spans: Vec<(Range<usize>, Part)>) -> Vec<Segment<'_>> {
    spans.sort_by_key(|(range, _)| range.start);

    let mut segments = Vec::new();
    let mut cut = 0;

    for (range, part) in spans {
        if range.start < cut {
            continue;
        }

        if range.start > cut {
            segments.push(Segment {
                text: &title[cut..range.start],
                part: None,
            });
        }

        cut = range.end;
        segments.push(Segment {
            text: &title[range.clone()],
            part: Some(part),
        });
    }

    if cut < title.len() {
        segments.push(Segment {
            text: &title[cut..],
            part: None,
        });
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::{Edits, PatternError, diff, rules};
    use crate::ruleset::{
        Diff, Field, FieldKind,
        FieldKind::{Season, Text},
        Part,
        Part::{Season as SeasonPart, Show},
    };

    fn field(
        name: &str,
        part: Part,
        kind: FieldKind,
        pattern: Option<&str>,
        required: bool,
        identity: bool,
    ) -> Field {
        Field {
            name: name.to_owned(),
            part,
            kind,
            pattern: pattern.map(ToOwned::to_owned),
            required,
            identity,
        }
    }

    /// The saved fields every edit below is measured against.
    fn declared() -> Vec<Field> {
        vec![
            field(
                "show",
                Show,
                Text,
                Some(r"^(?<show>[\w.]+?)\.S\d"),
                true,
                true,
            ),
            field("season", SeasonPart, Season, None, true, true),
        ]
    }

    fn resolved(fields: &[Field]) -> Vec<&Field> {
        fields.iter().collect()
    }

    const TITLE: &str = "The.Hollow.Meridian.S04E06.1080p";

    /// The rules the saved fields produce, with no edit applied.
    fn saved(fields: &[&Field]) -> Vec<super::Rule> {
        rules(fields, &Edits::default()).0
    }

    #[test]
    fn parse_reads_each_attribute_and_ignores_the_editor_controls() {
        let edits =
            Edits::parse("show.pattern=%5Ecoast&show.required=on&season.kind=season&diff=new");

        assert_eq!(
            edits.pairs(),
            [
                ("season.kind".to_owned(), "season".to_owned()),
                ("show.pattern".to_owned(), "^coast".to_owned()),
                ("show.required".to_owned(), "on".to_owned()),
            ],
            "a key with no dot is an editor control, not a field"
        );
    }

    #[test]
    fn pairs_round_trip_through_parse() {
        let edits =
            Edits::parse("show.name=title&show.kind=text&show.pattern=%5Ea&show.required=on");

        let encoded = edits
            .pairs()
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");

        assert_eq!(Edits::parse(&encoded), edits);
    }

    #[test]
    fn an_edited_pattern_claims_what_the_saved_one_refuses() {
        let declared = declared();
        let fields = resolved(&declared);

        // A space-separated title, which the saved show pattern never crosses.
        let spaced = "Ashfall Ridge S02E01";

        let edits = Edits::parse(
            "show.pattern=%5E(%3F%3Cshow%3E%5B%5Cw+%20%5D%2B%3F)%20S%5Cd&show.required=on",
        );
        let (edited, errors) = rules(&fields, &edits);

        assert_eq!(errors, Vec::new(), "both patterns compile");
        assert_eq!(
            diff(&saved(&fields), &edited, spaced).0,
            Diff::Added,
            "the saved pattern needs a dot before the season, the edit a space"
        );
        assert_eq!(
            diff(&saved(&fields), &edited, "Ashfall.County.S01E10").0,
            Diff::Removed,
            "the edit gives up the dot-separated names the saved pattern claims"
        );
    }

    #[test]
    fn a_broken_pattern_claims_nothing() {
        let declared = declared();
        let fields = resolved(&declared);

        let edits = Edits::parse("show.pattern=(&show.required=on");
        let (edited, errors) = rules(&fields, &edits);

        assert_eq!(
            errors.iter().map(|error| &error.field).collect::<Vec<_>>(),
            ["show"],
            "the message names the field the reader is typing in"
        );
        assert_eq!(
            diff(&saved(&fields), &edited, TITLE).0,
            Diff::Removed,
            "a required rule with no regex turns every title away"
        );
    }

    /// A text field with no pattern, which no kind stands in for.
    fn bare() -> Field {
        field("bare", Show, Text, None, true, false)
    }

    #[test]
    fn a_field_with_no_pattern_at_all_is_an_error() {
        let bare = bare();
        let fields = [&bare];

        assert_eq!(
            rules(&fields, &Edits::default()).1,
            [PatternError {
                field: "bare".to_owned(),
                message: "no pattern".to_owned(),
            }],
            "a text kind supplies no pattern of its own"
        );
    }

    #[test]
    fn diff_names_each_of_the_four_states() {
        let declared = declared();
        let fields = resolved(&declared);

        let narrowed = rules(
            &fields,
            &Edits::parse(
                "show.pattern=%5E(%3F%3Cshow%3EThe%5C.Hollow%5C.Meridian)%5C.S%5Cd&show.required=on",
            ),
        )
        .0;

        let states: Vec<Diff> = [
            TITLE,
            "Ashfall.County.S01E10",
            "just some words",
            "The.Hollow.Meridian.S04E06.720p",
        ]
        .iter()
        .map(|title| diff(&saved(&fields), &narrowed, title).0)
        .collect();

        assert_eq!(
            states,
            [Diff::Kept, Diff::Removed, Diff::Excluded, Diff::Kept],
            "kept, removed, excluded, then kept again"
        );
    }

    #[test]
    fn segments_reproduce_the_title_and_tint_each_claimed_run() {
        let declared = declared();
        let fields = resolved(&declared);

        let (_, segments) = diff(&saved(&fields), &saved(&fields), TITLE);

        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.text)
                .collect::<String>(),
            TITLE,
            "the runs in order are the title itself"
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.text, segment.part))
                .collect::<Vec<_>>(),
            [
                ("The.Hollow.Meridian", Some(Show)),
                (".S", None),
                ("04", Some(SeasonPart)),
                ("E06.1080p", None),
            ],
            "a run covers the captured group, so the pattern's anchor text stays untinted"
        );
    }

    #[test]
    fn an_overlapping_span_is_dropped() {
        let declared = declared();
        let fields = resolved(&declared);

        let edits = Edits::parse(
            "show.pattern=%5E(%3F%3Cshow%3EThe%5C.Hollow)&season.pattern=(%3F%3Cseason%3EHollow)",
        );
        let (edited, _) = rules(&fields, &edits);
        let (_, segments) = diff(&edited, &edited, TITLE);

        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.text, segment.part))
                .collect::<Vec<_>>(),
            [("The.Hollow", Some(Show)), (".Meridian.S04E06.1080p", None),],
            "the season span starts inside the show span, so it is dropped"
        );
    }
}
