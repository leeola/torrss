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

// `compose` is imported directly rather than through its module, because the
// `rules` function below takes that name in this scope.
use crate::parser::{Field, FieldKind, Segment};
use crate::rules::{Component, compose};
use crate::ruleset::{Condition, Diff};

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
#[derive(Debug)]
pub(super) struct Rule {
    name: String,

    /// Where the field sits among the ruleset's resolved fields.
    ///
    /// The position is what tints the run this rule claims, and it anchors
    /// the field's row in the editor.
    position: usize,

    /// What the captured text converts to before anything compares it.
    ///
    /// A saved test asserts the normalized value, which is also what the
    /// identity stores, so the kind has to reach [`values`].
    kind: FieldKind,

    required: bool,

    /// Whether the next rule's run starts where this one ends, as
    /// [`Field::tight`] defines it.
    tight: bool,
}

/// Every field of one draft, composed into the regex it matches with.
///
/// The regex is [`None`] when any pattern failed to compile or none was
/// available at all, so a draft the reader is still fixing claims nothing
/// rather than claiming everything.
///
/// The default claims nothing too, which is what an unsaved ruleset offers as
/// the rules an edit is measured against.
#[derive(Debug, Default)]
pub(super) struct Rules {
    pub(super) fields: Vec<Rule>,
    regex: Option<Regex>,

    /// Every comparison the draft makes on a value its regex read.
    ///
    /// One that names a field the draft does not carry claims nothing, which
    /// is the same refusal the engine makes of a saved ruleset.
    conditions: Vec<Condition>,
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
/// The title and the segments both borrow it, so a match costs no copy of the
/// name it describes.
#[derive(Debug)]
pub(super) struct Match<'a> {
    pub(super) id: i64,

    /// The whole title, which a reader saving this row as a test names.
    pub(super) title: &'a str,

    pub(super) segments: Vec<Segment<'a>>,

    /// What each claiming rule read, normalized, which is what a test
    /// saved from this row asserts.
    pub(super) values: Vec<(String, String)>,

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
///
/// An edit carries no tight flag. Every caller passes [`Edits::default`], and
/// the live draft reaches here as fields `RulesetForm::parse_draft` already
/// read, so the flag arrives on the field rather than beside it.
pub(super) fn rules(
    fields: &[&Field],
    conditions: &[Condition],
    edits: &Edits,
) -> (Rules, Vec<PatternError>) {
    let mut resolved = Vec::with_capacity(fields.len());
    let mut errors = Vec::new();

    for (position, field) in fields.iter().enumerate() {
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

        let Some(pattern) = pattern else {
            errors.push(PatternError {
                field: field.name.to_owned(),
                message:
                    "no pattern. A template leaves one blank for the ruleset based on it to fill."
                        .to_owned(),
            });

            continue;
        };

        // Each component compiles alone first, so a bad regex reports under
        // the field that carries it rather than under the whole draft.
        if let Err(error) = Regex::new(&compose(&[Component {
            name: &name,
            pattern: &pattern,
            required,
            tight: field.tight,
        }])) {
            errors.push(PatternError {
                field: field.name.to_owned(),
                message: error.to_string(),
            });

            continue;
        }

        resolved.push((
            Rule {
                name,
                position,
                kind,
                required,
                tight: field.tight,
            },
            pattern,
        ));
    }

    // A field that failed above never reaches the composition, so the whole
    // compiles only when every part stands.
    let regex = errors.is_empty().then(|| {
        let components = resolved
            .iter()
            .map(|(rule, pattern)| Component {
                name: &rule.name,
                pattern,
                required: rule.required,
                tight: rule.tight,
            })
            .collect::<Vec<_>>();

        Regex::new(&compose(&components))
    });

    let regex = match regex {
        Some(Ok(regex)) => Some(regex),
        Some(Err(error)) => {
            // A `PatternError` names one field, and two fields that write one
            // group name are equally to blame. The first is where a reader
            // starts reading.
            errors.push(PatternError {
                field: resolved
                    .first()
                    .map_or_else(String::new, |(rule, _)| rule.name.clone()),
                message: format!("the fields do not compose into one regex: {error}"),
            });

            None
        }
        None => None,
    };

    (
        Rules {
            fields: resolved.into_iter().map(|(rule, _)| rule).collect(),
            regex,
            conditions: conditions.to_vec(),
        },
        errors,
    )
}

/// What one title became under the saved rules and the edited ones.
pub(super) struct Diffed<'a> {
    pub(super) diff: Diff,
    pub(super) segments: Vec<Segment<'a>>,

    /// Each claiming rule's name and the normalized text it read.
    pub(super) values: Vec<(String, String)>,
}

/// Reports how `title` moved between the saved rules and the edited ones.
///
/// The segments and the values both come from `after` when the edit claims
/// the title, and from `before` otherwise, so a removed title keeps the
/// highlighting and the values the edit gives up. That is what
/// [`Diff::Removed`] promises the reader.
pub(super) fn diff<'a>(before: &Rules, after: &Rules, title: &'a str) -> Diffed<'a> {
    let was = captures(before, title);
    let now = captures(after, title);

    let diff = match (was.is_some(), now.is_some()) {
        (false, true) => Diff::Added,
        (true, false) => Diff::Removed,
        (true, true) => Diff::Kept,
        (false, false) => Diff::Excluded,
    };

    let claimed = now.or(was).unwrap_or_default();

    Diffed {
        diff,
        values: read(title, &claimed),
        segments: segments(title, claimed),
    }
}

/// Returns each rule's name and the normalized text it read from `title`, or
/// nothing when the rules do not claim it.
///
/// The value is normalized rather than raw, because that is the form a saved
/// test asserts and the form the identity stores. A test on the raw capture
/// passes on a title the library files elsewhere.
pub(super) fn values(rules: &Rules, title: &str) -> Option<Vec<(String, String)>> {
    Some(read(title, &captures(rules, title)?))
}

/// Where one rule landed in a title.
struct Capture<'a> {
    rule: &'a Rule,
    range: Range<usize>,
}

/// Returns where each rule matched in `title`, or nothing when the draft does
/// not claim it.
///
/// One match answers for every rule. A required rule is a plain group, so the
/// regex fails without it. An optional one is a skippable group that yields no
/// capture when it skips.
///
/// The conditions run after the regex, over the normalized values, so a title
/// the reader's comparisons refuse reads as unclaimed everywhere the editor
/// shows it.
fn captures<'a>(rules: &'a Rules, title: &str) -> Option<Vec<Capture<'a>>> {
    let caps = rules.regex.as_ref()?.captures(title)?;

    let captured = rules
        .fields
        .iter()
        .filter_map(|rule| {
            let found = caps.name(&rule.name)?;

            Some(Capture {
                rule,
                range: found.range(),
            })
        })
        .collect::<Vec<_>>();

    for condition in &rules.conditions {
        let kind = rules
            .fields
            .iter()
            .find(|rule| rule.name == condition.field)?
            .kind;

        let read = captured
            .iter()
            .find(|capture| capture.rule.name == condition.field)
            .map(|capture| kind.normalize(&title[capture.range.clone()]));

        if !condition.holds(kind, read.as_deref()) {
            return None;
        }
    }

    Some(captured)
}

/// Reads each capture out of `title` in the form its kind produces.
fn read(title: &str, captured: &[Capture<'_>]) -> Vec<(String, String)> {
    captured
        .iter()
        .map(|capture| {
            (
                capture.rule.name.clone(),
                capture.rule.kind.normalize(&title[capture.range.clone()]),
            )
        })
        .collect()
}

/// Cuts `title` into claimed and unclaimed runs.
///
/// The groups of one regex never overlap, so every claimed run stands whole
/// once the captures are in start order.
fn segments<'a>(title: &'a str, mut captured: Vec<Capture<'_>>) -> Vec<Segment<'a>> {
    captured.sort_by_key(|capture| capture.range.start);

    let mut segments = Vec::new();
    let mut cut = 0;

    for Capture { rule, range } in captured {
        let position = rule.position;

        if range.start > cut {
            segments.push(Segment {
                text: &title[cut..range.start],
                field: None,
            });
        }

        cut = range.end;
        segments.push(Segment {
            text: &title[range.clone()],
            field: Some(position),
        });
    }

    if cut < title.len() {
        segments.push(Segment {
            text: &title[cut..],
            field: None,
        });
    }

    segments
}

#[cfg(test)]
pub(super) mod tests {
    use super::{Edits, PatternError, diff, rules, values};
    use crate::parser::{
        Field, FieldKind,
        FieldKind::{Season, Text},
    };
    use crate::ruleset::{Condition, Diff, Op};

    fn field(
        name: &str,
        kind: FieldKind,
        pattern: Option<&str>,
        required: bool,
        identity: bool,
    ) -> Field {
        Field {
            name: name.to_owned(),
            kind,
            pattern: pattern.map(ToOwned::to_owned),
            required,
            tight: true,
            identity,
        }
    }

    /// The saved fields every edit below is measured against.
    pub(in crate::server) fn declared() -> Vec<Field> {
        vec![
            field("show", Text, Some(r"^(?<show>[\w.]+?)"), true, true),
            field("season", Season, None, true, true),
        ]
    }

    pub(in crate::server) fn resolved(fields: &[Field]) -> Vec<&Field> {
        fields.iter().collect()
    }

    pub(in crate::server) const TITLE: &str = "The.Hollow.Meridian.S04E06.1080p";

    /// The rules the saved fields produce, with no edit applied.
    pub(in crate::server) fn saved(fields: &[&Field]) -> super::Rules {
        rules(fields, &[], &Edits::default()).0
    }

    #[test]
    fn values_are_normalized_by_kind() {
        let declared = declared();
        let fields = resolved(&declared);

        assert_eq!(
            values(&saved(&fields), TITLE),
            Some(vec![
                ("show".to_owned(), "the hollow meridian".to_owned()),
                ("season".to_owned(), "4".to_owned()),
            ]),
            "the show collapses its dots and the season drops its leading zero"
        );
        assert_eq!(
            values(&saved(&fields), "just some words"),
            None,
            "a title the rules do not claim reads no value at all"
        );
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

        // A space-separated title. The saved show component never crosses a
        // space, so it never reaches the season component that follows it.
        let spaced = "Ashfall Ridge S02E01";

        let edits =
            Edits::parse("show.pattern=%5E(%3F%3Cshow%3E%5B%5Cw%20%5D%2B%3F)&show.required=on");
        let (edited, errors) = rules(&fields, &[], &edits);

        assert_eq!(errors, Vec::new(), "both patterns compile");
        assert_eq!(
            diff(&saved(&fields), &edited, spaced).diff,
            Diff::Added,
            "the saved pattern needs a dot before the season, the edit a space"
        );
        assert_eq!(
            diff(&saved(&fields), &edited, "Ashfall.County.S01E10").diff,
            Diff::Removed,
            "the edit gives up the dot-separated names the saved pattern claims"
        );
    }

    #[test]
    fn a_broken_pattern_claims_nothing() {
        let declared = declared();
        let fields = resolved(&declared);

        let edits = Edits::parse("show.pattern=(&show.required=on");
        let (edited, errors) = rules(&fields, &[], &edits);

        assert_eq!(
            errors.iter().map(|error| &error.field).collect::<Vec<_>>(),
            ["show"],
            "the message names the field the reader is typing in"
        );
        assert_eq!(
            diff(&saved(&fields), &edited, TITLE).diff,
            Diff::Removed,
            "a required rule with no regex turns every title away"
        );
    }

    /// A text field with no pattern, which no kind stands in for.
    fn bare() -> Field {
        field("bare", Text, None, true, false)
    }

    #[test]
    fn a_field_with_no_pattern_at_all_is_an_error() {
        let bare = bare();
        let fields = [&bare];

        assert_eq!(
            rules(&fields, &[], &Edits::default()).1,
            [PatternError {
                field: "bare".to_owned(),
                message:
                    "no pattern. A template leaves one blank for the ruleset based on it to fill."
                        .to_owned(),
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
            &[],
            &Edits::parse(
                "show.pattern=%5E(%3F%3Cshow%3EThe%5C.Hollow%5C.Meridian)&show.required=on",
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
        .map(|title| diff(&saved(&fields), &narrowed, title).diff)
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

        let segments = diff(&saved(&fields), &saved(&fields), TITLE).segments;

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
                .map(|segment| (segment.text, segment.field))
                .collect::<Vec<_>>(),
            [
                ("The.Hollow.Meridian", Some(0)),
                (".S", None),
                ("04", Some(1)),
                ("E06.1080p", None),
            ],
            "a run covers the captured group, so the pattern's anchor text stays untinted"
        );
    }

    #[test]
    fn a_condition_refuses_a_title_the_regex_reads() {
        let declared = declared();
        let fields = resolved(&declared);

        let narrowed = rules(
            &fields,
            &[Condition {
                field: "season".to_owned(),
                op: Op::Equals,
                value: "5".to_owned(),
            }],
            &Edits::default(),
        )
        .0;

        assert_eq!(
            values(&narrowed, TITLE),
            None,
            "the regex reads the fourth season, and the condition wants the fifth"
        );
        assert_eq!(
            diff(&saved(&fields), &narrowed, TITLE).diff,
            Diff::Removed,
            "so the edit gives up a title the saved rules claimed"
        );
    }
}
