//! What a ruleset is made of.
//!
//! A ruleset is a set of fields, and each field claims one part of a release
//! filename. [`Part`] names the vocabulary of parts, and [`Segment`] tags the
//! run of characters a part claimed. [`FieldKind`] says how a matched string
//! converts, and a premade kind carries the pattern its fields match with.
//!
//! [`Candidate`] and [`Diff`] belong here too. The editor tests a ruleset
//! against filenames as the reader edits, and a diff state is what each row
//! reports about the edit in progress.
//!
//! Every value is `'static`, so a handler borrows a ruleset rather than
//! building one per request. That is what a later store has to preserve.

#[cfg(test)]
pub(crate) mod fixture;
pub(crate) mod registry;
pub(crate) mod store;

/// A named component that a ruleset pulls out of a release filename.
///
/// The variant picks both the label and the highlight color, so a reader sees
/// at a glance which part of a filename a rule claimed.
///
/// Only a ruleset declaration names a part, and the application declares
/// none, so nothing constructs one outside the fixture and the tests. The
/// vocabulary stays whole because a stored ruleset has to name the same
/// parts the editor already renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the parts a stored ruleset will name, ahead of a store to name them"
)]
pub(crate) enum Part {
    Show,
    Movie,
    Season,
    Episode,
    Year,
    Resolution,
    Source,
    Codec,
    Audio,
    Publisher,
    Checksum,
    Extension,
}

impl Part {
    /// Every part, in the order the field editor lists them.
    pub(crate) const ALL: &'static [Self] = &[
        Self::Show,
        Self::Movie,
        Self::Season,
        Self::Episode,
        Self::Year,
        Self::Resolution,
        Self::Source,
        Self::Codec,
        Self::Audio,
        Self::Publisher,
        Self::Checksum,
        Self::Extension,
    ];

    /// URL-safe name, used as the anchor that links a matched run in a
    /// filename to the field rule that claimed it.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Movie => "movie",
            Self::Season => "season",
            Self::Episode => "episode",
            Self::Year => "year",
            Self::Resolution => "resolution",
            Self::Source => "source",
            Self::Codec => "codec",
            Self::Audio => "audio",
            Self::Publisher => "publisher",
            Self::Checksum => "checksum",
            Self::Extension => "extension",
        }
    }

    /// The part named by `slug`, or [`None`] for anything else.
    pub(crate) fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|part| part.slug() == slug)
    }

    #[allow(
        dead_code,
        reason = "read by the highlighted-filename render, ahead of a page that renders one"
    )]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Movie => "movie",
            Self::Season => "season",
            Self::Episode => "episode",
            Self::Year => "year",
            Self::Resolution => "resolution",
            Self::Source => "source",
            Self::Codec => "codec",
            Self::Audio => "audio",
            Self::Publisher => "publisher",
            Self::Checksum => "checksum",
            Self::Extension => "extension",
        }
    }

    /// Tailwind classes that tint a matched run inside a filename.
    ///
    /// Each list is a whole literal because the Tailwind CLI scans source
    /// text for class names. A list joined at runtime never reaches the
    /// generated stylesheet.
    pub(crate) fn classes(self) -> &'static str {
        match self {
            Self::Show => "bg-sky-500/15 text-sky-300",
            Self::Movie => "bg-violet-500/15 text-violet-300",
            Self::Season => "bg-amber-500/15 text-amber-300",
            Self::Episode => "bg-orange-500/15 text-orange-300",
            Self::Year => "bg-teal-500/15 text-teal-300",
            Self::Resolution => "bg-emerald-500/15 text-emerald-300",
            Self::Source => "bg-cyan-500/15 text-cyan-300",
            Self::Codec => "bg-fuchsia-500/15 text-fuchsia-300",
            Self::Audio => "bg-rose-500/15 text-rose-300",
            Self::Publisher => "bg-lime-500/15 text-lime-300",
            Self::Checksum => "bg-indigo-500/15 text-indigo-300",
            Self::Extension => "bg-slate-500/20 text-slate-300",
        }
    }

    /// Tailwind background for the solid dot beside a field name.
    pub(crate) fn dot(self) -> &'static str {
        match self {
            Self::Show => "bg-sky-400",
            Self::Movie => "bg-violet-400",
            Self::Season => "bg-amber-400",
            Self::Episode => "bg-orange-400",
            Self::Year => "bg-teal-400",
            Self::Resolution => "bg-emerald-400",
            Self::Source => "bg-cyan-400",
            Self::Codec => "bg-fuchsia-400",
            Self::Audio => "bg-rose-400",
            Self::Publisher => "bg-lime-400",
            Self::Checksum => "bg-indigo-400",
            Self::Extension => "bg-slate-400",
        }
    }
}

/// A run of characters in a filename, tagged with the part that claimed it.
///
/// [`None`] marks separators and anything no rule matched. The `text` values
/// in order reproduce the filename exactly, so the highlighted render never
/// drifts from the name it describes.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "read by the highlighted-filename render, ahead of a page that renders one"
)]
pub(crate) struct Segment<'a> {
    pub(crate) text: &'a str,
    pub(crate) part: Option<Part>,
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

    /// [`Ruleset::id`] of the ruleset this one narrows, or [`None`] for a base.
    ///
    /// One base ruleset describes a whole family, such as every episode name.
    /// A child narrows it to a single series by replacing one field with a
    /// constant, which saves restating the eight fields that do not change.
    pub(crate) inherits: Option<String>,

    /// The fields this ruleset declares itself.
    ///
    /// For a child this holds only the overrides, so a parent's later edit
    /// reaches every child that did not replace that field. Use
    /// [`Ruleset::resolved_fields`] to get the full list the editor shows.
    pub(crate) fields: Vec<Field>,
}

impl Ruleset {
    /// Every field the editor shows, each tagged with where it came from.
    ///
    /// A base ruleset returns its own fields untouched. A child returns the
    /// parent's fields in the parent's order, so the two editors read the
    /// same way down the page.
    ///
    /// `parent` is the ruleset this one narrows, which the caller resolves
    /// through [`crate::rules::Engine`]. Passing it in is what keeps a
    /// ruleset from reaching for a global list to find its own parent.
    ///
    /// `toggled` names the fields switched since the last save. Each named
    /// field flips to the other state. An inherited field starts overriding,
    /// and an overriding field drops back to the inherited value. The
    /// parent's field seeds a fresh override, so the reader edits a working
    /// pattern rather than a blank.
    pub(crate) fn resolved_fields<'a>(
        &'a self,
        parent: Option<&'a Ruleset>,
        toggled: &[&str],
    ) -> Vec<ResolvedField<'a>> {
        let Some(parent) = parent else {
            return self
                .fields
                .iter()
                .map(|field| ResolvedField {
                    field,
                    source: FieldSource::Own,
                })
                .collect();
        };

        parent
            .fields
            .iter()
            .map(|inherited| {
                let own = self.fields.iter().find(|own| own.name == inherited.name);

                match (own, toggled.contains(&inherited.name.as_str())) {
                    (Some(field), false) => ResolvedField {
                        field,
                        source: FieldSource::Overridden { parent: inherited },
                    },
                    (None, true) => ResolvedField {
                        field: inherited,
                        source: FieldSource::Overridden { parent: inherited },
                    },
                    (_, _) => ResolvedField {
                        field: inherited,
                        source: FieldSource::Inherited,
                    },
                }
            })
            .collect()
    }
}

/// A field as the editor shows it, once inheritance is applied.
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
    /// Declared by a ruleset that inherits nothing.
    Own,

    /// Carried unchanged from the parent.
    Inherited,

    /// Replaces the parent's field, usually narrowing a pattern to a constant.
    Overridden {
        /// The parent's field, shown beside the override it replaced.
        parent: &'a Field,
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

#[allow(
    dead_code,
    reason = "the labels and classes the Matches section dresses a row with, ahead of a page that renders one"
)]
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

/// One extraction rule: the part it fills, and how its value is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) part: Part,
    pub(crate) kind: FieldKind,

    /// The regex this field reads its value with, or `None` to take the
    /// pattern its kind supplies.
    ///
    /// A pattern declared on a premade kind wins over the built-in one. That
    /// is how a child ruleset narrows a season to a single constant while
    /// keeping the kind's normalization.
    pub(crate) pattern: Option<String>,

    pub(crate) required: bool,

    /// Whether this field is part of the key that decides whether two
    /// releases are the same item.
    ///
    /// A different group, resolution, or encode of one episode is still that
    /// episode, so only the fields that name what a release *is* take part.
    pub(crate) identity: bool,
}

impl Field {
    /// Returns the regex that reads this field's value.
    ///
    /// # Panics
    ///
    /// Panics when the field declares no pattern and its kind supplies none.
    /// The rulesets are checked in beside the code, so a text field without a
    /// pattern is a programming error rather than bad input.
    pub(crate) fn matcher(&self) -> &str {
        self.pattern
            .as_deref()
            .or_else(|| self.kind.pattern())
            .expect("a field with no pattern has a premade kind")
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
}
