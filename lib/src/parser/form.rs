//! What a parser editor posts, and what it becomes.
//!
//! The field list grows and shrinks in the browser, so the number of rows a
//! submission carries is not known ahead of time. A typed form struct needs
//! that number. This reads the rows out of the raw body instead.
//!
//! Rows are keyed `field.{i}.attribute` and `test.{i}.title`. The index
//! orders them and nothing more, so a row the reader removed leaves a gap
//! rather than renumbering every row after it.
//!
//! The ruleset editor posts the same field and test rows inside a larger
//! body, so [`crate::ruleset::form`] reads them through the row types and
//! resolvers here and adds only what a ruleset carries on top. Both editors
//! re-render their rows through that one reader, so a parser body reaches it
//! as a form that carries no condition.

use std::collections::{BTreeMap, BTreeSet};

use snafu::{OptionExt, Snafu, ensure};
use url::form_urlencoded;

use super::{Field, FieldKind, Preset, TitleTest};

/// A parser as its editor's form describes it.
///
/// The id is absent because the form never carries one. A create derives it
/// from the name through [`unique_slug`], and a save already knows it from
/// the route.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParserForm {
    pub(crate) name: String,
    pub(crate) fields: Vec<Field>,
    pub(crate) tests: Vec<TitleTest>,
}

/// The rows a parser editor holds, kept exactly as the form posted them.
///
/// A row the reader just added has a blank name and names no kind yet.
/// [`ParserForm::parse`] drops it, because a stored parser carries no
/// nameless field. The row shard reads this instead, so the row the reader
/// asked for appears.
///
/// A blank parser name is no error here either. The new page has none until
/// the reader types one, and the rows list before that.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParserRows {
    pub(crate) fields: Vec<Field>,
    pub(crate) tests: Vec<TitleTest>,
}

impl ParserRows {
    /// Reads every row a form-encoded body carries, refusing none of them.
    pub(crate) fn parse(body: &str) -> Self {
        let posted = read(body);

        Self {
            fields: posted.rows.into_values().map(draft_field).collect(),
            tests: posted.tests.into_values().map(draft_test).collect(),
        }
    }
}

/// A posted parser body sorted into its parts, before anything judges it.
#[derive(Default)]
struct Posted {
    name: String,

    /// Ordered by index, so the fields come out in the order the editor
    /// showed them however the browser ordered the pairs.
    rows: BTreeMap<usize, Row>,

    tests: BTreeMap<usize, TestRow>,
}

impl ParserForm {
    /// Reads a parser out of a form-encoded body.
    ///
    /// # Errors
    ///
    /// Returns the first row that names a type this build does not know, or
    /// that leaves a pattern empty where the type supplies none. A parser has
    /// nothing to fill a blank in, so every field writes its own regex.
    ///
    /// Returns a refusal when two rows name one field. The name keys the
    /// value the parser reads and the test columns, so two rows named alike
    /// are one field carrying two patterns.
    ///
    /// A row with an empty name is skipped rather than refused, because that
    /// is what an added row looks like before the reader fills it in. The
    /// editor lists such a row through `EditorRows`, which keeps it.
    pub(crate) fn parse(body: &str) -> Result<Self, FormError> {
        let form = Self::parse_draft(body)?;
        ensure!(!form.name.is_empty(), EmptyNameSnafu);

        Ok(form)
    }

    /// Reads the draft the editor holds, which carries a name only once the
    /// reader types one.
    ///
    /// The name has no part in a compare. The rules come from the fields and
    /// the verdicts from the tests, so a blank name is a save's concern
    /// rather than this one's.
    ///
    /// # Errors
    ///
    /// Returns the same refusals [`Self::parse`] does, less the empty name.
    pub(crate) fn parse_draft(body: &str) -> Result<Self, FormError> {
        let posted = read(body);

        let fields = posted
            .rows
            .into_values()
            .filter(|row| !row.name.trim().is_empty())
            .map(field)
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen = BTreeSet::new();
        for field in &fields {
            ensure!(
                seen.insert(field.name.as_str()),
                DuplicateNameSnafu { field: &field.name }
            );
        }

        Ok(Self {
            name: posted.name.trim().to_owned(),
            fields,
            // A blank title is a row the reader added and has not filled in,
            // and a blank expectation is an input they left alone. Neither
            // asserts that the value is empty.
            tests: posted
                .tests
                .into_values()
                .filter(|row| !row.title.trim().is_empty())
                .map(|row| TitleTest {
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

    /// Writes the pairs a browser posts for this parser.
    ///
    /// The inverse of [`Self::parse`], which the editor uses to seed the
    /// draft its live re-render reads. A pattern is written only when the
    /// kind supplies none, because that is what a form actually sends and the
    /// draft has to start where the browser takes over.
    pub(crate) fn encode(&self) -> String {
        let mut pairs = form_urlencoded::Serializer::new(String::new());

        pairs.append_pair("name", &self.name);

        for (index, field) in self.fields.iter().enumerate() {
            encode_field(&mut pairs, index, field);
        }

        for (index, test) in self.tests.iter().enumerate() {
            encode_test(&mut pairs, index, test);
        }

        pairs.finish()
    }
}

/// Writes the pairs one field row carries, under the `field.{index}.` prefix
/// a whole form adds.
pub(crate) fn encode_field(
    pairs: &mut form_urlencoded::Serializer<'_, String>,
    index: usize,
    field: &Field,
) {
    pairs.append_pair(&format!("field.{index}.name"), &field.name);
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

    if field.tight {
        pairs.append_pair(&format!("field.{index}.tight"), "on");
    }
}

/// Writes the pairs one saved test carries, under the `test.{index}.` prefix.
pub(crate) fn encode_test(
    pairs: &mut form_urlencoded::Serializer<'_, String>,
    index: usize,
    test: &TitleTest,
) {
    pairs.append_pair(&format!("test.{index}.title"), &test.title);

    for (field, expected) in &test.expected {
        pairs.append_pair(&format!("test.{index}.expect.{field}"), expected);
    }
}

/// Resolves one posted test row into a saved test, refusing nothing.
///
/// A row the reader just added carries a blank title, which the editor lists
/// and a save drops.
pub(crate) fn draft_test(row: TestRow) -> TitleTest {
    TitleTest {
        title: row.title.trim().to_owned(),
        expected: row.expected,
    }
}

/// Sorts a form-encoded body into its parts, judging none of them.
fn read(body: &str) -> Posted {
    let mut posted = Posted::default();

    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        if key == "name" {
            posted.name = value.into_owned();
            continue;
        }

        read_test(&mut posted.tests, &key, &value);
        read_row(&mut posted.rows, &key, &value);
    }

    posted
}

/// Why a posted parser or ruleset is not one.
///
/// Every variant names what the reader has to change, because the message
/// reaches them as the body of a 400.
///
/// One type serves both editors, because they post the same field and test
/// rows. The last two variants describe a condition, which only a ruleset
/// writes.
#[derive(Debug, PartialEq, Eq, Snafu)]
#[snafu(visibility(pub(crate)))]
pub(crate) enum FormError {
    #[snafu(display("the name is empty"))]
    EmptyName,

    #[snafu(display("no field type is named {kind}"))]
    UnknownKind { kind: String },

    #[snafu(display("field {field} needs a pattern, because its type supplies none"))]
    MissingPattern { field: String },

    #[snafu(display("two fields are named {field}"))]
    DuplicateName { field: String },

    #[snafu(display("no condition is named {op}"))]
    UnknownOp { op: String },

    #[snafu(display("the condition on {field} needs a value"))]
    MissingValue { field: String },

    #[snafu(display("the ruleset needs a parser"))]
    MissingParser,
}

/// One field row as the form posted it, before it becomes a [`Field`].
///
/// Every attribute is a string here, because a form posts text. The checkbox
/// flags are `bool` already. A checkbox posts its name only when checked, so
/// absence is `false` rather than missing.
#[derive(Default)]
pub(crate) struct Row {
    pub(crate) name: String,
    kind: String,
    pattern: Option<String>,
    required: bool,
    identity: bool,
    tight: bool,
}

/// One test row as the form posted it, before it becomes a [`TitleTest`].
///
/// Keyed `test.{index}.title` and `test.{index}.expect.{field}`, so a reader
/// names as few fields as they mean to assert.
#[derive(Default)]
pub(crate) struct TestRow {
    pub(crate) title: String,
    pub(crate) expected: BTreeMap<String, String>,
}

/// Writes the pairs one field row carries, for a preset menu to hand over.
///
/// The keys are the ones [`read_row`] reads, without the `field.{index}.`
/// prefix a whole form adds. So a row built from these parses as if the
/// reader had typed it, and the script that builds it copies pairs and knows
/// no preset.
pub(crate) fn encode_preset(preset: &Preset) -> String {
    let mut pairs = form_urlencoded::Serializer::new(String::new());

    pairs.append_pair("name", preset.name);
    pairs.append_pair("kind", preset.kind.label());

    if let Some(pattern) = preset.pattern {
        pairs.append_pair("pattern", pattern);
    }

    if preset.required {
        pairs.append_pair("required", "on");
    }

    if preset.identity {
        pairs.append_pair("identity", "on");
    }

    if preset.tight {
        pairs.append_pair("tight", "on");
    }

    pairs.finish()
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
pub(crate) fn read_test(tests: &mut BTreeMap<usize, TestRow>, key: &str, value: &str) {
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
pub(crate) fn read_row(rows: &mut BTreeMap<usize, Row>, key: &str, value: &str) {
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
        "kind" => row.kind = value.to_owned(),
        "pattern" => row.pattern = Some(value.to_owned()),
        "required" => row.required = true,
        "identity" => row.identity = true,
        "tight" => row.tight = true,
        _ => {}
    }
}

/// The pattern a row stores for itself, or nothing when it stores none.
///
/// A premade kind carries its own regex, so a row of that kind keeps no copy
/// of what the editor rendered. A blank pattern is no pattern.
pub(crate) fn own_pattern(kind: FieldKind, pattern: Option<String>) -> Option<String> {
    if kind.pattern().is_some() {
        return None;
    }

    pattern.filter(|pattern| !pattern.trim().is_empty())
}

/// Resolves one posted row into a field, refusing nothing.
///
/// A kind this build does not know falls back to the first option its select
/// renders, which is what a row that named none posted.
pub(crate) fn draft_field(row: Row) -> Field {
    let kind = FieldKind::from_label(&row.kind).unwrap_or(FieldKind::Text);

    Field {
        name: row.name.trim().to_owned(),
        kind,
        pattern: own_pattern(kind, row.pattern),
        required: row.required,
        tight: row.tight,
        identity: row.identity,
    }
}

/// Resolves one posted row into a field.
///
/// The pattern is dropped when the kind supplies one, so a premade kind keeps
/// its built-in regex rather than storing a copy the editor rendered.
///
/// A field with no pattern reads no value, so an empty one is refused here
/// rather than compiled into a regex that matches everything.
pub(crate) fn field(row: Row) -> Result<Field, FormError> {
    let kind = FieldKind::from_label(&row.kind).context(UnknownKindSnafu { kind: &row.kind })?;

    let pattern = own_pattern(kind, row.pattern);

    ensure!(
        pattern.is_some() || kind.pattern().is_some(),
        MissingPatternSnafu { field: &row.name }
    );

    Ok(Field {
        name: row.name.trim().to_owned(),
        kind,
        pattern,
        required: row.required,
        tight: row.tight,
        identity: row.identity,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FormError, ParserForm, encode_preset, slug, unique_slug};
    use crate::parser::{Field, FieldKind, PRESETS, TitleTest};

    fn text(name: &str, pattern: &str, required: bool, identity: bool) -> Field {
        Field {
            name: name.to_owned(),
            kind: FieldKind::Text,
            pattern: Some(pattern.to_owned()),
            required,
            tight: false,
            identity,
        }
    }

    #[test]
    fn rows_come_out_in_index_order_across_a_gap() {
        assert_eq!(
            ParserForm::parse(
                "name=Series\
                 &field.2.name=season&field.2.kind=text&field.2.pattern=S%5Cd%2B\
                 &field.0.name=show&field.0.kind=text&field.0.pattern=%5E.%2B\
                 &field.0.required=on&field.0.identity=on&field.0.tight=on",
            ),
            Ok(ParserForm {
                name: "Series".to_owned(),
                fields: vec![
                    Field {
                        tight: true,
                        ..text("show", "^.+", true, true)
                    },
                    text("season", r"S\d+", false, false),
                ],
                tests: Vec::new(),
            }),
            "the index orders the rows, and an absent checkbox reads false"
        );
    }

    #[test]
    fn a_row_with_no_name_is_skipped() {
        let form = ParserForm::parse(
            "name=Series\
             &field.0.name=show&field.0.kind=text&field.0.pattern=%5E.%2B\
             &field.1.name=&field.1.kind=text&field.1.pattern=",
        )
        .expect("the body parses");

        assert_eq!(
            form.fields,
            vec![text("show", "^.+", false, false)],
            "an added row the reader never filled in is not a field"
        );
    }

    #[test]
    fn tests_read_the_title_and_each_expected_value() {
        let form = ParserForm::parse(
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
                TitleTest {
                    title: "The.Hollow.Meridian.S04E06".to_owned(),
                    expected: BTreeMap::from([("season".to_owned(), "4".to_owned())]),
                },
                TitleTest {
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
        let form = ParserForm::parse(
            "name=Series&field.0.name=season&field.0.kind=season\
             &field.0.pattern=whatever%20the%20editor%20rendered",
        )
        .expect("the body parses");

        assert_eq!(
            form.fields[0].pattern, None,
            "the kind's own pattern applies, so the row stores none"
        );
    }

    #[test]
    fn a_parser_form_refuses_a_blank_pattern() {
        assert_eq!(
            ParserForm::parse("name=Series&field.0.name=show&field.0.kind=text&field.0.pattern="),
            Err(FormError::MissingPattern {
                field: "show".to_owned()
            }),
            "a parser has nothing to fill a blank in, so every field writes its own regex"
        );

        assert_eq!(
            ParserForm::parse("name=Series&field.0.name=show&field.0.kind=colour"),
            Err(FormError::UnknownKind {
                kind: "colour".to_owned()
            }),
            "a field still does not compile from a type this build does not know"
        );
    }

    #[test]
    fn two_rows_with_one_name_are_refused() {
        assert_eq!(
            ParserForm::parse(
                "name=Films&field.0.name=year&field.0.kind=number&field.0.pattern=x\
                 &field.1.name=year&field.1.kind=number&field.1.pattern=y"
            ),
            Err(FormError::DuplicateName {
                field: "year".to_owned()
            }),
            "one name is one field, so a second row under it has no rule of its own"
        );

        assert!(
            ParserForm::parse(
                "name=Films&field.0.name=year&field.0.kind=number&field.0.pattern=x\
                 &field.1.name=Year&field.1.kind=number&field.1.pattern=y"
            )
            .is_ok(),
            "a name is compared as typed, because that is how the rules look it up"
        );
    }

    #[test]
    fn an_encoded_form_parses_back_to_itself() {
        let series = ParserForm {
            name: "Series Episodes".to_owned(),
            fields: vec![
                text("show", "^.+", true, true),
                Field {
                    name: "season".to_owned(),
                    kind: FieldKind::Season,
                    pattern: None,
                    required: false,
                    tight: true,
                    identity: true,
                },
            ],
            tests: vec![TitleTest {
                title: "The.Hollow.Meridian.S04E06.1080p".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
        };

        assert_eq!(
            ParserForm::parse(&series.encode()),
            Ok(series),
            "what the editor seeds its draft with is what a post reads back"
        );
    }

    #[test]
    fn a_draft_needs_no_name() {
        assert_eq!(
            ParserForm::parse(""),
            Err(FormError::EmptyName),
            "a save names what it stores"
        );
        assert_eq!(
            ParserForm::parse_draft("field.0.name=show&field.0.kind=text&field.0.pattern=.%2A")
                .map(|form| form.name),
            Ok(String::new()),
            "the rules come from the fields, so a compare runs before the reader names anything"
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
    fn a_preset_encodes_the_keys_a_row_reads() {
        assert_eq!(
            encode_preset(&PRESETS[5]),
            "name=year&kind=number&pattern=%5C.%28%3F%3Cyear%3E%28%3F%3A19%7C20%29%5Cd%7B2%7D%29\
             &required=on&identity=on",
            "the pairs are what read_row reads, minus the prefix a whole form adds"
        );

        assert_eq!(
            encode_preset(&PRESETS[6]),
            "name=resolution&kind=enum\
             &pattern=%5C.%28%3F%3Cresolution%3E480p%7C720p%7C1080p%7C2160p%29",
            "a flag left unset posts nothing, as an unchecked box does"
        );

        assert_eq!(
            encode_preset(&PRESETS[0]),
            "name=show&kind=text&pattern=%5E%28%3F%3Cshow%3E%5B%5Cw.%5D%2B%29\
             &required=on&identity=on&tight=on",
            "a tight preset posts the flag as a checked box does"
        );
    }
}
