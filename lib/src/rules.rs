//! Running the rulesets over a release name.
//!
//! A tracker announces a filename and a torrent client reports another. This
//! module decides whether the two name the same release, which is the whole
//! question the library scan asks.
//!
//! The answer is an [`Identity`], built from the fields a ruleset marks as
//! identity. Normalizing them keeps punctuation and case from turning one
//! episode into two.

use std::fmt::{self, Display};

use regex::Regex;
use snafu::{OptionExt, ResultExt, Snafu, ensure};

use crate::parser::{Field, FieldKind, Parser};
use crate::ruleset::{Condition, Ruleset};

/// What one ruleset made of a release name.
///
/// A ruleset claims a name when its regex reads it and every condition it
/// carries holds on what the fields read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// The ruleset that claimed the name, which is the first one declared
    /// that does.
    pub(crate) ruleset: String,

    /// Every field that matched, in the ruleset's own order.
    ///
    /// A ruleset claims a title when its regex reads it and every condition
    /// holds, so these are the values that met the conditions too.
    pub(crate) values: Vec<(String, String)>,

    pub(crate) identity: Identity,
}

/// What makes two releases the same thing.
///
/// The parser named here is the one the claiming ruleset reads with, rather
/// than the ruleset itself. Every ruleset on one parser therefore shares one
/// namespace of releases, so the same episode claimed by two of them is one
/// release.
///
/// A trailing empty part makes the identity a span rather than one release. A
/// season pack captures a show and a season and no episode, so its key ends
/// empty, and it stands for every release that agrees on the parts it does
/// name. See [`Self::spans`] for the spans one release falls inside.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Identity {
    pub(crate) parser: String,

    /// The normalized value of each identity field, in the parser.s order.
    pub(crate) key: Vec<String>,
}

impl Identity {
    /// Returns every span this release falls inside, from the exact key
    /// outward, each rendered in the form the library stores.
    ///
    /// The first entry is the release itself, and each later one drops one
    /// more trailing part. An episode therefore yields its own key, its
    /// season, its show, and the bare ruleset. Testing all of them against
    /// the library is what lets a stored season pack own the episodes it
    /// carries.
    ///
    /// The allocations are proportional to the key, which runs to a handful
    /// of parts, and a page calls this once per row.
    pub(crate) fn spans(&self) -> Vec<String> {
        (0..=self.key.len())
            .rev()
            .map(|kept| {
                let mut key = self.key.clone();
                for part in &mut key[kept..] {
                    part.clear();
                }

                Self {
                    parser: self.parser.clone(),
                    key,
                }
                .to_string()
            })
            .collect()
    }
}

impl Display for Identity {
    /// Renders the form the library table stores.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.parser)?;

        for part in &self.key {
            write!(f, "|{part}")?;
        }

        Ok(())
    }
}

/// Every parser and ruleset the process reads titles with, compiled in
/// declaration order.
pub(crate) struct Engine {
    rulesets: Vec<Compiled>,

    /// The declarations the compiled set was built from.
    ///
    /// Inheritance resolves against this list alone, so an engine built from
    /// a fixture never reaches for the shipped set.
    source: Vec<Ruleset>,

    /// The parsers the set was built with, in declaration order.
    parsers: Vec<Parser>,

    /// The same parsers compiled, positionally aligned with `parsers`.
    ///
    /// A ruleset holds an index into this rather than a regex of its own, so
    /// every ruleset on one parser reads through the one regex compiled for
    /// it.
    compiled_parsers: Vec<CompiledParser>,
}

/// Why a set of parsers and rulesets does not compile into an engine.
///
/// Every variant names what it came from, because a reader who saved a bad
/// rule needs to know which page to open. A parser and a ruleset both
/// compile fields, so the variants they share carry an `owner` that names
/// the kind along with the id, such as `ruleset series-episodes`.
#[derive(Debug, Snafu)]
pub(crate) enum EngineError {
    #[snafu(display("the pattern of field {field} in {owner} is not a valid regex"))]
    Pattern {
        owner: String,
        field: String,
        source: regex::Error,
    },

    #[snafu(display("ruleset {ruleset} reads with parser {parser}, which does not exist"))]
    UnknownParser { ruleset: String, parser: String },

    #[snafu(display("{owner} leaves field {field} without a pattern"))]
    BlankField { owner: String, field: String },

    #[snafu(display("ruleset {ruleset} has a condition on field {field}, which it does not read"))]
    UnknownField { ruleset: String, field: String },

    #[snafu(display(
        "ruleset {ruleset} orders field {field}, which is not a number, season, or episode field"
    ))]
    UnorderedField { ruleset: String, field: String },

    /// Every field compiled alone and the whole did not.
    ///
    /// Two fields that both write one group name is the one way to reach
    /// this, because a name is what the composed regex reads each value by.
    #[snafu(display("the fields of {owner} do not compose into one regex"))]
    Composed { owner: String, source: regex::Error },
}

/// One field's contribution to a ruleset's composed regex.
pub(crate) struct Component<'a> {
    pub(crate) name: &'a str,
    pub(crate) pattern: &'a str,
    pub(crate) required: bool,
    pub(crate) tight: bool,
}

/// Joins every component into the one regex a ruleset matches with.
///
/// A component that names no group of its own gets one named after its field,
/// so every value comes back by name. One that already names that group is
/// wrapped without renaming.
///
/// Each component is wrapped, which scopes an inline flag such as `(?i)` and
/// a top-level alternation to the component that wrote it. An optional
/// component is wrapped again and made skippable, so it skips as a whole and
/// a title that lacks it still claims.
///
/// A component that follows a tight one starts where that one ends, with
/// nothing between. One that follows a field that is not tight starts with a
/// lazy gap, `.*?`, inside its own wrapper, so an optional component skips
/// the gap along with its run and a title that lacks the field still claims.
/// The reader still writes each separator into the component that follows it.
pub(crate) fn compose(components: &[Component<'_>]) -> String {
    let mut composed = String::new();
    let mut gap = false;

    for component in components {
        let names_itself = component
            .pattern
            .contains(&format!("(?<{}>", component.name))
            || component
                .pattern
                .contains(&format!("(?P<{}>", component.name));

        let part = if names_itself {
            format!("(?:{})", component.pattern)
        } else {
            format!("(?<{}>{})", component.name, component.pattern)
        };

        let part = if gap { format!(".*?{part}") } else { part };

        if component.required {
            composed.push_str(&part);
        } else {
            composed.push_str(&format!("(?:{part})?"));
        }

        gap = !component.tight;
    }

    composed
}

struct Compiled {
    id: String,

    /// Where the parser this ruleset reads with sits in the engine's
    /// compiled list.
    ///
    /// An index rather than a copy, because every ruleset on one parser
    /// reads through the single regex compiled for it.
    parser: usize,

    /// Every comparison the ruleset makes on a value the parser read, each
    /// already checked against that parser's fields.
    conditions: Vec<Condition>,
}

/// One list of fields composed into the regex that reads them.
struct CompiledParser {
    /// Every field composed into one regex, which each value comes out of by
    /// its field's name.
    regex: Regex,

    fields: Vec<CompiledField>,
}

struct CompiledField {
    name: String,
    kind: FieldKind,
    identity: bool,
}

impl Engine {
    /// Compiles every parser, then every ruleset, in declaration order.
    ///
    /// A ruleset holds no regex of its own. It names a parser and reads
    /// through the one compiled for it, so two rulesets on one parser share
    /// that regex and the namespace of releases behind it.
    ///
    /// # Errors
    ///
    /// Returns the first parser that leaves a field blank or carries a
    /// pattern the regex engine rejects.
    ///
    /// Returns the first ruleset that names a parser no declaration
    /// carries, or that writes a condition its parser's fields do not
    /// answer.
    pub(crate) fn new(parsers: Vec<Parser>, rulesets: Vec<Ruleset>) -> Result<Self, EngineError> {
        let compiled_parsers = parsers
            .iter()
            .map(|parser| {
                compile_fields(
                    &format!("parser {}", parser.id),
                    &parser.fields.iter().collect::<Vec<_>>(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let compiled = rulesets
            .iter()
            .map(|ruleset| Compiled::new(ruleset, &parsers, &compiled_parsers))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            rulesets: compiled,
            source: rulesets,
            parsers,
            compiled_parsers,
        })
    }

    /// Every ruleset this engine was built from, in declaration order.
    pub(crate) fn rulesets(&self) -> impl Iterator<Item = &Ruleset> {
        self.source.iter()
    }

    /// Every parser this engine was built from, in declaration order.
    pub(crate) fn parsers(&self) -> impl Iterator<Item = &Parser> {
        self.parsers.iter()
    }

    /// Finds the parser named by `id`.
    pub(crate) fn parser(&self, id: &str) -> Option<&Parser> {
        self.parsers.iter().find(|parser| parser.id == id)
    }

    /// Finds the ruleset named by `id`.
    pub(crate) fn ruleset(&self, id: &str) -> Option<&Ruleset> {
        self.source.iter().find(|ruleset| ruleset.id == id)
    }

    /// The parser `ruleset` reads titles with, or [`None`] when the engine
    /// was built without it.
    ///
    /// [`Engine::new`] refuses a ruleset whose parser is absent, so an engine
    /// that compiled always answers this. A caller holding a ruleset from
    /// somewhere else may not.
    pub(crate) fn parser_of(&self, ruleset: &Ruleset) -> Option<&Parser> {
        self.parser(&ruleset.parser)
    }

    /// Every ruleset that reads with `parser`, in declaration order.
    pub(crate) fn rulesets_on<'a>(
        &'a self,
        parser: &'a Parser,
    ) -> impl Iterator<Item = &'a Ruleset> {
        self.source
            .iter()
            .filter(move |one| one.parser == parser.id)
    }

    /// Lists every ruleset that claims `title`, in declaration order.
    ///
    /// A ruleset claims a title when its regex reads it and every condition
    /// holds.
    ///
    /// A parser claims nothing, so one never appears here even when a
    /// ruleset reading with it does.
    pub(crate) fn claimants(&self, title: &str) -> Vec<String> {
        self.rulesets
            .iter()
            .filter(|ruleset| {
                claims(ruleset, &self.compiled_parsers[ruleset.parser], title).is_some()
            })
            .map(|ruleset| ruleset.id.clone())
            .collect()
    }

    /// Parses `title` with the first declared ruleset that claims it.
    ///
    /// Two rulesets that both claim one title are a set the reader wrote to
    /// overlap, and declaration order is what settles it.
    pub(crate) fn parse(&self, title: &str) -> Option<Parsed> {
        self.rulesets.iter().find_map(|ruleset| {
            let parser = &self.compiled_parsers[ruleset.parser];
            let values = claims(ruleset, parser, title)?;

            Some(Parsed {
                ruleset: ruleset.id.clone(),
                identity: ruleset.identity(&self.parsers[ruleset.parser], &parser.fields, &values),
                values,
            })
        })
    }
}

impl Compiled {
    /// Finds the parser `ruleset` reads with and checks its conditions
    /// against that parser's fields.
    ///
    /// `parsers` is the engine's compiled list in declaration order, and the
    /// result indexes into it. Every ruleset on one parser therefore shares
    /// the single regex compiled for it, however many of them there are.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::UnknownParser`] when no parser carries the
    /// named id, and the condition refusals when one names a field the
    /// parser does not read or ranks a field that does not rank.
    fn new(
        ruleset: &Ruleset,
        parsers: &[Parser],
        compiled: &[CompiledParser],
    ) -> Result<Self, EngineError> {
        let index = parsers
            .iter()
            .position(|parser| parser.id == ruleset.parser)
            .context(UnknownParserSnafu {
                ruleset: ruleset.id.clone(),
                parser: ruleset.parser.clone(),
            })?;

        for condition in &ruleset.conditions {
            let field = compiled[index]
                .fields
                .iter()
                .find(|field| field.name == condition.field)
                .context(UnknownFieldSnafu {
                    ruleset: ruleset.id.clone(),
                    field: condition.field.clone(),
                })?;

            ensure!(
                !condition.op.orders()
                    || matches!(
                        field.kind,
                        FieldKind::Number | FieldKind::Season | FieldKind::Episode
                    ),
                UnorderedFieldSnafu {
                    ruleset: ruleset.id.clone(),
                    field: condition.field.clone(),
                }
            );
        }

        Ok(Self {
            id: ruleset.id.clone(),
            parser: index,
            conditions: ruleset.conditions.clone(),
        })
    }

    /// Builds the identity from what the fields captured.
    ///
    /// A missing optional identity field contributes an empty part rather
    /// than shortening the key. Every part keeps its position that way, so
    /// two releases only agree position by position, and a trailing gap
    /// reads as a span over everything inside it rather than as a shorter
    /// key that matches nothing.
    fn identity(
        &self,
        parser: &Parser,
        fields: &[CompiledField],
        values: &[(String, String)],
    ) -> Identity {
        Identity {
            parser: parser.id.clone(),
            key: fields
                .iter()
                .filter(|field| field.identity)
                .map(|field| {
                    values
                        .iter()
                        .find(|(name, _)| *name == field.name)
                        .map_or_else(String::new, |(_, raw)| field.kind.normalize(raw))
                })
                .collect(),
        }
    }
}

/// Composes `fields` in order into the one regex that reads a name apart.
///
/// Each pattern compiles alone first, so a bad regex names the field that
/// carries it rather than the whole list. `owner` names what carries the
/// fields, such as `parser series` or `ruleset series-episodes`, because a
/// reader who has to fix one needs to know which page to open.
///
/// # Errors
///
/// Returns the first field that leaves its pattern blank, or that carries a
/// pattern the regex engine rejects. Returns [`EngineError::Composed`] when
/// every field stands alone and the whole does not, which two fields under
/// one group name is the way to reach.
fn compile_fields(owner: &str, fields: &[&Field]) -> Result<CompiledParser, EngineError> {
    let patterns = fields
        .iter()
        .map(|field| {
            let matcher = field.matcher().context(BlankFieldSnafu {
                owner,
                field: field.name.clone(),
            })?;

            let component = Component {
                name: &field.name,
                pattern: matcher,
                required: field.required,
                tight: field.tight,
            };

            Regex::new(&compose(std::slice::from_ref(&component))).context(PatternSnafu {
                owner,
                field: field.name.clone(),
            })?;

            Ok(matcher)
        })
        .collect::<Result<Vec<_>, EngineError>>()?;

    let components = fields
        .iter()
        .zip(&patterns)
        .map(|(field, pattern)| Component {
            name: &field.name,
            pattern,
            required: field.required,
            tight: field.tight,
        })
        .collect::<Vec<_>>();

    Ok(CompiledParser {
        regex: Regex::new(&compose(&components)).context(ComposedSnafu { owner })?,
        fields: fields
            .iter()
            .map(|field| CompiledField {
                name: field.name.clone(),
                kind: field.kind,
                identity: field.identity,
            })
            .collect(),
    })
}

/// Runs the composed regex over `title`, or reports that the ruleset does not
/// claim it.
///
/// One match answers for every field. A required field is a plain group, so
/// the regex fails without it and the ruleset claims nothing. An optional one
/// is a skippable group that contributes no value when it skips, which is
/// what lets one ruleset claim a feed title and a folder-named torrent.
fn captures(parser: &CompiledParser, title: &str) -> Option<Vec<(String, String)>> {
    let caps = parser.regex.captures(title)?;

    Some(
        parser
            .fields
            .iter()
            .filter_map(|field| {
                let value = caps.name(&field.name)?;

                Some((field.name.clone(), value.as_str().to_owned()))
            })
            .collect(),
    )
}

/// Returns what the fields read from `title`, or reports that the ruleset
/// does not claim it.
///
/// The regex decides which titles have the shape the ruleset describes, and
/// the conditions decide which of those it wants. A condition compares the
/// normalized value rather than the raw capture, so it agrees with the
/// identity the library stores and with a saved test's verdict.
///
/// A condition on a field the title did not carry fails, which is what makes
/// an absent condition the way a ruleset asks for a pack alone.
fn claims(
    ruleset: &Compiled,
    parser: &CompiledParser,
    title: &str,
) -> Option<Vec<(String, String)>> {
    let values = captures(parser, title)?;

    for condition in &ruleset.conditions {
        // `Compiled::new` refused a condition on a name no field carries, so
        // a miss here is a field that read nothing.
        let kind = parser
            .fields
            .iter()
            .find(|field| field.name == condition.field)?
            .kind;

        let read = values
            .iter()
            .find(|(name, _)| *name == condition.field)
            .map(|(_, raw)| kind.normalize(raw));

        if !condition.holds(kind, read.as_deref()) {
            return None;
        }
    }

    Some(values)
}

#[cfg(test)]
mod tests {
    use super::{Component, Engine, EngineError, Identity, compose};
    use crate::parser::{Field, FieldKind, Parser};
    use crate::ruleset::fixture::{self, ENGINE};
    use crate::ruleset::{Condition, Op, Ruleset};

    const HOLLOW_1080: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    /// The resolution the show ruleset requires is absent. The parser reads
    /// the name and no ruleset admits it.
    const HOLLOW_720: &str =
        "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
    /// The same episode from another group. The show ruleset claims it,
    /// because it requires 1080p and names no publisher.
    const HOLLOW_OTHER_GROUP: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
    /// A client names a multi-file torrent after its folder, which carries the
    /// release name and no suffix.
    const HOLLOW_FOLDER: &str = "The.Hollow.Meridian.S04E06.1080p.Broadcast";
    /// A whole season announced as one release. It is a folder like
    /// [`HOLLOW_FOLDER`], so it carries no extension, and it names no episode.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S01.1080p.Broadcast.AAC.Stereo.H.264-PublicWave";
    const OTHER_EPISODE: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const NONSENSE: &str = "just some words with no structure at all";

    fn identity(title: &str) -> Identity {
        ENGINE
            .parse(title)
            .unwrap_or_else(|| panic!("{title} is claimed"))
            .identity
    }

    #[test]
    fn a_ruleset_claims_what_its_conditions_admit() {
        let parsed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        assert_eq!(
            parsed.ruleset, "series-hollow-meridian",
            "the ruleset whose conditions the title meets is what claims it"
        );
    }

    #[test]
    fn claimants_name_rulesets_and_never_a_parser() {
        assert_eq!(
            ENGINE.claimants(HOLLOW_1080),
            vec!["series-hollow-meridian"],
            "a parser claims nothing, so only the ruleset appears"
        );
    }

    #[test]
    fn claimants_of_unmatched_name_is_empty() {
        assert_eq!(ENGINE.claimants(NONSENSE), Vec::<&str>::new());
    }

    #[test]
    fn a_title_the_parser_reads_and_no_ruleset_wants_is_unclaimed() {
        assert_eq!(
            ENGINE.parse(HOLLOW_720),
            None,
            "the parser reads the 720p name, and no ruleset admits that resolution"
        );
    }

    #[test]
    fn identity_names_the_parser() {
        assert_eq!(
            identity(HOLLOW_1080).parser,
            "series-episodes",
            "the identity names the parser, not the ruleset that claimed it"
        );
    }

    #[test]
    fn same_episode_from_another_group_shares_identity() {
        assert_eq!(
            identity(HOLLOW_OTHER_GROUP),
            Identity {
                parser: "series-episodes".to_owned(),
                key: vec![
                    "the hollow meridian".to_owned(),
                    "4".to_owned(),
                    "6".to_owned(),
                ],
            },
            "a different group leaves the episode unchanged"
        );
        assert_eq!(
            identity(HOLLOW_OTHER_GROUP),
            identity(HOLLOW_1080),
            "so the two are one release"
        );
    }

    #[test]
    fn another_episode_differs() {
        assert_ne!(identity(OTHER_EPISODE), identity(HOLLOW_1080));
    }

    #[test]
    fn season_pack_parses_with_an_empty_episode() {
        let parsed = ENGINE.parse(HOLLOW_PACK).expect("claimed");

        assert_eq!(
            parsed.ruleset, "series-hollow-meridian",
            "a pack names its show and resolution, so the ruleset claims it"
        );
        assert_eq!(
            parsed.identity,
            Identity {
                parser: "series-episodes".to_owned(),
                key: vec![
                    "the hollow meridian".to_owned(),
                    "1".to_owned(),
                    String::new(),
                ],
            },
            "the missing episode holds its position in the key"
        );
    }

    #[test]
    fn spans_run_from_the_exact_key_outward() {
        assert_eq!(
            identity(HOLLOW_1080).spans(),
            [
                "series-episodes|the hollow meridian|4|6",
                "series-episodes|the hollow meridian|4|",
                "series-episodes|the hollow meridian||",
                "series-episodes|||",
            ],
            "the episode, then its season, then its show, then the parser"
        );
    }

    #[test]
    fn season_pack_renders_with_a_trailing_empty_part() {
        assert_eq!(
            identity(HOLLOW_PACK).to_string(),
            "series-episodes|the hollow meridian|1|",
            "the form the library stores"
        );
    }

    #[test]
    fn single_digit_season_and_episode_share_identity() {
        assert_eq!(
            identity("The.Hollow.Meridian.S4E6.1080p.Broadcast"),
            identity(HOLLOW_1080),
            "a tracker writes S4E6 where another writes S04E06"
        );
    }

    #[test]
    fn folder_name_without_extension_parses() {
        assert_eq!(
            identity(HOLLOW_FOLDER),
            identity(HOLLOW_1080),
            "a client names a folder with spaces and no extension"
        );
    }

    #[test]
    fn film_identity_is_title_and_year() {
        assert_eq!(
            identity(FILM),
            Identity {
                parser: "feature-films".to_owned(),
                key: vec!["coastal drift".to_owned(), "2024".to_owned()],
            }
        );
    }

    #[test]
    fn unmatched_name_is_none() {
        assert_eq!(ENGINE.parse(NONSENSE), None);
    }

    /// The rendered form is what the library table stores, so a change here
    /// orphans every row already written.
    #[test]
    fn identity_renders_as_the_stored_key() {
        assert_eq!(
            identity(HOLLOW_1080).to_string(),
            "series-episodes|the hollow meridian|4|6"
        );
    }

    /// Names a ruleset on `parser` with the conditions given, which is all
    /// the compile step reads.
    fn on_parser(id: &str, parser: &str, conditions: Vec<Condition>) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            parser: parser.to_owned(),
            conditions,
            tests: Vec::new(),
        }
    }

    /// Names a field with the pattern `pattern`, or a blank when it is
    /// [`None`].
    fn show_field(pattern: Option<&str>) -> Field {
        Field {
            name: "show".to_owned(),
            kind: FieldKind::Text,
            pattern: pattern.map(ToOwned::to_owned),
            required: true,
            tight: true,
            identity: true,
        }
    }

    #[test]
    fn a_ruleset_on_an_absent_parser_is_an_error() {
        let Err(error) = Engine::new(Vec::new(), vec![on_parser("derived", "absent", Vec::new())])
        else {
            panic!("a ruleset with nothing to read titles with never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::UnknownParser { ref ruleset, ref parser }
                    if ruleset == "derived" && parser == "absent"
            ),
            "the message names both ends: {error}"
        );
    }

    #[test]
    fn compose_wraps_each_component_and_skips_an_optional_one() {
        assert_eq!(
            compose(&[
                Component {
                    name: "show",
                    pattern: "^(?<show>.+?)",
                    required: true,
                    tight: true,
                },
                Component {
                    name: "season",
                    pattern: r"\.S(?<season>\d+)",
                    required: true,
                    tight: true,
                },
                Component {
                    name: "episode",
                    pattern: r"E\d+",
                    required: false,
                    tight: false,
                },
                Component {
                    name: "resolution",
                    pattern: r"\.(?<resolution>\d+p)",
                    required: false,
                    tight: false,
                },
            ]),
            concat!(
                r"(?:^(?<show>.+?))(?:\.S(?<season>\d+))(?:(?<episode>E\d+))?",
                r"(?:.*?(?:\.(?<resolution>\d+p)))?",
            ),
            "a component that names its own group is only wrapped, one that does \
             not is named after its field, and one after a field that is not \
             tight starts with the gap"
        );
    }

    #[test]
    fn two_fields_that_name_one_group_do_not_compose() {
        let clashing = parser(
            "clash",
            vec![
                Field {
                    name: "a".to_owned(),
                    ..show_field(Some(r"(?<a>\w)"))
                },
                Field {
                    name: "b".to_owned(),
                    ..show_field(Some(r"(?<a>\w)"))
                },
            ],
        );

        assert!(
            matches!(
                Engine::new(vec![clashing], Vec::new()),
                Err(EngineError::Composed { ref owner, .. }) if owner == "parser clash"
            ),
            "each field compiles alone, and the group name they share fails the whole"
        );
    }

    /// Names a condition, so the rulesets below read as a list of
    /// comparisons rather than a page of struct literals.
    fn condition(field: &str, op: Op, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op,
            value: value.to_owned(),
        }
    }

    /// The fixture's episode parser, which the rulesets below narrow.
    fn episodes() -> Parser {
        fixture::parsers()
            .into_iter()
            .find(|one| one.id == "series-episodes")
            .expect("the fixture declares the episode parser")
    }

    #[test]
    fn a_condition_narrows_what_the_parser_reads() {
        let engine = Engine::new(
            vec![episodes()],
            vec![on_parser(
                "high-definition",
                "series-episodes",
                vec![condition("resolution", Op::Equals, "1080p")],
            )],
        )
        .expect("a condition on a field the parser reads");

        assert_eq!(
            engine.claimants(HOLLOW_1080),
            vec!["high-definition"],
            "the parser reads the resolution and the condition wants this one"
        );
        assert_eq!(
            engine.claimants(HOLLOW_720),
            Vec::<&str>::new(),
            "the same parser reads the other resolution, and the condition refuses it"
        );
    }

    #[test]
    fn a_list_condition_admits_each_listed_value() {
        let engine = Engine::new(
            vec![episodes()],
            vec![on_parser(
                "either-definition",
                "series-episodes",
                vec![condition("resolution", Op::OneOf, "720p, 1080p")],
            )],
        )
        .expect("a list condition on a field the parser reads");

        assert_eq!(engine.claimants(HOLLOW_1080), vec!["either-definition"]);
        assert_eq!(
            engine.claimants(HOLLOW_720),
            vec!["either-definition"],
            "one condition covers both resolutions, where equals needs a ruleset each"
        );
    }

    #[test]
    fn a_condition_on_an_unread_field_is_an_error() {
        let Err(error) = Engine::new(
            vec![parser(
                "ashfall",
                vec![show_field(Some("^(?<show>Ashfall)"))],
            )],
            vec![on_parser(
                "wanted",
                "ashfall",
                vec![condition("resolution", Op::Equals, "1080p")],
            )],
        ) else {
            panic!("a condition on a value no field produces never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::UnknownField { ref ruleset, ref field }
                    if ruleset == "wanted" && field == "resolution"
            ),
            "the message names the field the parser does not read: {error}"
        );
    }

    #[test]
    fn an_ordering_on_a_text_field_is_an_error() {
        let Err(error) = Engine::new(
            vec![parser(
                "ashfall",
                vec![show_field(Some("^(?<show>Ashfall)"))],
            )],
            vec![on_parser(
                "wanted",
                "ashfall",
                vec![condition("show", Op::AtLeast, "10")],
            )],
        ) else {
            panic!("text has no place on a number line");
        };

        assert!(
            matches!(
                error,
                EngineError::UnorderedField { ref ruleset, ref field }
                    if ruleset == "wanted" && field == "show"
            ),
            "the message names the field that does not rank: {error}"
        );
    }

    /// Names a parser over `fields`, which is all the compile step reads.
    fn parser(id: &str, fields: Vec<Field>) -> Parser {
        Parser {
            id: id.to_owned(),
            name: id.to_owned(),
            fields,
            tests: Vec::new(),
        }
    }

    #[test]
    fn a_compiled_parser_is_kept_and_reads_no_title() {
        let series = parser("series", vec![show_field(Some("^(?<show>Ashfall)"))]);

        let engine = Engine::new(vec![series.clone()], Vec::new()).expect("a valid pattern");

        assert_eq!(
            engine.parsers().collect::<Vec<_>>(),
            vec![&series],
            "the parser is kept as it was declared"
        );
        assert_eq!(engine.parser("series"), Some(&series));
        assert_eq!(
            engine.claimants("Ashfall.S01E01"),
            Vec::<&str>::new(),
            "and nothing parses through it, because no ruleset names one yet"
        );
    }

    #[test]
    fn a_parser_with_a_blank_field_is_an_error() {
        let Err(error) = Engine::new(vec![parser("series", vec![show_field(None)])], Vec::new())
        else {
            panic!("a field with no pattern reads no value");
        };

        assert!(
            matches!(
                error,
                EngineError::BlankField { ref owner, ref field }
                    if owner == "parser series" && field == "show"
            ),
            "the message names the parser and the field: {error}"
        );
    }

    #[test]
    fn a_parser_that_does_not_compose_is_an_error() {
        let clashing = parser(
            "clash",
            vec![
                Field {
                    name: "a".to_owned(),
                    ..show_field(Some(r"(?<a>\w)"))
                },
                Field {
                    name: "b".to_owned(),
                    ..show_field(Some(r"(?<a>\w)"))
                },
            ],
        );

        assert!(
            matches!(
                Engine::new(vec![clashing], Vec::new()),
                Err(EngineError::Composed { ref owner, .. }) if owner == "parser clash"
            ),
            "each field compiles alone, and the group name they share fails the whole"
        );
    }
}
