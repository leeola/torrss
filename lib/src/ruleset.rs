//! What a ruleset is made of.
//!
//! A ruleset decides which of the titles a set of fields reads it wants. The
//! fields themselves belong to [`crate::parser`], which reads a filename
//! apart and judges nothing. A [`Condition`] is where the judgment lives:
//! one comparison on a value the fields already read.
//!
//! [`Diff`] belongs here too. The editor runs a ruleset against filenames as
//! the reader edits, and a diff state is what each row reports about the
//! edit in progress.
//!
//! Every value is `'static`, so a handler borrows a ruleset rather than
//! building one per request. That is what a later store has to preserve.

#[cfg(test)]
pub(crate) mod fixture;
pub(crate) mod form;
pub(crate) mod registry;
pub(crate) mod store;

use crate::parser::{Field, FieldKind, TitleTest};

/// A set of field rules that together parse one family of filenames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ruleset {
    /// Stable key that names the ruleset in a URL.
    pub(crate) id: String,

    pub(crate) name: String,

    /// Whether the ruleset claims titles.
    ///
    /// A new ruleset stores `true`, and the editor's switch flips the stored
    /// value through `Rulesets::set_enabled`.
    ///
    /// A template stores `false`. It claims nothing, and the editor offers it
    /// no switch.
    ///
    /// A disabled ruleset filters nothing, so its releases stay out of the
    /// feed.
    pub(crate) enabled: bool,

    /// Whether this ruleset only serves as a foundation for others.
    ///
    /// A template claims no title and filters no feed. It exists so several
    /// rulesets share one set of fields and replace only what differs.
    pub(crate) template: bool,

    /// [`Ruleset::id`] of the template this ruleset is built on, or [`None`]
    /// for a ruleset that declares every field itself.
    pub(crate) based_on: Option<String>,

    /// The fields this ruleset declares itself.
    ///
    /// A ruleset on a template holds only the fields it replaces, so an edit
    /// to the template reaches every ruleset that did not replace that
    /// field. Use [`Ruleset::resolved_fields`] to get the full list the
    /// editor shows.
    pub(crate) fields: Vec<Field>,

    /// Each comparison this ruleset makes on a value its regex read.
    ///
    /// The regex decides which titles have the shape this ruleset describes,
    /// and these decide which of those it wants. A ruleset with none claims
    /// every title its regex reads.
    pub(crate) conditions: Vec<Condition>,

    /// What the reader expects this ruleset to read from named titles.
    ///
    /// The engine never reads these. They exist for the editor, which runs
    /// them against the draft as the reader types, so a rule change that
    /// breaks a title the reader cared about says so at once.
    pub(crate) tests: Vec<TitleTest>,
}

impl Ruleset {
    /// Every field the editor shows, each tagged with where it came from.
    ///
    /// A ruleset with no template returns its own fields untouched. One
    /// built on a template returns the template's fields in the template's
    /// order, so the two editors read the same way down the page.
    ///
    /// `template` is the ruleset this one is built on, which the caller
    /// resolves through [`crate::rules::Engine`]. Passing it in is what
    /// keeps a ruleset from reaching for a global list to find its own.
    pub(crate) fn resolved_fields<'a>(
        &'a self,
        template: Option<&'a Ruleset>,
    ) -> Vec<ResolvedField<'a>> {
        let Some(template) = template else {
            return self
                .fields
                .iter()
                .map(|field| ResolvedField {
                    field,
                    source: FieldSource::Own,
                })
                .collect();
        };

        template
            .fields
            .iter()
            .map(
                |inherited| match self.fields.iter().find(|own| own.name == inherited.name) {
                    Some(field) => ResolvedField {
                        field,
                        source: FieldSource::Overridden {
                            template: inherited,
                        },
                    },
                    None => ResolvedField {
                        field: inherited,
                        source: FieldSource::Inherited,
                    },
                },
            )
            .collect()
    }
}

/// A field as the editor shows it, once the template is applied.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedField<'a> {
    /// The value in effect, whether inherited or replaced.
    pub(crate) field: &'a Field,

    pub(crate) source: FieldSource<'a>,
}

impl ResolvedField<'_> {
    /// Whether the editor greys the row and locks its inputs.
    pub(crate) fn is_inherited(&self) -> bool {
        matches!(self.source, FieldSource::Inherited)
    }
}

/// Where a resolved field came from.
#[derive(Clone, Copy)]
pub(crate) enum FieldSource<'a> {
    /// Declared by a ruleset with no template.
    Own,

    /// Carried from the template.
    Inherited,

    /// Replaces the template's field.
    Overridden {
        /// The template's field, shown beside the override it replaced.
        template: &'a Field,
    },
}

/// How a title's match changed under the edit in progress.
///
/// A [`Diff::Removed`] title keeps the highlighting it had before the
/// edit. The row shows what the edit gives up. An unhighlighted filename
/// loses that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Diff {
    /// The edit made this filename match. It did not before.
    Added,
    /// The edit stopped this filename from matching. It did before.
    Removed,
    /// Matched before the edit and still matches.
    Kept,
    /// Did not match before the edit and still does not.
    Excluded,
}

impl Diff {
    /// Every state, in the order the filter bar lists them.
    pub(crate) const ALL: &'static [Self] =
        &[Self::Added, Self::Removed, Self::Kept, Self::Excluded];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Added => "new",
            Self::Removed => "removed",
            Self::Kept => "unchanged",
            Self::Excluded => "no match",
        }
    }

    /// URL-safe name, used as the value of the filter query parameter.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Added => "new",
            Self::Removed => "removed",
            Self::Kept => "unchanged",
            Self::Excluded => "unmatched",
        }
    }

    /// The state named by `slug`, or [`None`] for anything else.
    pub(crate) fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|diff| diff.slug() == slug)
    }

    /// Tailwind classes tinting the whole candidate row.
    pub(crate) fn row_classes(self) -> &'static str {
        match self {
            Self::Added => "border-emerald-500/40 bg-emerald-500/5",
            Self::Removed => "border-rose-500/40 bg-rose-500/5",
            Self::Kept => "border-slate-800 bg-slate-900/40",
            Self::Excluded => "border-slate-800/60 bg-transparent opacity-60",
        }
    }

    /// Tailwind classes for the small state badge on a candidate row.
    pub(crate) fn badge_classes(self) -> &'static str {
        match self {
            Self::Added => "bg-emerald-500/15 text-emerald-300",
            Self::Removed => "bg-rose-500/15 text-rose-300",
            Self::Kept => "bg-slate-700/40 text-slate-300",
            Self::Excluded => "bg-slate-800/60 text-slate-500",
        }
    }
}

/// How a condition compares the value its field read.
///
/// The four orderings compare numbers, so a ruleset writes one only on a
/// number, season, or episode field. [`Self::Present`] and [`Self::Absent`]
/// ask whether the field read anything at all and carry no value of their
/// own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Equals,
    NotEquals,
    LessThan,
    AtMost,
    GreaterThan,
    AtLeast,
    Present,
    Absent,
}

impl Op {
    /// Every operator, in the order the editor's dropdown lists them.
    pub(crate) const ALL: &'static [Self] = &[
        Self::Equals,
        Self::NotEquals,
        Self::LessThan,
        Self::AtMost,
        Self::GreaterThan,
        Self::AtLeast,
        Self::Present,
        Self::Absent,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::NotEquals => "not equals",
            Self::LessThan => "less than",
            Self::AtMost => "at most",
            Self::GreaterThan => "greater than",
            Self::AtLeast => "at least",
            Self::Present => "present",
            Self::Absent => "absent",
        }
    }

    /// The operator named by `label`, or [`None`] for anything else.
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|op| op.label() == label)
    }

    /// Whether this operator ranks two numbers rather than comparing text.
    pub(crate) fn orders(self) -> bool {
        matches!(
            self,
            Self::LessThan | Self::AtMost | Self::GreaterThan | Self::AtLeast
        )
    }

    /// Whether this operator compares against a value the reader writes.
    ///
    /// [`Self::Present`] and [`Self::Absent`] ask about the value's existence
    /// alone, so the editor's input beside one asserts nothing.
    pub(crate) fn takes_value(self) -> bool {
        !matches!(self, Self::Present | Self::Absent)
    }
}

/// One comparison a ruleset makes on a value its regex read.
///
/// The field names one of the ruleset's resolved fields. A condition on any
/// other name never compiles, because the value it asks about is one no rule
/// produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Condition {
    pub(crate) field: String,
    pub(crate) op: Op,

    /// What the field's value is compared against, in the form the reader
    /// typed it.
    ///
    /// It normalizes through the field's kind before an equality compares it,
    /// so a show typed as `The Hollow Meridian` meets `The.Hollow.Meridian`.
    /// An operator that takes no value ignores this.
    pub(crate) value: String,
}

impl Condition {
    /// Reports whether the value `read` meets this condition.
    ///
    /// `read` is the normalized value the field produced, or [`None`] when
    /// the field read nothing. Every operator but [`Op::Absent`] fails on
    /// [`None`], so a ruleset that names a value its title does not carry
    /// claims nothing.
    ///
    /// An ordering parses both sides as numbers and fails when either side is
    /// not one. A field whose kind ranks may still read text the pattern let
    /// through, and text has no place on a number line.
    pub(crate) fn holds(&self, kind: FieldKind, read: Option<&str>) -> bool {
        let Some(read) = read else {
            return self.op == Op::Absent;
        };

        match self.op {
            Op::Absent => false,
            Op::Present => true,
            Op::Equals => read == kind.normalize(&self.value),
            Op::NotEquals => read != kind.normalize(&self.value),
            Op::LessThan | Op::AtMost | Op::GreaterThan | Op::AtLeast => {
                let (Ok(read), Ok(against)) =
                    (read.parse::<u64>(), self.value.trim().parse::<u64>())
                else {
                    return false;
                };

                match self.op {
                    Op::LessThan => read < against,
                    Op::AtMost => read <= against,
                    Op::GreaterThan => read > against,
                    _ => read >= against,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Condition, Op};
    use crate::parser::FieldKind;

    fn condition(field: &str, op: Op, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op,
            value: value.to_owned(),
        }
    }

    #[test]
    fn equals_compares_normalized_text() {
        let typed = condition("show", Op::Equals, "The Hollow Meridian");

        assert!(
            typed.holds(FieldKind::Text, Some("the hollow meridian")),
            "the reader's spacing normalizes to what the field read"
        );
        assert!(
            !typed.holds(FieldKind::Text, Some("ashfall county")),
            "and another show is another show"
        );
    }

    #[test]
    fn an_ordering_compares_numbers() {
        let tenth = condition("episodeNumber", Op::AtLeast, "10");

        assert!(tenth.holds(FieldKind::Episode, Some("12")));
        assert!(!tenth.holds(FieldKind::Episode, Some("9")));
        assert!(
            !tenth.holds(FieldKind::Episode, Some("abc")),
            "a value off the number line meets no ordering"
        );
    }

    #[test]
    fn absent_holds_on_no_value_and_the_rest_fail() {
        assert!(
            condition("episodeNumber", Op::Absent, "").holds(FieldKind::Episode, None),
            "a pack names no episode, which is what a pack-only ruleset asks for"
        );
        assert!(!condition("episodeNumber", Op::Present, "").holds(FieldKind::Episode, None));
        assert!(!condition("episodeNumber", Op::Equals, "6").holds(FieldKind::Episode, None));
    }
}
