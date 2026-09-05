//! What the ruleset editor posts, and what it becomes.
//!
//! The editor's condition and test lists grow and shrink in the browser, so
//! the number of rows a submission carries is not known ahead of time. A
//! typed form struct needs that number. This reads the rows out of the raw
//! body instead.
//!
//! Rows are keyed `condition.{i}.attribute` and `test.{i}.title`. The index
//! orders them and nothing more, so a row the reader removed leaves a gap
//! rather than renumbering every row after it.
//!
//! The fields belong to the parser the ruleset names, so
//! [`crate::parser::form`] reads the test rows and this adds the parser and
//! the conditions on top.

use std::collections::BTreeMap;

use snafu::{OptionExt, ensure};
use url::form_urlencoded;

use super::{Condition, Op};
use crate::parser::TitleTest;
use crate::parser::form::{
    FormError, MissingParserSnafu, MissingValueSnafu, TestRow, UnknownOpSnafu, draft_test,
    encode_test, read_test,
};

/// A ruleset as the editor's form describes it.
///
/// The id is absent because the form never carries one. A create derives it
/// from the name through [`crate::parser::form::unique_slug`], and a save
/// already knows it from the route.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RulesetForm {
    pub(crate) name: String,

    /// The parser this ruleset reads titles with.
    pub(crate) parser: String,

    pub(crate) conditions: Vec<Condition>,

    pub(crate) tests: Vec<TitleTest>,
}

/// One condition row as the form posted it, before it becomes a
/// [`Condition`].
///
/// Keyed `condition.{index}.field`, `condition.{index}.op`, and
/// `condition.{index}.value`. The operator is a string here because a select
/// posts text, and a row the reader just added names no field yet.
#[derive(Default)]
struct ConditionRow {
    field: String,
    op: String,
    value: String,
}

/// A posted body sorted into its parts, before anything judges it.
///
/// Both reads of a form start here and part company after. One drops what a
/// stored ruleset never carries. The other keeps every row the editor showed.
#[derive(Default)]
struct Posted {
    name: String,
    parser: String,

    conditions: BTreeMap<usize, ConditionRow>,

    tests: BTreeMap<usize, TestRow>,
}

/// The rows the editor holds, kept exactly as the form posted them.
///
/// A condition row the reader just added names no field yet.
/// [`RulesetForm::parse`] drops it, because a stored ruleset carries no
/// nameless condition. The row shards read this instead, so the row the
/// reader asked for appears.
///
/// A blank ruleset name is no error here either. The new page has none until
/// the reader types one, and the rows list before that.
///
/// A condition that posted no operator comes back under the first option
/// that select renders, which is the option the browser showed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EditorRows {
    /// The parser the draft names, or [`None`] when it names none yet.
    pub(crate) parser: Option<String>,

    pub(crate) conditions: Vec<Condition>,
    pub(crate) tests: Vec<TitleTest>,
}

impl EditorRows {
    /// Reads every row a form-encoded body carries, refusing none of them.
    pub(crate) fn parse(body: &str) -> Self {
        let posted = read(body);

        Self {
            parser: Some(posted.parser.trim().to_owned()).filter(|id| !id.is_empty()),
            conditions: posted
                .conditions
                .into_values()
                .map(|row| Condition {
                    field: row.field.trim().to_owned(),
                    op: Op::from_label(&row.op).unwrap_or(Op::Equals),
                    value: row.value,
                })
                .collect(),
            tests: posted.tests.into_values().map(draft_test).collect(),
        }
    }
}

impl RulesetForm {
    /// Reads a ruleset out of a form-encoded body.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the body names no parser. A ruleset with none
    /// reads no title, so there is nothing for its conditions to judge.
    ///
    /// A blank name is accepted. The write infers one from the conditions
    /// through [`crate::ruleset::inferred_name`], so the reader names a
    /// ruleset once or not at all.
    ///
    /// Returns a refusal for a condition naming an operator this build does
    /// not know, or leaving its value empty under an operator that compares
    /// one.
    ///
    /// A condition row with no field is skipped rather than refused, because
    /// that is what an added row looks like before the reader fills it in.
    /// The editor lists such a row through [`EditorRows`], which keeps it.
    pub(crate) fn parse(body: &str) -> Result<Self, FormError> {
        let form = Self::parse_draft(body)?;
        ensure!(!form.parser.is_empty(), MissingParserSnafu);

        Ok(form)
    }

    /// Reads the draft the editor holds, which carries a name only once the
    /// reader types one.
    ///
    /// The name has no part in a compare. The rules come from the parser and
    /// the conditions, and the verdicts from the tests, so a blank name is
    /// the write's concern rather than this one's, and the write infers one.
    /// A blank parser is the write's concern too: the editor renders nothing
    /// to compare against until the reader picks one.
    ///
    /// # Errors
    ///
    /// Returns the same condition refusals [`Self::parse`] does, less the
    /// missing parser.
    pub(crate) fn parse_draft(body: &str) -> Result<Self, FormError> {
        let posted = read(body);

        // A row with no field name is one the reader added and has not filled
        // in.
        let conditions = posted
            .conditions
            .into_values()
            .filter(|row| !row.field.trim().is_empty())
            .map(condition)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            name: posted.name.trim().to_owned(),
            parser: posted.parser.trim().to_owned(),
            conditions,
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

    /// Writes the pairs a browser posts for this ruleset.
    ///
    /// The inverse of [`Self::parse`], which the editor uses to seed the
    /// draft its live re-render reads.
    pub(crate) fn encode(&self) -> String {
        let mut pairs = form_urlencoded::Serializer::new(String::new());

        pairs.append_pair("name", &self.name);
        pairs.append_pair("parser", &self.parser);

        for (index, condition) in self.conditions.iter().enumerate() {
            pairs.append_pair(&format!("condition.{index}.field"), &condition.field);
            pairs.append_pair(&format!("condition.{index}.op"), condition.op.label());
            pairs.append_pair(&format!("condition.{index}.value"), &condition.value);
        }

        for (index, test) in self.tests.iter().enumerate() {
            encode_test(&mut pairs, index, test);
        }

        pairs.finish()
    }
}

fn read_condition(conditions: &mut BTreeMap<usize, ConditionRow>, key: &str, value: &str) {
    let Some(rest) = key.strip_prefix("condition.") else {
        return;
    };

    let Some((index, attribute)) = rest.split_once('.') else {
        return;
    };

    let Ok(index) = index.parse::<usize>() else {
        return;
    };

    let condition = conditions.entry(index).or_default();

    match attribute {
        "field" => condition.field = value.to_owned(),
        "op" => condition.op = value.to_owned(),
        "value" => condition.value = value.to_owned(),
        _ => {}
    }
}

/// Sorts a form-encoded body into its parts, judging none of them.
fn read(body: &str) -> Posted {
    let mut posted = Posted::default();

    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "name" => posted.name = value.into_owned(),
            "parser" => posted.parser = value.into_owned(),
            _ => {
                read_test(&mut posted.tests, &key, &value);
                read_condition(&mut posted.conditions, &key, &value);
            }
        }
    }

    posted
}

/// Resolves one posted row into a condition.
///
/// An operator that compares no value keeps whatever the input beside it
/// held, because the editor renders that input under every operator and the
/// reader's text survives a change of mind about the operator.
fn condition(row: ConditionRow) -> Result<Condition, FormError> {
    let op = Op::from_label(&row.op).context(UnknownOpSnafu { op: &row.op })?;
    let field = row.field.trim().to_owned();

    ensure!(
        !op.takes_value() || !row.value.trim().is_empty(),
        MissingValueSnafu { field: &field }
    );

    Ok(Condition {
        field,
        op,
        value: row.value,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Condition, EditorRows, FormError, Op, RulesetForm};
    use crate::parser::TitleTest;

    #[test]
    fn a_ruleset_names_a_parser_and_nothing_else_reads_as_one() {
        assert_eq!(
            RulesetForm::parse("name=Hollow&parser=series"),
            Ok(RulesetForm {
                name: "Hollow".to_owned(),
                parser: "series".to_owned(),
                conditions: Vec::new(),
                tests: Vec::new(),
            }),
            "a ruleset with no condition claims every title its parser reads"
        );

        assert_eq!(
            RulesetForm::parse("name=Hollow"),
            Err(FormError::MissingParser),
            "a ruleset with no parser reads no title"
        );

        assert_eq!(
            RulesetForm::parse("parser=series"),
            Ok(RulesetForm {
                name: String::new(),
                parser: "series".to_owned(),
                conditions: Vec::new(),
                tests: Vec::new(),
            }),
            "a blank name is inferred at the write"
        );
    }

    #[test]
    fn tests_read_the_title_and_each_expected_value() {
        let form = RulesetForm::parse(
            "name=Hollow&parser=series\
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
    fn conditions_read_field_op_and_value() {
        assert_eq!(
            RulesetForm::parse(
                "name=Hollow&parser=series\
                 &condition.1.field=episodeNumber&condition.1.op=one+of&condition.1.value=10,+12\
                 &condition.0.field=resolution&condition.0.op=equals&condition.0.value=1080p\
                 &condition.2.field=&condition.2.op=equals&condition.2.value="
            )
            .map(|form| form.conditions),
            Ok(vec![
                Condition {
                    field: "resolution".to_owned(),
                    op: Op::Equals,
                    value: "1080p".to_owned(),
                },
                Condition {
                    field: "episodeNumber".to_owned(),
                    op: Op::OneOf,
                    value: "10, 12".to_owned(),
                },
            ]),
            "the index orders the conditions, and a row naming no field is one the reader added"
        );

        assert_eq!(
            RulesetForm::parse("name=X&parser=series&condition.0.field=show&condition.0.op=x"),
            Err(FormError::UnknownOp { op: "x".to_owned() }),
        );

        assert_eq!(
            RulesetForm::parse("name=X&parser=series&condition.0.field=show&condition.0.op=equals"),
            Err(FormError::MissingValue {
                field: "show".to_owned()
            }),
            "an equality with nothing to compare against asserts nothing"
        );
    }

    #[test]
    fn an_encoded_form_parses_back_to_itself() {
        let saved = RulesetForm {
            name: "The Hollow Meridian".to_owned(),
            parser: "series-episodes".to_owned(),
            conditions: vec![Condition {
                field: "season".to_owned(),
                op: Op::AtLeast,
                value: "2".to_owned(),
            }],
            tests: vec![TitleTest {
                title: "The.Hollow.Meridian.S04E06.1080p".to_owned(),
                expected: BTreeMap::from([
                    ("show".to_owned(), "the hollow meridian".to_owned()),
                    ("season".to_owned(), "4".to_owned()),
                ]),
            }],
        };

        assert_eq!(
            RulesetForm::parse(&saved.encode()),
            Ok(saved),
            "what the editor seeds its draft with is what a post reads back"
        );
    }

    #[test]
    fn editor_rows_keep_a_blank_row_and_need_no_name() {
        assert_eq!(
            EditorRows::parse("name=&parser=&condition.0.field=&test.0.title="),
            EditorRows {
                parser: None,
                conditions: vec![Condition {
                    field: String::new(),
                    op: Op::Equals,
                    value: String::new(),
                }],
                tests: vec![TitleTest {
                    title: String::new(),
                    expected: BTreeMap::new(),
                }],
            },
            "an added row lists under the first operator, and the page needs no name yet"
        );
    }

    #[test]
    fn a_draft_needs_no_name_and_no_parser() {
        assert_eq!(
            RulesetForm::parse_draft("condition.0.field=show&condition.0.op=present"),
            Ok(RulesetForm {
                name: String::new(),
                parser: String::new(),
                conditions: vec![Condition {
                    field: "show".to_owned(),
                    op: Op::Present,
                    value: String::new(),
                }],
                tests: Vec::new(),
            }),
            "the editor compares nothing until the reader picks a parser, so neither blocks a \
             draft"
        );
    }
}
