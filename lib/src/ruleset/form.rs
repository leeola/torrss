//! What the ruleset editor posts, and what it becomes.
//!
//! The editor's field list grows and shrinks in the browser, so the number of
//! rows a submission carries is not known ahead of time. A typed form struct
//! needs that number. This reads the rows out of the raw body instead.
//!
//! Rows are keyed `field.{i}.attribute`. The index orders them and nothing
//! more, so a row the reader removed leaves a gap rather than renumbering
//! every row after it.

use std::collections::{BTreeMap, BTreeSet};

use snafu::{OptionExt, ensure};
use url::form_urlencoded;

use super::{Condition, Op};
use crate::parser::form::{
    DuplicateNameSnafu, EmptyNameSnafu, FormError, MissingValueSnafu, Row, TestRow, UnknownOpSnafu,
    draft_field, encode_field, encode_test, field, read_row, read_test,
};
use crate::parser::{Field, TitleTest};

/// The role a posted ruleset names, which decides what its base means.
///
/// A ruleset is one of these three and never two, so one radio group posts
/// the whole choice and no combination of controls is illegal. An absent or
/// unknown `role` reads as [`STANDALONE_ROLE`], which is what a body written
/// outside the editor most likely means.
pub(crate) const TEMPLATE_ROLE: &str = "template";

/// See [`TEMPLATE_ROLE`].
pub(crate) const STANDALONE_ROLE: &str = "standalone";

/// See [`TEMPLATE_ROLE`].
pub(crate) const BASED_ROLE: &str = "based";

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
    /// True under [`TEMPLATE_ROLE`] and false under either other role.
    pub(crate) template: bool,

    /// The template this ruleset is built on, or [`None`] for a ruleset that
    /// declares every field itself.
    ///
    /// A base counts only under [`BASED_ROLE`], so a select the editor hides
    /// still posts its value and no other role reads it.
    pub(crate) based_on: Option<String>,

    pub(crate) fields: Vec<Field>,

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
    role: String,
    based_on: String,

    /// Ordered by index, so the fields come out in the order the editor
    /// showed them however the browser ordered the pairs.
    rows: BTreeMap<usize, Row>,

    conditions: BTreeMap<usize, ConditionRow>,

    tests: BTreeMap<usize, TestRow>,
}

/// The rows the editor holds, kept exactly as the form posted them.
///
/// A row the reader just added has a blank name and names no kind yet.
/// [`RulesetForm::parse`] drops it, because a stored ruleset carries no
/// nameless field. The row shards read this instead, so the row the reader
/// asked for appears.
///
/// A blank ruleset name is no error here either. The new page has none until
/// the reader types one, and the rows list before that.
///
/// A row that posted no kind comes back under the first option that select
/// renders, which is the option the browser showed. A condition that posted
/// no operator reads the same way.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EditorRows {
    pub(crate) based_on: Option<String>,
    pub(crate) fields: Vec<Field>,
    pub(crate) conditions: Vec<Condition>,
    pub(crate) tests: Vec<TitleTest>,
}

impl EditorRows {
    /// Reads every row a form-encoded body carries, refusing none of them.
    pub(crate) fn parse(body: &str) -> Self {
        let posted = read(body);

        Self {
            based_on: based_on(&posted),
            fields: posted.rows.into_values().map(draft_field).collect(),
            conditions: posted
                .conditions
                .into_values()
                .map(|row| Condition {
                    field: row.field.trim().to_owned(),
                    op: Op::from_label(&row.op).unwrap_or(Op::Equals),
                    value: row.value,
                })
                .collect(),
            tests: posted
                .tests
                .into_values()
                .map(|row| TitleTest {
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
    /// Returns the first row that names a type this build does not know, or
    /// that leaves a pattern empty where the type supplies none and the form
    /// is no template. A template keeps that blank for the ruleset built on
    /// it to fill.
    ///
    /// Returns a refusal when two rows name one field. The name keys the
    /// identity, the test columns, and the override lookup, so two rows named
    /// alike are one field carrying two patterns.
    ///
    /// Returns a refusal for a condition naming an operator this build does
    /// not know, or leaving its value empty under an operator that compares
    /// one.
    ///
    /// A field row with an empty name and a condition row with no field are
    /// both skipped rather than refused, because that is what an added row
    /// looks like before the reader fills it in. The editor lists such a row
    /// through [`EditorRows`], which keeps it.
    pub(crate) fn parse(body: &str) -> Result<Self, FormError> {
        let form = Self::parse_draft(body)?;
        ensure!(!form.name.is_empty(), EmptyNameSnafu);

        Ok(form)
    }

    /// Reads the draft the editor holds, which carries a name only once the
    /// reader types one.
    ///
    /// The name has no part in a compare. The rules come from `based_on` and
    /// the fields, and the verdicts from the tests, so a blank name is a
    /// save's concern rather than this one's.
    ///
    /// # Errors
    ///
    /// Returns the same refusals [`Self::parse`] does, less the empty name. A
    /// rule still does not compile from an unknown type or a missing pattern,
    /// and two rows under one name are still one field with two.
    ///
    /// Returns a refusal for a condition that names an operator this build
    /// does not know, or that leaves its value empty under an operator that
    /// compares one.
    pub(crate) fn parse_draft(body: &str) -> Result<Self, FormError> {
        let posted = read(body);

        let name = posted.name.trim().to_owned();
        let template = posted.role == TEMPLATE_ROLE;
        let based_on = based_on(&posted);

        let fields = posted
            .rows
            .into_values()
            .filter(|row| !row.name.trim().is_empty())
            .map(|row| field(row, template))
            .collect::<Result<Vec<_>, _>>()?;

        // Only the own rows are checked. A name that repeats a template's
        // field is an override, which `Ruleset::resolved_fields` matches by
        // that same name.
        let mut seen = BTreeSet::new();
        for field in &fields {
            ensure!(
                seen.insert(field.name.as_str()),
                DuplicateNameSnafu { field: &field.name }
            );
        }

        // A row with no field name is one the reader added and has not filled
        // in, as a nameless field row is.
        let conditions = posted
            .conditions
            .into_values()
            .filter(|row| !row.field.trim().is_empty())
            .map(condition)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            name,
            template,
            based_on,
            fields,
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
    /// draft its live re-render reads. A base is written only under the based
    /// role and a pattern only when the kind supplies none, because that is
    /// what a form actually sends and the draft has to start where the
    /// browser takes over.
    pub(crate) fn encode(&self) -> String {
        let mut pairs = form_urlencoded::Serializer::new(String::new());

        pairs.append_pair("name", &self.name);

        if self.template {
            pairs.append_pair("role", TEMPLATE_ROLE);
        } else if self.based_on.is_some() {
            pairs.append_pair("role", BASED_ROLE);
        } else {
            pairs.append_pair("role", STANDALONE_ROLE);
        }

        if let Some(based_on) = &self.based_on {
            pairs.append_pair("based_on", based_on);
        }

        for (index, field) in self.fields.iter().enumerate() {
            encode_field(&mut pairs, index, field);
        }

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

/// Files one `condition.{index}.attribute` pair under its condition.
///
/// A key that is not a condition row passes through untouched, as
/// [`read_row`] does with its own.
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

/// The template a posted body names, or [`None`] under any other role.
///
/// The editor hides the select rather than disabling it, so a ruleset that
/// once named a base still posts it after the reader picks another role. The
/// role decides whether that value counts, which is what keeps a stale one
/// out of a shard and out of a save.
fn based_on(posted: &Posted) -> Option<String> {
    if posted.role != BASED_ROLE {
        return None;
    }

    Some(posted.based_on.trim().to_owned()).filter(|id| !id.is_empty())
}

/// Sorts a form-encoded body into its parts, judging none of them.
fn read(body: &str) -> Posted {
    let mut posted = Posted::default();

    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "name" => posted.name = value.into_owned(),
            "role" => posted.role = value.into_owned(),
            "based_on" => posted.based_on = value.into_owned(),
            _ => {
                read_test(&mut posted.tests, &key, &value);
                read_condition(&mut posted.conditions, &key, &value);
                read_row(&mut posted.rows, &key, &value);
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
    use crate::parser::{Field, FieldKind, TitleTest};

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
        let form = RulesetForm::parse(
            "name=Series&role=standalone\
             &field.2.name=season&field.2.kind=text&field.2.pattern=S%5Cd%2B\
             &field.0.name=show&field.0.kind=text&field.0.pattern=%5E.%2B\
             &field.0.required=on&field.0.identity=on&field.0.tight=on",
        )
        .expect("the body parses");

        assert_eq!(
            form,
            RulesetForm {
                name: "Series".to_owned(),
                template: false,
                based_on: None,
                fields: vec![
                    Field {
                        tight: true,
                        ..text("show", "^.+", true, true)
                    },
                    text("season", r"S\d+", false, false),
                ],
                conditions: Vec::new(),
                tests: Vec::new(),
            },
            "the index orders the rows, and an absent checkbox reads false"
        );
    }

    #[test]
    fn an_inherited_ruleset_keeps_the_parent_it_names() {
        let form =
            RulesetForm::parse("name=Ashfall&role=based&based_on=series").expect("the body parses");

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
            RulesetForm::parse("name=Series&field.0.name=show&field.0.kind=colour"),
            Err(FormError::UnknownKind {
                kind: "colour".to_owned()
            })
        );
        assert_eq!(
            RulesetForm::parse("name=Series&field.0.name=show&field.0.kind=text"),
            Err(FormError::MissingPattern {
                field: "show".to_owned()
            }),
            "a text field carries its own pattern or reads nothing"
        );
    }

    #[test]
    fn a_template_keeps_a_blank_pattern() {
        const ROW: &str = "field.0.name=show&field.0.kind=text&field.0.pattern=";

        let parsed = RulesetForm::parse(&format!("name=Series&role=template&{ROW}"))
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
    fn an_encoded_form_parses_back_to_itself() {
        let based = RulesetForm {
            name: "Series Episodes".to_owned(),
            template: false,
            based_on: Some("series".to_owned()),
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

        let template = RulesetForm {
            name: "Series".to_owned(),
            template: true,
            based_on: None,
            fields: vec![text("show", "^.+", true, true)],
            conditions: Vec::new(),
            tests: Vec::new(),
        };

        assert_eq!(
            RulesetForm::parse(&based.encode()),
            Ok(based),
            "what the editor seeds its draft with is what a post reads back"
        );
        assert_eq!(
            RulesetForm::parse(&template.encode()),
            Ok(template),
            "and a template round trips under its own role"
        );
    }

    #[test]
    fn a_base_counts_only_under_the_based_role() {
        assert_eq!(
            RulesetForm::parse("name=X&role=template&based_on=series"),
            Ok(RulesetForm {
                name: "X".to_owned(),
                template: true,
                based_on: None,
                fields: Vec::new(),
                conditions: Vec::new(),
                tests: Vec::new(),
            }),
            "a select the editor hides still posts, and a template is based on nothing"
        );

        assert_eq!(
            RulesetForm::parse("name=X&role=standalone&based_on=series"),
            Ok(RulesetForm {
                name: "X".to_owned(),
                template: false,
                based_on: None,
                fields: Vec::new(),
                conditions: Vec::new(),
                tests: Vec::new(),
            }),
            "a ruleset that stands alone declares every field itself"
        );
    }

    #[test]
    fn editor_rows_keep_a_blank_row_and_need_no_name() {
        assert_eq!(
            EditorRows::parse(
                "name=&field.0.name=show&field.0.kind=text\
                 &field.0.pattern=%5E.%2B&field.1.name=&test.0.title="
            ),
            EditorRows {
                based_on: None,
                fields: vec![
                    text("show", "^.+", false, false),
                    Field {
                        name: String::new(),
                        kind: FieldKind::Text,
                        pattern: None,
                        required: false,
                        tight: false,
                        identity: false,
                    },
                ],
                conditions: Vec::new(),
                tests: vec![TitleTest {
                    title: String::new(),
                    expected: BTreeMap::new(),
                }],
            },
            "an added row lists under the first part and kind, and the page needs no name yet"
        );
    }

    #[test]
    fn a_draft_needs_no_name() {
        assert_eq!(
            RulesetForm::parse_draft("field.0.name=show&field.0.kind=text&field.0.pattern=.%2A"),
            Ok(RulesetForm {
                name: String::new(),
                template: false,
                based_on: None,
                fields: vec![text("show", ".*", false, false)],
                conditions: Vec::new(),
                tests: Vec::new(),
            }),
            "the rules come from the fields, so a compare runs before the reader names anything"
        );

        assert_eq!(
            RulesetForm::parse_draft("field.0.name=show&field.0.kind=colour"),
            Err(FormError::UnknownKind {
                kind: "colour".to_owned()
            }),
            "a rule still does not compile from a type this build does not know"
        );
    }

    #[test]
    fn conditions_read_field_op_and_value() {
        assert_eq!(
            RulesetForm::parse(
                "name=Series&role=standalone\
                 &condition.1.field=episodeNumber&condition.1.op=at+least&condition.1.value=10\
                 &condition.0.field=resolution&condition.0.op=equals&condition.0.value=1080p\
                 &condition.2.field=&condition.2.op=equals&condition.2.value="
            ),
            Ok(RulesetForm {
                name: "Series".to_owned(),
                template: false,
                based_on: None,
                fields: Vec::new(),
                conditions: vec![
                    Condition {
                        field: "resolution".to_owned(),
                        op: Op::Equals,
                        value: "1080p".to_owned(),
                    },
                    Condition {
                        field: "episodeNumber".to_owned(),
                        op: Op::AtLeast,
                        value: "10".to_owned(),
                    },
                ],
                tests: Vec::new(),
            }),
            "the index orders the conditions, and a row naming no field is one the reader added"
        );

        assert_eq!(
            RulesetForm::parse("name=X&condition.0.field=show&condition.0.op=rhymes+with"),
            Err(FormError::UnknownOp {
                op: "rhymes with".to_owned()
            }),
        );

        assert_eq!(
            RulesetForm::parse("name=X&condition.0.field=show&condition.0.op=equals"),
            Err(FormError::MissingValue {
                field: "show".to_owned()
            }),
            "an equality with nothing to compare against asserts nothing"
        );

        assert_eq!(
            RulesetForm::parse("name=X&condition.0.field=show&condition.0.op=present")
                .map(|form| form.conditions),
            Ok(vec![Condition {
                field: "show".to_owned(),
                op: Op::Present,
                value: String::new(),
            }]),
            "an operator that compares no value needs none"
        );
    }
}
