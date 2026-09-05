//! Which of the titles a parser reads are wanted.
//!
//! A [`crate::parser::Parser`] reads a filename apart and judges nothing. A
//! ruleset names one and decides which of the names it reads it claims,
//! through [`Condition`]: one comparison on a value the parser already read.
//!
//! The identity names the parser rather than the ruleset, so two rulesets on
//! one parser share one namespace of releases.
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

use crate::parser::{FieldKind, TitleTest};

/// One set of conditions over the values a parser reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ruleset {
    /// Stable key that names the ruleset in a URL.
    pub(crate) id: String,

    /// What the reader calls this ruleset.
    ///
    /// The editor posts this blank when the reader types nothing, and a
    /// blank one stores [`inferred_name`] of the conditions instead. A typed
    /// name is kept as typed.
    pub(crate) name: String,

    /// Whether the ruleset claims titles.
    ///
    /// A new ruleset stores `true`, and the editor's switch flips the stored
    /// value through `Rulesets::set_enabled`.
    ///
    /// A disabled ruleset filters nothing, so its releases stay out of the
    /// feed.
    pub(crate) enabled: bool,

    /// [`crate::parser::Parser::id`] of the parser this ruleset reads titles
    /// with.
    ///
    /// The identity names the parser too, so every ruleset reading through
    /// one shares a single namespace of releases. Two rulesets that claim
    /// different halves of what a parser reads therefore never file the same
    /// episode twice.
    pub(crate) parser: String,

    /// Each comparison this ruleset makes on a value the parser read.
    ///
    /// The parser decides which titles have the shape this ruleset works on,
    /// and these decide which of those it wants. A ruleset with none claims
    /// every title its parser reads.
    pub(crate) conditions: Vec<Condition>,

    /// What the reader expects this ruleset to read from named titles.
    ///
    /// The engine never reads these. They exist for the editor, which runs
    /// them against the draft as the reader types, so a rule change that
    /// breaks a title the reader cared about says so at once.
    pub(crate) tests: Vec<TitleTest>,
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
///
/// [`Self::OneOf`] and [`Self::NoneOf`] read their value as a list, so one
/// condition names every resolution a reader wants rather than one ruleset
/// per resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Equals,
    NotEquals,
    OneOf,
    NoneOf,
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
        Self::OneOf,
        Self::NoneOf,
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
            Self::OneOf => "one of",
            Self::NoneOf => "none of",
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

    /// Whether this operator reads its value as a comma-separated list.
    ///
    /// Every other operator compares the value whole, so a comma inside one
    /// is a character like any other.
    pub(crate) fn lists(self) -> bool {
        matches!(self, Self::OneOf | Self::NoneOf)
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
    ///
    /// An operator that [`Op::lists`] reads this as a comma-separated list
    /// instead, and normalizes each entry on its own.
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
            Op::OneOf => self.choices().any(|choice| read == kind.normalize(choice)),
            Op::NoneOf => !self.choices().any(|choice| read == kind.normalize(choice)),
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

    /// Returns each entry of the value, read as a comma-separated list.
    ///
    /// A blank entry is dropped, so a trailing comma names nothing. Each
    /// entry comes out as the reader typed it, because the kind normalizes
    /// it only once it is compared.
    fn choices(&self) -> impl Iterator<Item = &str> {
        self.value
            .split(',')
            .map(str::trim)
            .filter(|choice| !choice.is_empty())
    }
}

/// Returns the name a condition list reads as.
///
/// An equals value is the name a reader types by hand, so it stands alone
/// and leads the list. Every other condition reads as its field, its
/// operator, and its value, which is how the editor's own row reads it. An
/// operator that takes no value renders without one.
///
/// A list value keeps its commas, because the reader wrote them.
///
/// A ruleset with no condition claims every title its parser reads, so it
/// takes the parser's name.
pub(crate) fn inferred_name(conditions: &[Condition], parser: &str) -> String {
    if conditions.is_empty() {
        return parser.to_owned();
    }

    let leading = conditions.iter().filter(|held| held.op == Op::Equals);
    let trailing = conditions.iter().filter(|held| held.op != Op::Equals);

    let mut name = String::new();

    for condition in leading.chain(trailing) {
        if !name.is_empty() {
            name.push_str(", ");
        }

        if condition.op == Op::Equals {
            name.push_str(condition.value.trim());
            continue;
        }

        name.push_str(&condition.field);
        name.push(' ');
        name.push_str(condition.op.label());

        if condition.op.takes_value() {
            name.push(' ');
            name.push_str(condition.value.trim());
        }
    }

    name
}

#[cfg(test)]
mod tests {
    use super::{Condition, Op, inferred_name};
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
    fn one_of_meets_any_listed_value() {
        let either = condition("resolution", Op::OneOf, "720p, 1080p");

        assert!(either.holds(FieldKind::Text, Some("720p")));
        assert!(either.holds(FieldKind::Text, Some("1080p")));
        assert!(
            !either.holds(FieldKind::Text, Some("2160p")),
            "a resolution the list does not name meets nothing"
        );
        assert!(
            condition("season", Op::OneOf, "1, 03").holds(FieldKind::Season, Some("3")),
            "each entry normalizes through the field's kind before it compares"
        );
    }

    #[test]
    fn none_of_refuses_every_listed_value() {
        let neither = condition("show", Op::NoneOf, "The Hollow Meridian, Ashfall County");

        assert!(
            !neither.holds(FieldKind::Text, Some("ashfall county")),
            "the second entry names this show, so the condition refuses it"
        );
        assert!(neither.holds(FieldKind::Text, Some("other")));
        assert!(
            !neither.holds(FieldKind::Text, None),
            "every operator but absent fails on a field that read nothing"
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

    #[test]
    fn equals_values_lead_and_the_rest_follow() {
        assert_eq!(
            inferred_name(
                &[
                    condition("show", Op::Equals, "Coastal Ecology"),
                    condition("season", Op::AtLeast, "2"),
                    condition("resolution", Op::OneOf, "1080p, 2160p"),
                ],
                "Series Episodes"
            ),
            "Coastal Ecology, season at least 2, resolution one of 1080p, 2160p"
        );
    }

    #[test]
    fn an_equals_after_an_ordering_still_leads() {
        assert_eq!(
            inferred_name(
                &[
                    condition("season", Op::AtLeast, "2"),
                    condition("show", Op::Equals, "Coastal Ecology"),
                ],
                "Series Episodes"
            ),
            "Coastal Ecology, season at least 2"
        );
    }

    #[test]
    fn no_condition_takes_the_parser_name() {
        assert_eq!(inferred_name(&[], "Series Episodes"), "Series Episodes");
        assert_eq!(
            inferred_name(
                &[condition("episodeNumber", Op::Absent, "")],
                "Series Episodes"
            ),
            "episodeNumber absent",
            "an operator that takes no value renders without one"
        );
    }
}
