//! Mock feed results and rulesets that stand in for real parsing.
//!
//! Nothing here reads a feed or runs a pattern. The data renders the pages,
//! which puts the design and the navigation in front of a reviewer ahead of
//! the parser. Every show, movie, release group, and feed name is invented.
//!
//! Every value is `'static`, so a handler borrows the data rather than
//! building it per request.

mod candidates;
mod releases;
mod rulesets;

pub(crate) use releases::RELEASES;
pub(crate) use rulesets::RULESETS;

/// A named component that a ruleset pulls out of a release filename.
///
/// The variant picks both the label and the highlight color, so a reader sees
/// at a glance which part of a filename a rule claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub(crate) struct Segment {
    pub(crate) text: &'static str,
    pub(crate) part: Option<Part>,
}

/// One release a mock ruleset claimed.
///
/// This exists to give [`Ruleset::match_count`] something to count. The feed
/// page renders stored items, so nothing here describes a filename any more.
pub(crate) struct Release {
    /// [`Ruleset::id`] of the ruleset that claimed it.
    pub(crate) ruleset: &'static str,
}

/// A set of field rules that together parse one family of filenames.
pub(crate) struct Ruleset {
    /// Stable key that names the ruleset in a URL.
    pub(crate) id: &'static str,

    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    /// Whether the ruleset runs when the process starts.
    ///
    /// Every shipped ruleset declares `false`, so the wanted list stays empty
    /// until the reader chooses what to follow.
    ///
    /// A disabled ruleset filters nothing, so its releases stay out of the
    /// feed. [`crate::server`] holds the live value, which a reader flips at
    /// runtime without editing this declaration.
    pub(crate) enabled: bool,
    pub(crate) feeds: &'static [&'static str],

    /// [`Ruleset::id`] of the ruleset this one narrows, or [`None`] for a base.
    ///
    /// One base ruleset describes a whole family, such as every episode name.
    /// A child narrows it to a single series by replacing one field with a
    /// constant, which saves restating the eight fields that do not change.
    pub(crate) inherits: Option<&'static str>,

    /// The fields this ruleset declares itself.
    ///
    /// For a child this holds only the overrides, so a parent's later edit
    /// reaches every child that did not replace that field. Use
    /// [`Ruleset::resolved_fields`] to get the full list the editor shows.
    pub(crate) fields: &'static [Field],

    /// A filename the editor highlights to preview the rules below it.
    pub(crate) sample: &'static [Segment],

    /// Filenames the editor tests the rules against, shown as a diff.
    pub(crate) candidates: &'static [Candidate],
}

impl Ruleset {
    /// Counts the releases this ruleset claimed.
    pub(crate) fn match_count(&self) -> usize {
        RELEASES
            .iter()
            .filter(|release| release.ruleset == self.id)
            .count()
    }

    /// Counts the candidates sitting in one diff state.
    pub(crate) fn diff_count(&self, diff: Diff) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.diff == diff)
            .count()
    }

    /// The ruleset this one narrows.
    pub(crate) fn parent(&self) -> Option<&'static Ruleset> {
        ruleset(self.inherits?)
    }

    /// The rulesets that narrow this one.
    pub(crate) fn children(&self) -> impl Iterator<Item = &'static Ruleset> {
        RULESETS
            .iter()
            .filter(move |child| child.inherits == Some(self.id))
    }

    /// Every field the editor shows, each tagged with where it came from.
    ///
    /// A base ruleset returns its own fields untouched. A child returns the
    /// parent's fields in the parent's order, so the two editors read the
    /// same way down the page.
    ///
    /// `toggled` names the fields switched since the last save. Each named
    /// field flips to the other state. An inherited field starts overriding,
    /// and an overriding field drops back to the inherited value. The parent's field seeds a fresh
    /// override, so the reader edits a working pattern rather than a blank.
    pub(crate) fn resolved_fields(&self, toggled: &[&str]) -> Vec<ResolvedField> {
        let Some(parent) = self.parent() else {
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

                match (own, toggled.contains(&inherited.name)) {
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
pub(crate) struct ResolvedField {
    /// The value in effect, whether inherited or replaced.
    pub(crate) field: &'static Field,

    pub(crate) source: FieldSource,
}

impl ResolvedField {
    /// Whether the editor greys the row and locks its inputs.
    pub(crate) fn is_inherited(&self) -> bool {
        matches!(self.source, FieldSource::Inherited)
    }
}

/// Where a resolved field came from.
#[derive(Clone, Copy)]
pub(crate) enum FieldSource {
    /// Declared by a ruleset that inherits nothing.
    Own,

    /// Carried unchanged from the parent.
    Inherited,

    /// Replaces the parent's field, usually narrowing a pattern to a constant.
    Overridden {
        /// The parent's field, shown beside the override it replaced.
        parent: &'static Field,
    },
}

/// One filename the editor tests the ruleset against.
pub(crate) struct Candidate {
    /// Stable key that names the candidate in the pin list.
    pub(crate) id: &'static str,

    pub(crate) segments: &'static [Segment],
    pub(crate) diff: Diff,
    pub(crate) feed: &'static str,
}

/// How a candidate's match changed under the edit in progress.
///
/// A [`Diff::Removed`] candidate keeps the highlighting it had before the
/// edit. The row shows what the edit gives up. An unhighlighted filename
/// loses that.
#[derive(Clone, Copy, PartialEq, Eq)]
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

/// Marks a run of text that no rule claimed, such as a separator.
const fn gap(text: &'static str) -> Segment {
    Segment { text, part: None }
}

/// Marks a run of text that `part` claimed.
const fn hit(text: &'static str, part: Part) -> Segment {
    Segment {
        text,
        part: Some(part),
    }
}

/// One extraction rule: the part it fills, and how its value is read.
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) part: Part,
    pub(crate) kind: FieldKind,

    /// The regex this field reads its value with, or `None` to take the
    /// pattern its kind supplies.
    ///
    /// A pattern declared on a premade kind wins over the built-in one. That
    /// is how a child ruleset narrows a season to a single constant while
    /// keeping the kind's normalization.
    pub(crate) pattern: Option<&'static str>,

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
    pub(crate) fn matcher(&self) -> &'static str {
        self.pattern
            .or_else(|| self.kind.pattern())
            .expect("a field with no pattern has a premade kind")
    }
}

/// How a matched string converts before the rest of the app sees it.
///
/// [`Self::Season`] and [`Self::Episode`] are premade kinds: each carries its
/// own pattern, so a ruleset names the kind and writes no regex. Every other
/// kind leaves the pattern to the field.
#[derive(Clone, Copy, PartialEq, Eq)]
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

/// Finds the ruleset named by `id`.
pub(crate) fn ruleset(id: &str) -> Option<&'static Ruleset> {
    RULESETS.iter().find(|ruleset| ruleset.id == id)
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
