//! What a ruleset is made of.
//!
//! A ruleset is a set of fields, and each field claims one run of a release
//! filename. [`Segment`] tags such a run with the position of the field that
//! claimed it. [`FieldKind`] says how a matched string converts, and a
//! premade kind carries the pattern its fields match with.
//!
//! [`Candidate`] and [`Diff`] belong here too. The editor tests a ruleset
//! against filenames as the reader edits, and a diff state is what each row
//! reports about the edit in progress.
//!
//! Every value is `'static`, so a handler borrows a ruleset rather than
//! building one per request. That is what a later store has to preserve.

#[cfg(test)]
pub(crate) mod fixture;
pub(crate) mod form;
pub(crate) mod registry;
pub(crate) mod store;

use std::collections::BTreeMap;

/// The color one field wears wherever the reader meets it.
///
/// A field takes its color from its position among a ruleset's resolved
/// fields, so the same field reads the same in Matches, on the home page, and
/// in the editor. A color by position never collides inside one ruleset, which
/// a color hashed from the name does.
///
/// The twelve colors repeat past the twelfth field. A ruleset that long has
/// already lost the reader's eye, and a repeat reads better than running out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tint(usize);

impl Tint {
    pub(crate) fn at(position: usize) -> Self {
        Self(position % 12)
    }

    /// Tailwind classes that tint a matched run inside a filename.
    ///
    /// Each list is a whole literal because the Tailwind CLI scans source
    /// text for class names. A list joined at runtime never reaches the
    /// generated stylesheet.
    pub(crate) fn classes(self) -> &'static str {
        match self.0 {
            0 => "bg-sky-500/15 text-sky-300",
            1 => "bg-violet-500/15 text-violet-300",
            2 => "bg-amber-500/15 text-amber-300",
            3 => "bg-orange-500/15 text-orange-300",
            4 => "bg-teal-500/15 text-teal-300",
            5 => "bg-emerald-500/15 text-emerald-300",
            6 => "bg-cyan-500/15 text-cyan-300",
            7 => "bg-fuchsia-500/15 text-fuchsia-300",
            8 => "bg-rose-500/15 text-rose-300",
            9 => "bg-lime-500/15 text-lime-300",
            10 => "bg-indigo-500/15 text-indigo-300",
            _ => "bg-slate-500/20 text-slate-300",
        }
    }

    /// Tailwind background for the solid dot beside a field name.
    pub(crate) fn dot(self) -> &'static str {
        match self.0 {
            0 => "bg-sky-400",
            1 => "bg-violet-400",
            2 => "bg-amber-400",
            3 => "bg-orange-400",
            4 => "bg-teal-400",
            5 => "bg-emerald-400",
            6 => "bg-cyan-400",
            7 => "bg-fuchsia-400",
            8 => "bg-rose-400",
            9 => "bg-lime-400",
            10 => "bg-indigo-400",
            _ => "bg-slate-400",
        }
    }
}

/// A run of characters in a filename, tagged with the field that claimed it.
///
/// The field is its position among the ruleset's resolved fields, which is
/// also the anchor of that field's row in the editor.
///
/// [`None`] marks separators and anything no rule matched. The `text` values
/// in order reproduce the filename exactly, so the highlighted render never
/// drifts from the name it describes.
#[derive(Debug)]
pub(crate) struct Segment<'a> {
    pub(crate) text: &'a str,
    pub(crate) field: Option<usize>,
}

/// A set of field rules that together parse one family of filenames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ruleset {
    /// Stable key that names the ruleset in a URL.
    pub(crate) id: String,

    pub(crate) name: String,

    /// Whether the ruleset runs when the process starts.
    ///
    /// This seeds the switch state once. [`crate::server`] holds the live
    /// value from then on, which a reader flips at runtime without editing
    /// this declaration.
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

    /// What the reader expects this ruleset to read from named titles.
    ///
    /// The engine never reads these. They exist for the editor, which runs
    /// them against the draft as the reader types, so a rule change that
    /// breaks a title the reader cared about says so at once.
    pub(crate) tests: Vec<RulesetTest>,
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

/// One extraction rule, naming the run it claims and how its value is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) kind: FieldKind,

    /// The regex this field reads its value with, or `None` to take the
    /// pattern its kind supplies.
    ///
    /// A pattern declared on a premade kind wins over the built-in one. That
    /// is how a ruleset replaces a season with a single constant while
    /// keeping the kind's normalization.
    ///
    /// `None` on a kind that supplies none is a blank. Only a template
    /// declares one, naming the part and the flags and leaving the regex for
    /// the ruleset built on it to write.
    pub(crate) pattern: Option<String>,

    pub(crate) required: bool,

    /// Whether this field is part of the key that decides whether two
    /// releases are the same item.
    ///
    /// A different group, resolution, or encode of one episode is still that
    /// episode, so only the fields that name what a release *is* take part.
    pub(crate) identity: bool,
}

/// One title the reader named, and what each field must read from it.
///
/// The expected value is in the normalized form the field's kind produces,
/// which is what the identity stores, so a season field that captures `01`
/// from `S01E40` passes with the expected value `1`. A test on the raw
/// capture instead disagrees with the library the first time a zero moves.
///
/// A field absent from `expected` asserts nothing about that field. A reader
/// pins down the one value they care about without writing out the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RulesetTest {
    pub(crate) title: String,

    /// Keyed by field name, in that name's order.
    pub(crate) expected: BTreeMap<String, String>,
}

impl Field {
    /// Returns the regex that reads this field's value, or [`None`] when the
    /// field is a blank.
    ///
    /// A blank is what a template leaves for the ruleset based on it to fill.
    /// It reads no value of its own, so a ruleset that inherits one without
    /// replacing it never compiles.
    pub(crate) fn matcher(&self) -> Option<&str> {
        self.pattern.as_deref().or_else(|| self.kind.pattern())
    }
}

/// How a matched string converts before the rest of the app sees it.
///
/// [`Self::Season`] and [`Self::Episode`] are premade kinds: each carries its
/// own pattern, so a ruleset names the kind and writes no regex. Every other
/// kind leaves the pattern to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    Text,
    Number,
    Enum,
    Boolean,
    Season,
    Episode,
}

impl FieldKind {
    /// Every kind, in the order the editor's dropdown lists them.
    pub(crate) const ALL: &'static [Self] = &[
        Self::Text,
        Self::Number,
        Self::Enum,
        Self::Boolean,
        Self::Season,
        Self::Episode,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Enum => "enum",
            Self::Boolean => "boolean",
            Self::Season => "season",
            Self::Episode => "episode",
        }
    }

    /// The kind named by `label`, or [`None`] for anything else.
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.label() == label)
    }

    /// Returns the pattern the kind supplies, or `None` when the field has to
    /// declare one.
    ///
    /// The season pattern reads `S01`, `S1`, `S01E02`, `Season.1`, and
    /// `Season 1`. It refuses `S123` and a bare year, because a season number
    /// runs to two digits and a year carries no `S`. The episode pattern
    /// requires the season prefix, so a loose number elsewhere in a title
    /// never reads as an episode.
    ///
    /// Each pattern names its capture group after the field the shipped
    /// rulesets give it. A ruleset that names the field something else still
    /// reads it, because `captures` in [`crate::rules`] falls back to group 1.
    pub(crate) fn pattern(self) -> Option<&'static str> {
        match self {
            Self::Season => {
                Some(r"(?i)(?:^|[. _-])(?:S|Season[. _]?)(?<season>\d{1,2})(?:E\d|[. _-]|$)")
            }
            Self::Episode => Some(r"(?i)S\d{1,2}E(?<episode>\d{1,3})"),
            Self::Text | Self::Number | Self::Enum | Self::Boolean => None,
        }
    }

    /// Reduces a captured value to the form two releases have to agree on.
    ///
    /// A number kind drops leading zeros. Every other kind lowercases the
    /// value and collapses its separators to one space.
    ///
    /// A tracker writes `The.Hollow.Meridian` where a torrent client writes
    /// `The Hollow Meridian`, and the two capitalize differently. A season
    /// reads `01` on one side and `1` on the other. Collapsing both is what
    /// makes those one release rather than two.
    ///
    /// The identity and a saved test's verdict both read through this, so a
    /// test that passes describes the same value the library stores.
    pub(crate) fn normalize(self, raw: &str) -> String {
        if matches!(self, Self::Number | Self::Season | Self::Episode)
            && let Ok(number) = raw.trim_start_matches('0').parse::<u64>()
        {
            return number.to_string();
        }

        raw.to_lowercase()
            .split(['.', '_', '-', ' ', '\t'])
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::FieldKind;

    /// Reads `title` through the pattern `kind` supplies, as the engine does.
    fn read(kind: FieldKind, title: &str) -> Option<String> {
        let regex = Regex::new(kind.pattern().expect("a premade kind")).expect("a valid regex");

        regex
            .captures(title)
            .and_then(|caps| caps.name(kind.label()).or_else(|| caps.get(1)))
            .map(|value| value.as_str().to_owned())
    }

    #[test]
    fn season_kind_reads_each_form() {
        let titles = [
            "Show.S01.mkv",
            "Show.S1E1",
            "Show.S01E02",
            "Show.Season.1",
            "Show Season 1",
            "Show.S123.mkv",
            "Coastal.Drift.2024.1080p",
        ];

        assert_eq!(
            titles
                .iter()
                .map(|title| read(FieldKind::Season, title))
                .collect::<Vec<_>>(),
            [
                Some("01".to_owned()),
                Some("1".to_owned()),
                Some("01".to_owned()),
                Some("1".to_owned()),
                Some("1".to_owned()),
                None,
                None,
            ],
            "season read from each title"
        );
    }

    #[test]
    fn episode_kind_needs_a_season_prefix() {
        assert_eq!(
            read(FieldKind::Episode, "Show.S04E06"),
            Some("06".to_owned()),
            "episode behind a season"
        );

        assert_eq!(
            read(FieldKind::Episode, "[OpenReel] Coastal.Ecology - 18"),
            None,
            "loose number with no season"
        );
    }

    #[test]
    fn normalize_by_kind() {
        assert_eq!(
            [
                FieldKind::Season.normalize("01"),
                FieldKind::Text.normalize("The.Hollow.Meridian"),
                FieldKind::Number.normalize("x"),
            ],
            ["1", "the hollow meridian", "x"],
            "a number kind drops leading zeros, and anything else collapses"
        );
    }
}
