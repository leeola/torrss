//! What reads a release filename apart.
//!
//! A parser is a list of [`Field`]s in the order a filename carries them.
//! Composed, they become the one regex that cuts a name into the values
//! behind it. A parser answers what a name says and nothing about whether
//! anyone wants it, so it has no switch and claims no title.
//!
//! [`FieldKind`] says how a matched string converts, and a premade kind
//! carries the pattern its fields match with. [`PRESETS`] are the rows a
//! reader starts from, drawn from scene naming. [`Segment`] tags one run of
//! a filename with the field that claimed it, and [`Tint`] is the color that
//! field wears wherever the reader meets it.

pub(crate) mod store;

use std::collections::BTreeMap;

/// One way of reading a family of filenames apart.
///
/// The fields compose in order into a single regex, so a parser reads a name
/// the way the tracker wrote it: left to right, each field claiming its own
/// run. It reads nothing it was not asked to read, and it decides nothing
/// about the release it just described.
///
/// A parser therefore carries no enabled state. Turning one off would mean
/// nothing, because nothing happens on its own when a parser matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parser {
    /// Stable key that names the parser in a URL.
    pub(crate) id: String,

    pub(crate) name: String,

    pub(crate) fields: Vec<Field>,

    /// What the reader expects this parser to read from named titles.
    ///
    /// Nothing outside the editor runs these. They exist so a field change
    /// that breaks a title the reader cared about says so as they type.
    pub(crate) tests: Vec<TitleTest>,
}

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

/// One extraction rule, naming the run it claims and how its value is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) kind: FieldKind,

    /// The regex this field reads its value with, or `None` to take the
    /// pattern its kind supplies.
    ///
    /// It is one component of the ruleset's composed regex, so it names its
    /// own run and its leading separator and nothing else. The components
    /// around it supply the context, so it guards against nothing itself.
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

    /// Whether the next field's run starts where this one ends.
    ///
    /// A run such as a show name has no end of its own. The component
    /// after it is what stops it, so nothing may come between the two. A
    /// field that is not tight lets the next field's run sit anywhere after
    /// its own, which is how a resolution reads past an episode name the
    /// ruleset does not claim.
    pub(crate) tight: bool,

    /// Whether this field is part of the key that decides whether two
    /// releases are the same item.
    ///
    /// A different group, resolution, or encode of one episode is still that
    /// episode, so only the fields that name what a release *is* take part.
    pub(crate) identity: bool,
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
pub(crate) struct TitleTest {
    pub(crate) title: String,

    /// Keyed by field name, in that name's order.
    pub(crate) expected: BTreeMap<String, String>,
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
    /// Each is a component that follows the one before it, so it names its own
    /// run and its leading separator and nothing else.
    ///
    /// The season reads `S01`, `S1`, `Season.1`, and `Season 1`. It checks
    /// nothing about what follows, because the next component consumes that,
    /// while the season stays tight.
    ///
    /// The episode needs no season prefix of its own. The season component
    /// precedes it in field order, so a loose number elsewhere in a title
    /// never reads as an episode.
    ///
    /// Each names its capture group after the preset that carries it, so a
    /// field started from that preset reads the number alone. A field of this
    /// kind under another name reads the whole component, prefix included,
    /// because `compose` in [`crate::rules`] wraps it in a group under that
    /// name.
    pub(crate) fn pattern(self) -> Option<&'static str> {
        match self {
            Self::Season => Some(r"(?i)[. _-](?:S|Season[. _]?)(?<season>\d{1,2})"),
            Self::Episode => Some(r"(?i)E(?<episodeNumber>\d{1,3})"),
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

/// The values a new field row starts with.
///
/// A preset is a starting value and nothing remembers it, so a later edit is
/// the reader's own with no touched flag to carry.
pub(crate) struct Preset {
    pub(crate) name: &'static str,
    pub(crate) kind: FieldKind,
    pub(crate) pattern: Option<&'static str>,
    pub(crate) required: bool,
    pub(crate) identity: bool,
    pub(crate) tight: bool,
}

/// The rows a reader starts from, drawn from scene naming.
///
/// Each is one component of the composed regex, so it carries the separator
/// before its run and nothing after it. A tight preset is one whose run only
/// the next component ends: the show, the movie, the season, and the episode
/// name. Every other preset lets the next component sit anywhere after its
/// own run, so a resolution reads past an episode name the ruleset does not
/// claim. The reader still orders the rows as the tracker orders the tokens.
///
/// Each names its capture group after its preset, because that is the name the
/// row starts with. A reader who renames the field keeps a working rule,
/// because `compose` in [`crate::rules`] then wraps the whole component in a
/// group under the new name.
///
/// The show, movie, season, episode number, and year decide what a release
/// is, so they are required or part of the identity. The rest describe one
/// copy of it, and two copies that differ only in codec are still one
/// release.
///
/// The show, the movie, and the episode name are greedy runs. A run reads to
/// the end of its class when no required component follows it, so a lone show
/// reads a whole `foo.bar`. A required component after it makes the run stop
/// at the last place that component fits, so a title with two season tokens
/// reads the later one. An optional component after it skips rather than
/// reads, so a resolution after an episode name has to be required to be
/// read.
pub(crate) const PRESETS: &[Preset] = &[
    Preset {
        name: "show",
        kind: FieldKind::Text,
        pattern: Some(r"^(?<show>[\w.]+)"),
        required: true,
        identity: true,
        tight: true,
    },
    Preset {
        name: "movie",
        kind: FieldKind::Text,
        pattern: Some(r"^(?<movie>[\w.]+)"),
        required: true,
        identity: true,
        tight: true,
    },
    Preset {
        name: "season",
        kind: FieldKind::Season,
        pattern: None,
        required: true,
        identity: true,
        tight: true,
    },
    Preset {
        name: "episodeNumber",
        kind: FieldKind::Episode,
        pattern: None,
        required: false,
        identity: true,
        tight: false,
    },
    Preset {
        name: "episodeName",
        kind: FieldKind::Text,
        pattern: Some(r"\.(?<episodeName>[\w.]+)"),
        required: false,
        identity: false,
        tight: true,
    },
    Preset {
        name: "year",
        kind: FieldKind::Number,
        pattern: Some(r"\.(?<year>(?:19|20)\d{2})"),
        required: true,
        identity: true,
        tight: false,
    },
    Preset {
        name: "resolution",
        kind: FieldKind::Enum,
        pattern: Some(r"\.(?<resolution>480p|720p|1080p|2160p)"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "source",
        kind: FieldKind::Enum,
        pattern: Some(r"\.(?<source>WEB-?DL|WEBRip|BluRay|BDRip|HDTV|DVDRip|Remux)"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "codec",
        kind: FieldKind::Enum,
        pattern: Some(r"\.(?<codec>[xXhH]\.?26[45]|HEVC|AV1|XviD)"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "audio",
        kind: FieldKind::Text,
        pattern: Some(r"\.(?<audio>DDP?\d\.\d|AAC|DTS(?:-HD)?|TrueHD|Atmos|FLAC)"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "publisher",
        kind: FieldKind::Text,
        pattern: Some(r"-(?<publisher>[A-Za-z0-9]+)"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "checksum",
        kind: FieldKind::Text,
        pattern: Some(r"\[(?<checksum>[0-9A-Fa-f]{8})\]"),
        required: false,
        identity: false,
        tight: false,
    },
    Preset {
        name: "extension",
        kind: FieldKind::Enum,
        pattern: Some(r"(?<extension>\.mkv|\.mp4|\.avi)$"),
        required: false,
        identity: false,
        tight: false,
    },
];

#[cfg(test)]
mod tests {
    use regex::Regex;

    use std::collections::BTreeSet;

    use super::{Field, FieldKind, PRESETS};
    use crate::rules::{Component, Engine, compose};
    use crate::ruleset::Ruleset;

    /// Reads `title` through the pattern `kind` supplies, as the engine does.
    ///
    /// The pattern is one component, so it composes before it compiles.
    fn read(kind: FieldKind, title: &str) -> Option<String> {
        let name = PRESETS
            .iter()
            .find(|preset| preset.kind == kind)
            .expect("a preset carries every premade kind")
            .name;

        let component = Component {
            name,
            pattern: kind.pattern().expect("a premade kind"),
            required: true,
            tight: true,
        };

        let regex = Regex::new(&compose(std::slice::from_ref(&component))).expect("a valid regex");

        regex
            .captures(title)
            .and_then(|caps| caps.name(name))
            .map(|value| value.as_str().to_owned())
    }

    /// The field a new row starts as when the reader picks the preset `name`.
    fn preset_field(name: &str) -> Field {
        let preset = PRESETS
            .iter()
            .find(|preset| preset.name == name)
            .unwrap_or_else(|| panic!("{name} is a preset"));

        Field {
            name: preset.name.to_owned(),
            kind: preset.kind,
            pattern: preset.pattern.map(str::to_owned),
            required: preset.required,
            tight: preset.tight,
            identity: preset.identity,
        }
    }

    /// Reads `title` through a standalone ruleset over `fields`.
    ///
    /// Each value comes back normalized by its field's kind, which is the
    /// form the identity stores.
    fn read_fields(fields: &[Field], title: &str) -> Vec<(String, String)> {
        let engine = Engine::from_rulesets(vec![Ruleset {
            id: "scene".to_owned(),
            name: "Scene".to_owned(),
            enabled: true,
            template: false,
            based_on: None,
            fields: fields.to_vec(),
            conditions: Vec::new(),
            tests: Vec::new(),
        }])
        .expect("the fields compose into one regex");

        engine
            .parse(title)
            .unwrap_or_else(|| panic!("{title} is claimed"))
            .values
            .iter()
            .map(|(name, raw)| {
                let kind = fields
                    .iter()
                    .find(|field| &field.name == name)
                    .expect("a field of the ruleset")
                    .kind;

                (name.clone(), kind.normalize(raw))
            })
            .collect()
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
                Some("12".to_owned()),
                None,
            ],
            "season read from each title, and the guard belongs to the next component"
        );
    }

    #[test]
    fn episode_kind_reads_the_number_behind_e() {
        assert_eq!(
            read(FieldKind::Episode, "Show.S04E06"),
            Some("06".to_owned()),
            "the season component precedes this one, so it needs no prefix of its own"
        );

        assert_eq!(
            read(FieldKind::Episode, "[OpenReel] Coastal.Ecology - 18"),
            None,
            "a loose number carries no E"
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

    #[test]
    fn every_preset_pattern_compiles_under_its_own_name() {
        for preset in PRESETS {
            let Some(pattern) = preset.pattern else {
                continue;
            };

            let regex = Regex::new(pattern)
                .unwrap_or_else(|error| panic!("{} does not compile: {error}", preset.name));

            assert!(
                regex
                    .capture_names()
                    .flatten()
                    .any(|name| name == preset.name),
                "{} carries no group under its own name",
                preset.name
            );
        }

        assert_eq!(
            PRESETS
                .iter()
                .map(|preset| preset.name)
                .collect::<BTreeSet<_>>()
                .len(),
            13,
            "each preset names a distinct field, so two never collide in one ruleset"
        );
    }

    #[test]
    fn presets_in_scene_order_read_a_scene_title() {
        let fields = [
            "show",
            "season",
            "episodeNumber",
            "resolution",
            "source",
            "codec",
            "audio",
            "publisher",
            "extension",
        ]
        .map(preset_field);

        assert_eq!(
            read_fields(
                &fields,
                "Coastal.Ecology.S02E05.1080p.WEB-DL.x265.DDP5.1-OpenReel.mkv"
            ),
            [
                ("show", "coastal ecology"),
                ("season", "2"),
                ("episodeNumber", "5"),
                ("resolution", "1080p"),
                ("source", "web dl"),
                ("codec", "x265"),
                ("audio", "ddp5 1"),
                ("publisher", "openreel"),
                ("extension", "mkv"),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned())),
            "each preset reads its own run when the rows follow the tracker's order"
        );
    }

    #[test]
    fn presets_read_past_a_run_no_field_claims() {
        let fields = ["show", "season", "episodeNumber", "resolution"].map(preset_field);

        assert_eq!(
            [
                read_fields(
                    &fields,
                    "Coastal.Ecology.S02E05.Some.Episode.Name.1080p.WEB-DL.x265.DDP5.1-OpenReel.mkv"
                ),
                read_fields(
                    &fields,
                    "Coastal.Ecology.S02E05.Some.Name.WEB-DL.x265-OpenReel.mkv"
                ),
            ],
            [
                vec![
                    ("show".to_owned(), "coastal ecology".to_owned()),
                    ("season".to_owned(), "2".to_owned()),
                    ("episodeNumber".to_owned(), "5".to_owned()),
                    ("resolution".to_owned(), "1080p".to_owned()),
                ],
                vec![
                    ("show".to_owned(), "coastal ecology".to_owned()),
                    ("season".to_owned(), "2".to_owned()),
                    ("episodeNumber".to_owned(), "5".to_owned()),
                ],
            ],
            "the resolution reads past a run no field claims, and skips where the title carries none"
        );
    }

    #[test]
    fn episode_name_reads_up_to_a_required_guard() {
        // The resolution is the required component the greedy name runs up to.
        // Every other row keeps what its preset ships.
        let fields = [
            "show",
            "season",
            "episodeNumber",
            "episodeName",
            "resolution",
        ]
        .map(|name| {
            let field = preset_field(name);

            Field {
                required: field.required || name == "resolution",
                ..field
            }
        });

        assert_eq!(
            [
                read_fields(&fields, "Coastal.Ecology.S02E05.The.Tide.Line.1080p.mkv"),
                read_fields(&fields, "Coastal.Ecology.S02E05.1080p.mkv"),
            ],
            [
                vec![
                    ("show".to_owned(), "coastal ecology".to_owned()),
                    ("season".to_owned(), "2".to_owned()),
                    ("episodeNumber".to_owned(), "5".to_owned()),
                    ("episodeName".to_owned(), "the tide line".to_owned()),
                    ("resolution".to_owned(), "1080p".to_owned()),
                ],
                vec![
                    ("show".to_owned(), "coastal ecology".to_owned()),
                    ("season".to_owned(), "2".to_owned()),
                    ("episodeNumber".to_owned(), "5".to_owned()),
                    ("resolution".to_owned(), "1080p".to_owned()),
                ],
            ],
            "the name runs up to the required resolution, and skips where the title carries none"
        );
    }

    #[test]
    fn a_show_preset_alone_reads_the_whole_title() {
        assert_eq!(
            read_fields(&["show"].map(preset_field), "Coastal.Ecology"),
            [("show".to_owned(), "coastal ecology".to_owned())],
            "a run with no component after it reads to the end of its class"
        );
    }

    #[test]
    fn a_show_that_is_not_tight_reads_up_to_the_season() {
        let fields = [
            Field {
                tight: false,
                ..preset_field("show")
            },
            preset_field("season"),
        ];

        assert_eq!(
            read_fields(
                &fields,
                "Coastal.Ecology.S02E05.1080p.WEB-DL.x265.DDP5.1-OpenReel.mkv"
            ),
            [
                ("show".to_owned(), "coastal ecology".to_owned()),
                ("season".to_owned(), "2".to_owned()),
            ],
            "the gap after a show that is not tight does not cut the run short"
        );
    }
}
