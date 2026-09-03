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

use crate::ruleset::{Field, FieldKind, Ruleset};

/// What one ruleset made of a release name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// The ruleset that claimed the name, which is the first one declared
    /// that does.
    pub(crate) ruleset: String,

    /// Every field that matched, in the ruleset's own order.
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
}

/// Why a set of rulesets does not compile into an engine.
///
/// Every variant names the ruleset it came from, because a reader who saved a
/// bad rule needs to know which one to open.
#[derive(Debug, Snafu)]
pub(crate) enum EngineError {
    #[snafu(display("the pattern of field {field} in ruleset {ruleset} is not a valid regex"))]
    Pattern {
        ruleset: String,
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

    #[snafu(display(
        "ruleset {ruleset} leaves field {field} without a pattern, which only a template does"
    ))]
    BlankField { ruleset: String, field: String },
}

struct Compiled {
    id: String,

    /// The template this ruleset is based on, which names the identity, or
    /// the ruleset's own id when it is based on nothing.
    root: String,

    fields: Vec<CompiledField>,
}

struct CompiledField {
    name: String,
    kind: FieldKind,
    required: bool,
    identity: bool,
    regex: Regex,
}

impl Engine {
    /// Compiles the rulesets that are not templates, in declaration order.
    ///
    /// A template claims nothing, so it never reaches the compiled list. Its
    /// patterns still compile, because the reader edits them and a bad regex
    /// belongs to the template that carries it rather than to the first
    /// ruleset that inherits it.
    ///
    /// # Errors
    ///
    /// Returns the first ruleset that names a template no other ruleset
    /// declares, that is based on a ruleset that is not a template, that is
    /// a template based on another, or that carries a pattern the regex
    /// engine rejects.
    pub(crate) fn from_rulesets(rulesets: Vec<Ruleset>) -> Result<Self, EngineError> {
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
        })
    }

    /// Every ruleset this engine was built from, in declaration order.
    pub(crate) fn rulesets(&self) -> impl Iterator<Item = &Ruleset> {
        self.source.iter()
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
    /// A template claims nothing, so one never appears here even when the
    /// ruleset built on it does.
    pub(crate) fn claimants(&self, title: &str) -> Vec<String> {
        self.rulesets
            .iter()
            .filter(|ruleset| captures(ruleset, title).is_some())
            .map(|ruleset| ruleset.id.clone())
            .collect()
    }

    /// Parses `title` with the first declared ruleset that claims it.
    ///
    /// Two rulesets that both claim one title are a set the reader wrote to
    /// overlap, and declaration order is what settles it.
    pub(crate) fn parse(&self, title: &str) -> Option<Parsed> {
        self.rulesets.iter().find_map(|ruleset| {
            let values = captures(ruleset, title)?;

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

        Ok(Self {
            id: ruleset.id.clone(),
            root: template.map_or_else(|| ruleset.id.clone(), |found| found.id.clone()),
            fields: ruleset
                .resolved_fields(template)
                .into_iter()
                .map(|resolved| CompiledField::new(resolved.field, &ruleset.id))
                .collect::<Result<Vec<_>, _>>()?,
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
                .fields
                .iter()
                .filter(|field| field.identity)
                .map(|field| {
                    values
                        .iter()
                        .find(|(name, _)| *name == field.name)
                        .map_or_else(String::new, |(_, raw)| normalize(field.kind, raw))
                })
                .collect(),
        }
    }
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

        Regex::new(matcher).context(PatternSnafu {
            ruleset: ruleset.id.clone(),
            field: field.name.clone(),
        })?;
    }

    Ok(())
}

impl CompiledField {
    fn new(field: &Field, ruleset: &str) -> Result<Self, EngineError> {
        let matcher = field.matcher().context(BlankFieldSnafu {
            ruleset: ruleset.to_owned(),
            field: field.name.clone(),
        })?;

        Ok(Self {
            name: field.name.clone(),
            kind: field.kind,
            required: field.required,
            identity: field.identity,
            regex: Regex::new(matcher).context(PatternSnafu {
                ruleset: ruleset.to_owned(),
                field: field.name.clone(),
            })?,
        })
    }
}

/// Runs every field over `title`, or reports that the ruleset does not claim it.
///
/// A ruleset claims a title only when every required field matched. An
/// optional field that misses contributes nothing and blocks nothing, which
/// is what lets one ruleset claim a feed title and a folder-named torrent.
fn captures(ruleset: &Compiled, title: &str) -> Option<Vec<(String, String)>> {
    let mut values = Vec::new();

    for field in &ruleset.fields {
        // The capture group carries the field's name by convention, but a
        // pattern written without one still works through group 1.
        let matched = field
            .regex
            .captures(title)
            .and_then(|caps| caps.name(&field.name).or_else(|| caps.get(1)))
            .map(|value| value.as_str().to_owned());

        match matched {
            Some(raw) => values.push((field.name.clone(), raw)),
            None if field.required => return None,
            None => {}
        }
    }

    Some(values)
}

/// Reduces a captured value to the form two releases have to agree on.
///
/// A tracker writes `The.Hollow.Meridian` where a torrent client writes
/// `The Hollow Meridian`, and the two capitalize differently. Collapsing
/// separators and case is what makes those one release rather than two.
fn normalize(kind: FieldKind, raw: &str) -> String {
    if matches!(
        kind,
        FieldKind::Number | FieldKind::Season | FieldKind::Episode
    ) {
        // A season or an episode reads with a leading zero on one side and
        // without it on the other.
        if let Ok(number) = raw.trim_start_matches('0').parse::<u64>() {
            return number.to_string();
        }
    }

    raw.to_lowercase()
        .split(['.', '_', '-', ' ', '\t'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{Engine, EngineError, Identity};
    use crate::ruleset::fixture::ENGINE;
    use crate::ruleset::{Field, FieldKind, Part, Ruleset};

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
        }
    }

    #[test]
    fn a_template_based_on_a_template_is_an_error() {
        let Err(error) = Engine::from_rulesets(vec![
            based_on("first", None, true),
            based_on("second", Some("first"), true),
        ]) else {
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
            part: Part::Show,
            kind: FieldKind::Text,
            pattern: pattern.map(ToOwned::to_owned),
            required: true,
            identity: true,
        }
    }

    #[test]
    fn a_blank_template_field_left_unreplaced_is_an_error() {
        let template = Ruleset {
            fields: vec![show_field(None)],
            ..based_on("series", None, true)
        };

        let Err(error) = Engine::from_rulesets(vec![
            template.clone(),
            based_on("bare", Some("series"), false),
        ]) else {
            panic!("an unreplaced blank never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::BlankField { ref ruleset, ref field }
                    if ruleset == "bare" && field == "show"
            ),
            "the ruleset owes the template a pattern: {error}"
        );

        let engine = Engine::from_rulesets(vec![
            template,
            Ruleset {
                fields: vec![show_field(Some("^(?<show>Ashfall)"))],
                ..based_on("ashfall", Some("series"), false)
            },
        ])
        .expect("a ruleset that replaces the blank compiles");

        assert_eq!(
            engine.claimants("Ashfall.S01E01"),
            vec!["ashfall"],
            "and claims what the pattern it wrote describes"
        );
    }

    #[test]
    fn a_ruleset_based_on_a_non_template_is_an_error() {
        let Err(error) = Engine::from_rulesets(vec![
            based_on("first", None, false),
            based_on("second", Some("first"), false),
        ]) else {
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
        let Err(error) = Engine::from_rulesets(vec![based_on("derived", Some("absent"), false)])
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
}
