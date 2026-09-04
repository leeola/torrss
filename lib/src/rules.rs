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
/// The ruleset here is the template when the claimant has one, rather than
/// the ruleset that claimed the name. Every ruleset built on one template
/// therefore shares one namespace of releases, so the same episode claimed
/// by two rulesets on one template is one release.
///
/// A trailing empty part makes the identity a span rather than one release. A
/// season pack captures a show and a season and no episode, so its key ends
/// empty, and it stands for every release that agrees on the parts it does
/// name. See [`Self::spans`] for the spans one release falls inside.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Identity {
    pub(crate) ruleset: String,

    /// The normalized value of each identity field, in the ruleset's order.
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
                    ruleset: self.ruleset.clone(),
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
        write!(f, "{}", self.ruleset)?;

        for part in &self.key {
            write!(f, "|{part}")?;
        }

        Ok(())
    }
}

/// Every ruleset that claims titles, compiled in declaration order.
///
/// A template is not among them. It describes the fields the rulesets on it
/// resolve against, and claims nothing itself.
pub(crate) struct Engine {
    rulesets: Vec<Compiled>,

    /// The declarations the compiled set was built from.
    ///
    /// Inheritance resolves against this list alone, so an engine built from
    /// a fixture never reaches for the shipped set.
    source: Vec<Ruleset>,

    /// The parsers the set was built with, each already compiled once.
    ///
    /// Nothing parses through one yet. They are kept so the pages that list
    /// and edit a parser read the same snapshot every other page reads.
    #[allow(dead_code, reason = "the parser pages read this through parsers()")]
    parsers: Vec<Parser>,
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

    #[snafu(display("ruleset {ruleset} is based on {template}, which does not exist"))]
    UnknownTemplate { ruleset: String, template: String },

    #[snafu(display("ruleset {ruleset} is based on {template}, which is not a template"))]
    NotATemplate { ruleset: String, template: String },

    #[snafu(display(
        "template {ruleset} is based on another template, and a template stands alone"
    ))]
    NestedTemplate { ruleset: String },

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

    /// The template this ruleset is based on, which names the identity, or
    /// the ruleset's own id when it is based on nothing.
    root: String,

    parser: CompiledParser,

    /// Every comparison the ruleset makes on a value the regex read, each
    /// already checked against the fields.
    conditions: Vec<Condition>,
}

/// One list of fields composed into the regex that reads them.
///
/// A parser is exactly this, and a ruleset carries one built from the fields
/// it resolved against its template.
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
    /// Compiles every parser, then the rulesets that are not templates, in
    /// declaration order.
    ///
    /// A parser claims nothing, so its compiled regex is not kept. It
    /// compiles here because the reader edits it, and the editor reports on
    /// a set only while every parser in it stands.
    ///
    /// A template claims nothing either, so it never reaches the compiled
    /// list. Its patterns still compile, because the reader edits them and a
    /// bad regex belongs to the template that carries it rather than to the
    /// first ruleset that inherits it.
    ///
    /// # Errors
    ///
    /// Returns the first parser that leaves a field blank or carries a
    /// pattern the regex engine rejects.
    ///
    /// Returns the first ruleset that names a template no other ruleset
    /// declares, that is based on a ruleset that is not a template, that is
    /// a template based on another, or that carries a pattern the regex
    /// engine rejects.
    pub(crate) fn new(parsers: Vec<Parser>, rulesets: Vec<Ruleset>) -> Result<Self, EngineError> {
        for parser in &parsers {
            compile_fields(
                &format!("parser {}", parser.id),
                &parser.fields.iter().collect::<Vec<_>>(),
            )?;
        }

        let mut compiled = Vec::new();

        for ruleset in &rulesets {
            if ruleset.template {
                check_template(ruleset)?;
            } else {
                compiled.push(Compiled::new(ruleset, &rulesets)?);
            }
        }

        Ok(Self {
            rulesets: compiled,
            source: rulesets,
            parsers,
        })
    }

    /// Every ruleset this engine was built from, in declaration order.
    pub(crate) fn rulesets(&self) -> impl Iterator<Item = &Ruleset> {
        self.source.iter()
    }

    /// Every parser this engine was built from, in declaration order.
    #[allow(dead_code, reason = "the parser index lists these")]
    pub(crate) fn parsers(&self) -> impl Iterator<Item = &Parser> {
        self.parsers.iter()
    }

    /// Finds the parser named by `id`.
    #[allow(dead_code, reason = "the parser editor opens the one it is routed to")]
    pub(crate) fn parser(&self, id: &str) -> Option<&Parser> {
        self.parsers.iter().find(|parser| parser.id == id)
    }

    /// Finds the ruleset named by `id`.
    pub(crate) fn ruleset(&self, id: &str) -> Option<&Ruleset> {
        self.source.iter().find(|ruleset| ruleset.id == id)
    }

    /// The template `ruleset` is built on, or [`None`] when it has none.
    pub(crate) fn template_of(&self, ruleset: &Ruleset) -> Option<&Ruleset> {
        self.ruleset(ruleset.based_on.as_deref()?)
    }

    /// Every ruleset based on nothing, template or not, which is where the
    /// index starts.
    pub(crate) fn roots(&self) -> impl Iterator<Item = &Ruleset> {
        self.source
            .iter()
            .filter(|ruleset| ruleset.based_on.is_none())
    }

    /// Every template, which is what a ruleset is allowed to be based on.
    pub(crate) fn templates(&self) -> impl Iterator<Item = &Ruleset> {
        self.source.iter().filter(|ruleset| ruleset.template)
    }

    /// Every ruleset built on `template`.
    pub(crate) fn derived<'a>(
        &'a self,
        template: &'a Ruleset,
    ) -> impl Iterator<Item = &'a Ruleset> {
        self.source
            .iter()
            .filter(move |one| one.based_on.as_deref() == Some(template.id.as_str()))
    }

    /// Lists every ruleset that claims `title`, in declaration order.
    ///
    /// A ruleset claims a title when its regex reads it and every condition
    /// holds.
    ///
    /// A template claims nothing, so one never appears here even when the
    /// ruleset built on it does.
    pub(crate) fn claimants(&self, title: &str) -> Vec<String> {
        self.rulesets
            .iter()
            .filter(|ruleset| claims(ruleset, title).is_some())
            .map(|ruleset| ruleset.id.clone())
            .collect()
    }

    /// Parses `title` with the first declared ruleset that claims it.
    ///
    /// Two rulesets that both claim one title are a set the reader wrote to
    /// overlap, and declaration order is what settles it.
    pub(crate) fn parse(&self, title: &str) -> Option<Parsed> {
        self.rulesets.iter().find_map(|ruleset| {
            let values = claims(ruleset, title)?;

            Some(Parsed {
                ruleset: ruleset.id.clone(),
                identity: ruleset.identity(&values),
                values,
            })
        })
    }
}

impl Compiled {
    /// Resolves `ruleset` against its template and compiles its fields.
    ///
    /// One lookup reaches the template, because a template stands alone.
    /// There is no chain to walk, and therefore none to loop.
    fn new(ruleset: &Ruleset, rulesets: &[Ruleset]) -> Result<Self, EngineError> {
        let template = match &ruleset.based_on {
            Some(id) => {
                let found = rulesets
                    .iter()
                    .find(|candidate| &candidate.id == id)
                    .context(UnknownTemplateSnafu {
                        ruleset: ruleset.id.clone(),
                        template: id.clone(),
                    })?;

                ensure!(
                    found.template,
                    NotATemplateSnafu {
                        ruleset: ruleset.id.clone(),
                        template: id.clone(),
                    }
                );

                Some(found)
            }
            None => None,
        };

        let resolved = ruleset.resolved_fields(template);

        for condition in &ruleset.conditions {
            let field = resolved
                .iter()
                .map(|resolved| resolved.field)
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
            root: template.map_or_else(|| ruleset.id.clone(), |found| found.id.clone()),
            parser: compile_fields(
                &format!("ruleset {}", ruleset.id),
                &resolved
                    .iter()
                    .map(|resolved| resolved.field)
                    .collect::<Vec<_>>(),
            )?,
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
    fn identity(&self, values: &[(String, String)]) -> Identity {
        Identity {
            ruleset: self.root.to_owned(),
            key: self
                .parser
                .fields
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

/// Checks a template without compiling it into the claiming set.
///
/// A template claims nothing, so nothing here is kept. The patterns still
/// compile, because the reader edits them here and a bad regex belongs to
/// the template that carries it.
///
/// A blank field has no pattern to compile. It is what the template exists
/// to declare, so it passes here and is refused only where a ruleset
/// inherits it without writing one.
fn check_template(ruleset: &Ruleset) -> Result<(), EngineError> {
    ensure!(
        ruleset.based_on.is_none(),
        NestedTemplateSnafu {
            ruleset: ruleset.id.clone(),
        }
    );

    for field in &ruleset.fields {
        let Some(matcher) = field.matcher() else {
            continue;
        };

        let component = Component {
            name: &field.name,
            pattern: matcher,
            required: field.required,
            tight: field.tight,
        };

        Regex::new(&compose(std::slice::from_ref(&component))).context(PatternSnafu {
            owner: format!("ruleset {}", ruleset.id),
            field: field.name.clone(),
        })?;
    }

    Ok(())
}

/// Runs the composed regex over `title`, or reports that the ruleset does not
/// claim it.
///
/// One match answers for every field. A required field is a plain group, so
/// the regex fails without it and the ruleset claims nothing. An optional one
/// is a skippable group that contributes no value when it skips, which is
/// what lets one ruleset claim a feed title and a folder-named torrent.
fn captures(ruleset: &Compiled, title: &str) -> Option<Vec<(String, String)>> {
    let caps = ruleset.parser.regex.captures(title)?;

    Some(
        ruleset
            .parser
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
fn claims(ruleset: &Compiled, title: &str) -> Option<Vec<(String, String)>> {
    let values = captures(ruleset, title)?;

    for condition in &ruleset.conditions {
        // `Compiled::new` refused a condition on a name no field carries, so
        // a miss here is a field that read nothing.
        let kind = ruleset
            .parser
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
    /// The resolution the show ruleset requires is absent, so only the
    /// template describes this one and nothing claims it.
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
    fn a_ruleset_on_a_template_claims_its_show() {
        let parsed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        assert_eq!(
            parsed.ruleset, "series-hollow-meridian",
            "the ruleset built on the template is what claims the name"
        );
    }

    #[test]
    fn claimants_never_name_a_template() {
        assert_eq!(
            ENGINE.claimants(HOLLOW_1080),
            vec!["series-hollow-meridian"],
            "the template claims nothing, so only the ruleset appears"
        );
    }

    #[test]
    fn claimants_of_unmatched_name_is_empty() {
        assert_eq!(ENGINE.claimants(NONSENSE), Vec::<&str>::new());
    }

    #[test]
    fn a_title_only_the_template_describes_is_unclaimed() {
        assert_eq!(
            ENGINE.parse(HOLLOW_720),
            None,
            "the ruleset requires 1080p, and the template it is based on claims nothing"
        );
    }

    #[test]
    fn identity_names_the_template() {
        assert_eq!(
            identity(HOLLOW_1080).ruleset,
            "series-episodes",
            "the identity names the template, not the ruleset that claimed it"
        );
    }

    #[test]
    fn same_episode_from_another_group_shares_identity() {
        assert_eq!(
            identity(HOLLOW_OTHER_GROUP),
            Identity {
                ruleset: "series-episodes".to_owned(),
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
                ruleset: "series-episodes".to_owned(),
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
            "the episode, then its season, then its show, then the ruleset"
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
                ruleset: "feature-films".to_owned(),
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

    /// Names a ruleset built on `based_on` that declares no field of its
    /// own, which is all the template resolution reads.
    fn based_on(id: &str, based_on: Option<&str>, template: bool) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            template,
            based_on: based_on.map(ToOwned::to_owned),
            fields: Vec::new(),
            conditions: Vec::new(),
            tests: Vec::new(),
        }
    }

    #[test]
    fn a_template_based_on_a_template_is_an_error() {
        let Err(error) = Engine::new(
            Vec::new(),
            vec![
                based_on("first", None, true),
                based_on("second", Some("first"), true),
            ],
        ) else {
            panic!("a nested template never compiles");
        };

        assert!(
            matches!(error, EngineError::NestedTemplate { ref ruleset } if ruleset == "second"),
            "a template stands alone: {error}"
        );
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
    fn a_blank_template_field_left_unreplaced_is_an_error() {
        let template = Ruleset {
            fields: vec![show_field(None)],
            ..based_on("series", None, true)
        };

        let Err(error) = Engine::new(
            Vec::new(),
            vec![template.clone(), based_on("bare", Some("series"), false)],
        ) else {
            panic!("an unreplaced blank never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::BlankField { ref owner, ref field }
                    if owner == "ruleset bare" && field == "show"
            ),
            "the ruleset owes the template a pattern: {error}"
        );

        let engine = Engine::new(
            Vec::new(),
            vec![
                template,
                Ruleset {
                    fields: vec![show_field(Some("^(?<show>Ashfall)"))],
                    ..based_on("ashfall", Some("series"), false)
                },
            ],
        )
        .expect("a ruleset that replaces the blank compiles");

        assert_eq!(
            engine.claimants("Ashfall.S01E01"),
            vec!["ashfall"],
            "and claims what the pattern it wrote describes"
        );
    }

    #[test]
    fn a_ruleset_based_on_a_non_template_is_an_error() {
        let Err(error) = Engine::new(
            Vec::new(),
            vec![
                based_on("first", None, false),
                based_on("second", Some("first"), false),
            ],
        ) else {
            panic!("a ruleset based on a ruleset never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::NotATemplate { ref ruleset, ref template }
                    if ruleset == "second" && template == "first"
            ),
            "only a template serves as one: {error}"
        );
    }

    #[test]
    fn a_template_no_ruleset_declares_is_an_error() {
        let Err(error) = Engine::new(Vec::new(), vec![based_on("derived", Some("absent"), false)])
        else {
            panic!("an unknown template never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::UnknownTemplate { ref ruleset, ref template }
                    if ruleset == "derived" && template == "absent"
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
        let clashing = Ruleset {
            id: "clash".to_owned(),
            name: "Clash".to_owned(),
            enabled: true,
            template: false,
            based_on: None,
            fields: vec![
                Field {
                    name: "a".to_owned(),
                    kind: FieldKind::Text,
                    pattern: Some(r"(?<a>\w)".to_owned()),
                    required: true,
                    tight: true,
                    identity: true,
                },
                Field {
                    name: "b".to_owned(),
                    kind: FieldKind::Text,
                    pattern: Some(r"(?<a>\w)".to_owned()),
                    required: true,
                    tight: true,
                    identity: false,
                },
            ],
            conditions: Vec::new(),
            tests: Vec::new(),
        };

        assert!(
            matches!(
                Engine::new(Vec::new(), vec![clashing]),
                Err(EngineError::Composed { ref owner, .. }) if owner == "ruleset clash"
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

    /// The fixture's episode template, which the rulesets below narrow.
    fn episodes() -> Ruleset {
        fixture::rulesets()
            .into_iter()
            .find(|one| one.id == "series-episodes")
            .expect("the fixture declares the episode template")
    }

    #[test]
    fn a_condition_narrows_what_the_regex_reads() {
        let engine = Engine::new(
            Vec::new(),
            vec![
                episodes(),
                Ruleset {
                    conditions: vec![condition("resolution", Op::Equals, "1080p")],
                    ..based_on("high-definition", Some("series-episodes"), false)
                },
            ],
        )
        .expect("a condition on a field the template reads");

        assert_eq!(
            engine.claimants(HOLLOW_1080),
            vec!["high-definition"],
            "the regex reads the resolution and the condition wants this one"
        );
        assert_eq!(
            engine.claimants(HOLLOW_720),
            Vec::<&str>::new(),
            "the same regex reads the other resolution, and the condition refuses it"
        );
    }

    #[test]
    fn a_condition_on_an_unread_field_is_an_error() {
        let Err(error) = Engine::new(
            Vec::new(),
            vec![Ruleset {
                fields: vec![show_field(Some("^(?<show>Ashfall)"))],
                conditions: vec![condition("resolution", Op::Equals, "1080p")],
                ..based_on("ashfall", None, false)
            }],
        ) else {
            panic!("a condition on a value no field produces never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::UnknownField { ref ruleset, ref field }
                    if ruleset == "ashfall" && field == "resolution"
            ),
            "the message names the field the ruleset does not read: {error}"
        );
    }

    #[test]
    fn an_ordering_on_a_text_field_is_an_error() {
        let Err(error) = Engine::new(
            Vec::new(),
            vec![Ruleset {
                fields: vec![show_field(Some("^(?<show>Ashfall)"))],
                conditions: vec![condition("show", Op::AtLeast, "10")],
                ..based_on("ashfall", None, false)
            }],
        ) else {
            panic!("text has no place on a number line");
        };

        assert!(
            matches!(
                error,
                EngineError::UnorderedField { ref ruleset, ref field }
                    if ruleset == "ashfall" && field == "show"
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
            panic!("a parser has no template to fill a blank in");
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
