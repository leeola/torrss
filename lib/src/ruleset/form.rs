//! What the ruleset editor posts, and what it becomes.
//!
//! The editor's field list grows and shrinks in the browser, so the number of
//! rows a submission carries is not known ahead of time. A typed form struct
//! needs that number. This reads the rows out of the raw body instead.
//!
//! Rows are keyed `field.{i}.attribute`. The index orders them and nothing
//! more, so a row the reader removed leaves a gap rather than renumbering
//! every row after it.

use std::collections::BTreeMap;

use snafu::{OptionExt, Snafu, ensure};
use url::form_urlencoded;

use super::{Field, FieldKind, Part, RulesetTest};

/// A ruleset as the editor's form describes it.
///
/// The id is absent because the form never carries one. A create derives it
/// from the name through [`unique_slug`], and a save already knows it from
/// the route.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RulesetForm {
    pub(crate) name: String,

    /// Whether this ruleset only serves as a foundation for others.
    ///
    /// A checkbox posts its name only when checked, so absence is `false`.
    pub(crate) template: bool,

    /// The template this ruleset is built on, or [`None`] for a ruleset that
    /// declares every field itself.
    ///
    /// An empty value means none. A select posts its empty option rather than
    /// omitting the key.
    pub(crate) based_on: Option<String>,

    pub(crate) fields: Vec<Field>,

    pub(crate) tests: Vec<RulesetTest>,
}

/// Why a posted ruleset is not one.
///
/// Every variant names what the reader has to change, because the message
/// reaches them as the body of a 400.
#[derive(Debug, PartialEq, Eq, Snafu)]
pub(crate) enum FormError {
    #[snafu(display("the ruleset needs a name"))]
    EmptyName,

    #[snafu(display("no part is named {part}"))]
    UnknownPart { part: String },

    #[snafu(display("no field type is named {kind}"))]
    UnknownKind { kind: String },

    #[snafu(display("field {field} needs a pattern, because its type supplies none"))]
    MissingPattern { field: String },
}

/// One field row as the form posted it, before it becomes a [`Field`].
///
/// Every attribute is a string here, because a form posts text. The checkbox
/// flags are `bool` already. A checkbox posts its name only when checked, so
/// absence is `false` rather than missing.
#[derive(Default)]
struct Row {
    name: String,
    part: String,
    kind: String,
    pattern: Option<String>,
    required: bool,
    identity: bool,
}

/// One test row as the form posted it, before it becomes a [`RulesetTest`].
///
/// Keyed `test.{index}.title` and `test.{index}.expect.{field}`, so a reader
/// names as few fields as they mean to assert.
#[derive(Default)]
struct TestRow {
    title: String,
    expected: BTreeMap<String, String>,
}

/// A posted body sorted into its parts, before anything judges it.
///
/// Both reads of a form start here and part company after. One drops what a
/// stored ruleset never carries. The other keeps every row the editor showed.
#[derive(Default)]
struct Posted {
    name: String,
    based_on: String,
    template: bool,

    /// Ordered by index, so the fields come out in the order the editor
    /// showed them however the browser ordered the pairs.
    rows: BTreeMap<usize, Row>,

    tests: BTreeMap<usize, TestRow>,
}

/// The rows the editor holds, kept exactly as the form posted them.
///
/// A row the reader just added has a blank name and names no part or kind
/// yet. [`RulesetForm::parse`] drops it, because a stored ruleset carries no
/// nameless field. The row shards read this instead, so the row the reader
/// asked for appears.
///
/// A blank ruleset name is no error here either. The new page has none until
/// the reader types one, and the rows list before that.
///
/// A row that posted no part or kind comes back under the first option each
/// select renders, which is the option the browser showed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EditorRows {
    pub(crate) based_on: Option<String>,
    pub(crate) fields: Vec<Field>,
    pub(crate) tests: Vec<RulesetTest>,
}

impl EditorRows {
    /// Reads every row a form-encoded body carries, refusing none of them.
    pub(crate) fn parse(body: &str) -> Self {
        let posted = read(body);

        Self {
            based_on: Some(posted.based_on.trim().to_owned()).filter(|id| !id.is_empty()),
            fields: posted.rows.into_values().map(draft_field).collect(),
            tests: posted
                .tests
                .into_values()
                .map(|row| RulesetTest {
                    title: row.title.trim().to_owned(),
                    expected: row.expected,
                })
                .collect(),
        }
    }
}

impl RulesetForm {
    /// Reads a ruleset out of a form-encoded body.
    ///
    /// # Errors
    ///
    /// Returns the first row that names a part or a type this build does not
    /// know, or that leaves a pattern empty where the type supplies none and
    /// the form is no template. A template keeps that blank for the ruleset
    /// built on it to fill.
    ///
    /// A row with an empty name is skipped rather than refused, because that
    /// is what an added row looks like before the reader fills it in. The
    /// editor lists such a row through [`EditorRows`], which keeps it.
    pub(crate) fn parse(body: &str) -> Result<Self, FormError> {
        let posted = read(body);

        let name = posted.name.trim().to_owned();
        ensure!(!name.is_empty(), EmptyNameSnafu);

        let template = posted.template;

        Ok(Self {
            name,
            template,
            based_on: Some(posted.based_on.trim().to_owned()).filter(|id| !id.is_empty()),
            fields: posted
                .rows
                .into_values()
                .filter(|row| !row.name.trim().is_empty())
                .map(|row| field(row, template))
                .collect::<Result<Vec<_>, _>>()?,
            // A blank title is a row the reader added and has not filled in,
            // and a blank expectation is an input they left alone. Neither
            // asserts that the value is empty.
            tests: posted
                .tests
                .into_values()
                .filter(|row| !row.title.trim().is_empty())
                .map(|row| RulesetTest {
                    title: row.title.trim().to_owned(),
                    expected: row
                        .expected
                        .into_iter()
                        .filter(|(_, value)| !value.trim().is_empty())
                        .collect(),
                })
                .collect(),
        })
    }

    /// Writes the pairs a browser posts for this ruleset.
    ///
    /// The inverse of [`Self::parse`], which the editor uses to seed the
    /// draft its live re-render reads. A checkbox is written only when set
    /// and a pattern only when the kind supplies none, because that is what
    /// a form actually sends and the draft has to start where the browser
    /// takes over.
    pub(crate) fn encode(&self) -> String {
        let mut pairs = form_urlencoded::Serializer::new(String::new());

        pairs.append_pair("name", &self.name);
        pairs.append_pair("based_on", self.based_on.as_deref().unwrap_or_default());

        if self.template {
            pairs.append_pair("template", "on");
        }

        for (index, field) in self.fields.iter().enumerate() {
            pairs.append_pair(&format!("field.{index}.name"), &field.name);
            pairs.append_pair(&format!("field.{index}.part"), field.part.slug());
            pairs.append_pair(&format!("field.{index}.kind"), field.kind.label());

            if let Some(pattern) = &field.pattern {
                pairs.append_pair(&format!("field.{index}.pattern"), pattern);
            }

            if field.required {
                pairs.append_pair(&format!("field.{index}.required"), "on");
            }

            if field.identity {
                pairs.append_pair(&format!("field.{index}.identity"), "on");
            }
        }

        for (index, test) in self.tests.iter().enumerate() {
            pairs.append_pair(&format!("test.{index}.title"), &test.title);

            for (field, expected) in &test.expected {
                pairs.append_pair(&format!("test.{index}.expect.{field}"), expected);
            }
        }

        pairs.finish()
    }
}

/// Turns `name` into the id a ruleset carries in its URL.
///
/// Every run of characters outside the alphabet and the digits becomes one
/// `-`, so two names that differ only in punctuation reach the same slug and
/// the caller sees the collision.
pub(crate) fn slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());

    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }

    slug.trim_matches('-').to_owned()
}

/// Returns a slug of `name` that `taken` reports free, or [`None`] when the
/// name slugs to nothing.
///
/// A collision appends a counter rather than a random suffix, so the second
/// "Series Episodes" reads as `series-episodes-2` and stays guessable.
pub(crate) fn unique_slug(name: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    let base = slug(name);

    if base.is_empty() {
        return None;
    }

    if !taken(&base) {
        return Some(base);
    }

    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id))
}

/// Files one `test.{index}.title` or `test.{index}.expect.{field}` pair under
/// its test.
///
/// A key that is not a test row passes through untouched, as [`read_row`]
/// does with its own.
fn read_test(tests: &mut BTreeMap<usize, TestRow>, key: &str, value: &str) {
    let Some(rest) = key.strip_prefix("test.") else {
        return;
    };

    let Some((index, attribute)) = rest.split_once('.') else {
        return;
    };

    let Ok(index) = index.parse::<usize>() else {
        return;
    };

    let test = tests.entry(index).or_default();

    if attribute == "title" {
        test.title = value.to_owned();
    } else if let Some(field) = attribute.strip_prefix("expect.") {
        test.expected.insert(field.to_owned(), value.to_owned());
    }
}

/// Files one `field.{index}.attribute` pair under its row.
///
/// A key that is not a field row passes through untouched. The editor's own
/// controls share the body with the field inputs.
fn read_row(rows: &mut BTreeMap<usize, Row>, key: &str, value: &str) {
    let Some(rest) = key.strip_prefix("field.") else {
        return;
    };

    let Some((index, attribute)) = rest.split_once('.') else {
        return;
    };

    let Ok(index) = index.parse::<usize>() else {
        return;
    };

    let row = rows.entry(index).or_default();

    match attribute {
        "name" => row.name = value.to_owned(),
        "part" => row.part = value.to_owned(),
        "kind" => row.kind = value.to_owned(),
        "pattern" => row.pattern = Some(value.to_owned()),
        "required" => row.required = true,
        "identity" => row.identity = true,
        _ => {}
    }
}

/// Sorts a form-encoded body into its parts, judging none of them.
fn read(body: &str) -> Posted {
    let mut posted = Posted::default();

    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "name" => posted.name = value.into_owned(),
            "based_on" => posted.based_on = value.into_owned(),
            "template" => posted.template = true,
            _ => {
                read_test(&mut posted.tests, &key, &value);
                read_row(&mut posted.rows, &key, &value);
            }
        }
    }

    posted
}

/// The pattern a row stores for itself, or nothing when it stores none.
///
/// A premade kind carries its own regex, so a row of that kind keeps no copy
/// of what the editor rendered. A blank pattern is no pattern.
fn own_pattern(kind: FieldKind, pattern: Option<String>) -> Option<String> {
    if kind.pattern().is_some() {
        return None;
    }

    pattern.filter(|pattern| !pattern.trim().is_empty())
}

/// Resolves one posted row into a field, refusing nothing.
///
/// A part or a kind this build does not know falls back to the first option
/// its select renders, which is what a row that named neither posted.
fn draft_field(row: Row) -> Field {
    let kind = FieldKind::from_label(&row.kind).unwrap_or(FieldKind::Text);

    Field {
        name: row.name.trim().to_owned(),
        part: Part::from_slug(&row.part).unwrap_or(Part::ALL[0]),
        kind,
        pattern: own_pattern(kind, row.pattern),
        required: row.required,
        identity: row.identity,
    }
}

/// Resolves one posted row into a field.
///
/// The pattern is dropped when the kind supplies one, so a premade kind keeps
/// its built-in regex rather than storing a copy the editor rendered.
///
/// A template keeps an empty pattern as a blank rather than refusing it. A
/// template names the part and the flags, and the ruleset built on it writes
/// the regex.
fn field(row: Row, template: bool) -> Result<Field, FormError> {
    let kind = FieldKind::from_label(&row.kind).context(UnknownKindSnafu { kind: &row.kind })?;
    let part = Part::from_slug(&row.part).context(UnknownPartSnafu { part: &row.part })?;

    let pattern = own_pattern(kind, row.pattern);

    if pattern.is_none() && kind.pattern().is_none() {
        ensure!(template, MissingPatternSnafu { field: &row.name });
    }

    Ok(Field {
        name: row.name.trim().to_owned(),
        part,
        kind,
        pattern,
        required: row.required,
        identity: row.identity,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{EditorRows, FormError, RulesetForm, slug, unique_slug};
    use crate::ruleset::{Field, FieldKind, Part, RulesetTest};

    fn text(name: &str, part: Part, pattern: &str, required: bool, identity: bool) -> Field {
        Field {
            name: name.to_owned(),
            part,
            kind: FieldKind::Text,
            pattern: Some(pattern.to_owned()),
            required,
            identity,
        }
    }

    #[test]
    fn rows_come_out_in_index_order_across_a_gap() {
        let form = RulesetForm::parse(
            "name=Series&based_on=\
             &field.2.name=season&field.2.part=season&field.2.kind=text&field.2.pattern=S%5Cd%2B\
             &field.0.name=show&field.0.part=show&field.0.kind=text&field.0.pattern=%5E.%2B\
             &field.0.required=on&field.0.identity=on",
        )
        .expect("the body parses");

        assert_eq!(
            form,
            RulesetForm {
                name: "Series".to_owned(),
                template: false,
                based_on: None,
                fields: vec![
                    text("show", Part::Show, "^.+", true, true),
                    text("season", Part::Season, r"S\d+", false, false),
                ],
                tests: Vec::new(),
            },
            "the index orders the rows, and an absent checkbox reads false"
        );
    }

    #[test]
    fn a_row_with_no_name_is_skipped() {
        let form = RulesetForm::parse(
            "name=Series\
             &field.0.name=show&field.0.part=show&field.0.kind=text&field.0.pattern=%5E.%2B\
             &field.1.name=&field.1.part=season&field.1.kind=text&field.1.pattern=",
        )
        .expect("the body parses");

        assert_eq!(
            form.fields,
            vec![text("show", Part::Show, "^.+", false, false)],
            "an added row the reader never filled in is not a field"
        );
    }

    #[test]
    fn tests_read_the_title_and_each_expected_value() {
        let form = RulesetForm::parse(
            "name=Series\
             &test.1.title=Coastal.Drift.2024&test.1.expect.show=coastal%20drift\
             &test.1.expect.year=\
             &test.0.title=The.Hollow.Meridian.S04E06&test.0.expect.season=4\
             &test.2.title=%20%20",
        )
        .expect("the body parses");

        assert_eq!(
            form.tests,
            vec![
                RulesetTest {
                    title: "The.Hollow.Meridian.S04E06".to_owned(),
                    expected: BTreeMap::from([("season".to_owned(), "4".to_owned())]),
                },
                RulesetTest {
                    title: "Coastal.Drift.2024".to_owned(),
                    expected: BTreeMap::from([("show".to_owned(), "coastal drift".to_owned())]),
                },
            ],
            "the index orders them, a blank expectation asserts nothing, and a blank title \
             is a row the reader never filled in"
        );
    }

    #[test]
    fn a_premade_kind_stores_no_pattern_of_its_own() {
        let form = RulesetForm::parse(
            "name=Series&field.0.name=season&field.0.part=season&field.0.kind=season\
             &field.0.pattern=whatever%20the%20editor%20rendered",
        )
        .expect("the body parses");

        assert_eq!(
            form.fields[0].pattern, None,
            "the kind's own pattern applies, so the row stores none"
        );
    }

    #[test]
    fn an_inherited_ruleset_keeps_the_parent_it_names() {
        let form = RulesetForm::parse("name=Ashfall&based_on=series").expect("the body parses");

        assert_eq!(form.based_on, Some("series".to_owned()));
        assert_eq!(
            form.fields,
            Vec::new(),
            "a ruleset on a template declares no field of its own"
        );
    }

    #[test]
    fn each_refusal_names_what_to_change() {
        assert_eq!(RulesetForm::parse("name=%20%20"), Err(FormError::EmptyName));
        assert_eq!(
            RulesetForm::parse(
                "name=Series&field.0.name=show&field.0.part=nowhere&field.0.kind=text"
            ),
            Err(FormError::UnknownPart {
                part: "nowhere".to_owned()
            })
        );
        assert_eq!(
            RulesetForm::parse(
                "name=Series&field.0.name=show&field.0.part=show&field.0.kind=colour"
            ),
            Err(FormError::UnknownKind {
                kind: "colour".to_owned()
            })
        );
        assert_eq!(
            RulesetForm::parse("name=Series&field.0.name=show&field.0.part=show&field.0.kind=text"),
            Err(FormError::MissingPattern {
                field: "show".to_owned()
            }),
            "a text field carries its own pattern or reads nothing"
        );
    }

    #[test]
    fn a_template_keeps_a_blank_pattern() {
        const ROW: &str = "field.0.name=show&field.0.part=show&field.0.kind=text&field.0.pattern=";

        let parsed = RulesetForm::parse(&format!("name=Series&template=on&{ROW}"))
            .expect("a template declares a blank");

        assert_eq!(
            parsed
                .fields
                .iter()
                .map(|field| &field.pattern)
                .collect::<Vec<_>>(),
            vec![&None],
            "the blank is what the ruleset based on this fills in"
        );
        assert_eq!(
            RulesetForm::parse(&format!("name=Series&{ROW}")),
            Err(FormError::MissingPattern {
                field: "show".to_owned()
            }),
            "only a template leaves one"
        );
    }

    #[test]
    fn a_name_slugs_to_one_dash_per_run() {
        assert_eq!(slug("The Hollow Meridian!"), "the-hollow-meridian");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn a_taken_slug_counts_up() {
        assert_eq!(
            unique_slug("Series Episodes", |id| id == "series-episodes"),
            Some("series-episodes-2".to_owned())
        );
        assert_eq!(
            unique_slug("Series Episodes", |id| id.starts_with("series-episodes")
                && id != "series-episodes-3"),
            Some("series-episodes-3".to_owned())
        );
        assert_eq!(
            unique_slug("!!!", |_| false),
            None,
            "a name that slugs to nothing names nothing"
        );
    }

    #[test]
    fn an_encoded_form_parses_back_to_itself() {
        let form = RulesetForm {
            name: "Series Episodes".to_owned(),
            template: true,
            based_on: Some("series".to_owned()),
            fields: vec![
                text("show", Part::Show, "^.+", true, true),
                Field {
                    name: "season".to_owned(),
                    part: Part::Season,
                    kind: FieldKind::Season,
                    pattern: None,
                    required: false,
                    identity: true,
                },
            ],
            tests: vec![RulesetTest {
                title: "The.Hollow.Meridian.S04E06.1080p".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
        };

        assert_eq!(
            RulesetForm::parse(&form.encode()),
            Ok(form),
            "what the editor seeds its draft with is what a post reads back"
        );
    }

    #[test]
    fn editor_rows_keep_a_blank_row_and_need_no_name() {
        assert_eq!(
            EditorRows::parse(
                "name=&field.0.name=show&field.0.part=show&field.0.kind=text\
                 &field.0.pattern=%5E.%2B&field.1.name=&test.0.title="
            ),
            EditorRows {
                based_on: None,
                fields: vec![
                    text("show", Part::Show, "^.+", false, false),
                    Field {
                        name: String::new(),
                        part: Part::Show,
                        kind: FieldKind::Text,
                        pattern: None,
                        required: false,
                        identity: false,
                    },
                ],
                tests: vec![RulesetTest {
                    title: String::new(),
                    expected: BTreeMap::new(),
                }],
            },
            "an added row lists under the first part and kind, and the page needs no name yet"
        );
    }
}
