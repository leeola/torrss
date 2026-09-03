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

use super::{Field, FieldKind, Part};

/// A ruleset as the editor's form describes it.
///
/// The id is absent because the form never carries one. A create derives it
/// from the name through [`unique_slug`], and a save already knows it from
/// the route.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RulesetForm {
    pub(crate) name: String,

    /// The ruleset this one narrows, or [`None`] for a base.
    ///
    /// An empty value means none. A select posts its empty option rather than
    /// omitting the key.
    pub(crate) inherits: Option<String>,

    pub(crate) fields: Vec<Field>,
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

impl RulesetForm {
    /// Reads a ruleset out of a form-encoded body.
    ///
    /// # Errors
    ///
    /// Returns the first row that names a part or a type this build does not
    /// know, or that leaves a pattern empty where the type supplies none. A
    /// row with an empty name is skipped rather than refused, because that is
    /// what an added row looks like before the reader fills it in.
    pub(crate) fn parse(body: &str) -> Result<Self, FormError> {
        let mut name = String::new();
        let mut inherits = String::new();

        // Ordered by index, so the fields come out in the order the editor
        // showed them however the browser ordered the pairs.
        let mut rows: BTreeMap<usize, Row> = BTreeMap::new();

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match key.as_ref() {
                "name" => name = value.into_owned(),
                "inherits" => inherits = value.into_owned(),
                _ => read_row(&mut rows, &key, &value),
            }
        }

        let name = name.trim().to_owned();
        ensure!(!name.is_empty(), EmptyNameSnafu);

        Ok(Self {
            name,
            inherits: Some(inherits.trim().to_owned()).filter(|id| !id.is_empty()),
            fields: rows
                .into_values()
                .filter(|row| !row.name.trim().is_empty())
                .map(field)
                .collect::<Result<Vec<_>, _>>()?,
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
        pairs.append_pair("inherits", self.inherits.as_deref().unwrap_or_default());

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

/// Resolves one posted row into a field.
///
/// The pattern is dropped when the kind supplies one, so a premade kind keeps
/// its built-in regex rather than storing a copy the editor rendered.
fn field(row: Row) -> Result<Field, FormError> {
    let kind = FieldKind::from_label(&row.kind).context(UnknownKindSnafu { kind: &row.kind })?;
    let part = Part::from_slug(&row.part).context(UnknownPartSnafu { part: &row.part })?;

    let pattern = match kind.pattern() {
        Some(_) => None,
        None => {
            let pattern = row.pattern.unwrap_or_default();

            ensure!(
                !pattern.trim().is_empty(),
                MissingPatternSnafu { field: &row.name }
            );

            Some(pattern)
        }
    };

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
    use super::{FormError, RulesetForm, slug, unique_slug};
    use crate::ruleset::{Field, FieldKind, Part};

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
            "name=Series&inherits=\
             &field.2.name=season&field.2.part=season&field.2.kind=text&field.2.pattern=S%5Cd%2B\
             &field.0.name=show&field.0.part=show&field.0.kind=text&field.0.pattern=%5E.%2B\
             &field.0.required=on&field.0.identity=on",
        )
        .expect("the body parses");

        assert_eq!(
            form,
            RulesetForm {
                name: "Series".to_owned(),
                inherits: None,
                fields: vec![
                    text("show", Part::Show, "^.+", true, true),
                    text("season", Part::Season, r"S\d+", false, false),
                ],
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
        let form = RulesetForm::parse("name=Ashfall&inherits=series").expect("the body parses");

        assert_eq!(form.inherits, Some("series".to_owned()));
        assert_eq!(
            form.fields,
            Vec::new(),
            "a child declares no field of its own"
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
            inherits: Some("series".to_owned()),
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
        };

        assert_eq!(
            RulesetForm::parse(&form.encode()),
            Ok(form),
            "what the editor seeds its draft with is what a post reads back"
        );
    }
}
