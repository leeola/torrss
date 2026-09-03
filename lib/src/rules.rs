//! Running the rulesets over a release name.
//!
//! A tracker announces a filename and a torrent client reports another. This
//! module decides whether the two name the same release, which is the whole
//! question the library scan asks.
//!
//! The answer is an [`Identity`], built from the fields a ruleset marks as
//! identity. Normalizing them keeps punctuation and case from turning one
//! episode into two.

use std::cmp::Reverse;
use std::fmt::{self, Display};
use std::sync::LazyLock;

use regex::Regex;
use snafu::{OptionExt, ResultExt, Snafu};

use crate::ruleset::{Field, FieldKind, Ruleset};

/// The rulesets this application ships, compiled once.
///
/// The application declares none. A ruleset the reader never wrote claims
/// releases they never asked for, so the set stays empty until rulesets come
/// from a store the reader writes into.
///
/// # Panics
///
/// Panics when the set fails to compile, which an empty one never does.
pub(crate) static ENGINE: LazyLock<Engine> =
    LazyLock::new(|| Engine::from_rulesets(Vec::new()).expect("an empty ruleset set compiles"));

/// What one ruleset made of a release name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// The ruleset that claimed the name, which is the most specific one.
    pub(crate) ruleset: String,

    /// Every field that matched, in the ruleset's own order.
    pub(crate) values: Vec<(String, String)>,

    pub(crate) identity: Identity,
}

/// What makes two releases the same thing.
///
/// The ruleset here is the root of the inheritance chain rather than the one
/// that claimed the name. A child only narrows what its base describes, so an
/// episode claimed by the base and the same episode claimed by a child are
/// one release, not two.
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

/// Every ruleset, compiled and ordered most specific first.
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

    #[snafu(display("ruleset {ruleset} narrows {parent}, which does not exist"))]
    UnknownParent { ruleset: String, parent: String },

    #[snafu(display("ruleset {ruleset} narrows itself through a cycle"))]
    Cycle { ruleset: String },
}

struct Compiled {
    id: String,

    /// The base of the inheritance chain, which names the identity.
    root: String,

    /// How many rulesets this one narrows, which is what orders the list.
    depth: usize,

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
    /// Compiles every ruleset, most specific first.
    ///
    /// The sort is stable and by depth, so a child precedes the base it
    /// narrows while declaration order still breaks ties among equals.
    ///
    /// # Errors
    ///
    /// Returns the first ruleset that names a parent no other ruleset
    /// declares, that narrows itself through a cycle, or that carries a
    /// pattern the regex engine rejects.
    pub(crate) fn from_rulesets(rulesets: Vec<Ruleset>) -> Result<Self, EngineError> {
        let mut compiled = rulesets
            .iter()
            .map(|ruleset| Compiled::new(ruleset, &rulesets))
            .collect::<Result<Vec<_>, _>>()?;

        compiled.sort_by_key(|ruleset| Reverse(ruleset.depth));

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

    /// The ruleset `ruleset` narrows, or [`None`] when it is a base.
    pub(crate) fn parent(&self, ruleset: &Ruleset) -> Option<&Ruleset> {
        self.ruleset(ruleset.inherits.as_deref()?)
    }

    /// Every ruleset that narrows nothing, which is where the editor starts.
    pub(crate) fn bases(&self) -> impl Iterator<Item = &Ruleset> {
        self.source
            .iter()
            .filter(|ruleset| ruleset.inherits.is_none())
    }

    /// Every ruleset that narrows `base`.
    pub(crate) fn children<'a>(&'a self, base: &'a Ruleset) -> impl Iterator<Item = &'a Ruleset> {
        self.source
            .iter()
            .filter(move |child| child.inherits.as_deref() == Some(base.id.as_str()))
    }

    /// Lists every ruleset that claims `title`, most specific first.
    pub(crate) fn claimants(&self, title: &str) -> Vec<String> {
        self.rulesets
            .iter()
            .filter(|ruleset| captures(ruleset, title).is_some())
            .map(|ruleset| ruleset.id.clone())
            .collect()
    }

    /// Parses `title` with the most specific ruleset that claims it.
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
    /// Walks `ruleset` to the base of its chain and compiles its fields.
    ///
    /// The walk is bounded by the number of rulesets, because a stored set
    /// can name a parent chain that loops back on itself. An unbounded walk
    /// would hang the process on data a reader saved.
    fn new(ruleset: &Ruleset, rulesets: &[Ruleset]) -> Result<Self, EngineError> {
        let find = |id: &str| rulesets.iter().find(|candidate| candidate.id == id);

        let parent = match &ruleset.inherits {
            Some(id) => Some(find(id).context(UnknownParentSnafu {
                ruleset: ruleset.id.clone(),
                parent: id.clone(),
            })?),
            None => None,
        };

        let mut root = ruleset;
        let mut depth = 0;

        while let Some(id) = &root.inherits {
            root = find(id).context(UnknownParentSnafu {
                ruleset: root.id.clone(),
                parent: id.clone(),
            })?;

            depth += 1;

            if depth > rulesets.len() {
                return CycleSnafu {
                    ruleset: ruleset.id.clone(),
                }
                .fail();
            }
        }

        Ok(Self {
            id: ruleset.id.clone(),
            root: root.id.clone(),
            depth,
            fields: ruleset
                .resolved_fields(parent, &[])
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

impl CompiledField {
    fn new(field: &Field, ruleset: &str) -> Result<Self, EngineError> {
        Ok(Self {
            name: field.name.clone(),
            kind: field.kind,
            required: field.required,
            identity: field.identity,
            regex: Regex::new(field.matcher()).context(PatternSnafu {
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
    use crate::ruleset::Ruleset;
    use crate::ruleset::fixture::ENGINE;

    const HOLLOW_1080: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const HOLLOW_720: &str =
        "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
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
    fn child_beats_its_base_for_a_matching_name() {
        let parsed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        assert_eq!(
            parsed.ruleset, "series-hollow-meridian",
            "the most specific ruleset wins"
        );
    }

    #[test]
    fn claimants_lists_child_before_base() {
        assert_eq!(
            ENGINE.claimants(HOLLOW_1080),
            vec!["series-hollow-meridian", "series-episodes"],
            "a child narrows its base, so both claim it"
        );
    }

    #[test]
    fn claimants_of_unmatched_name_is_empty() {
        assert_eq!(ENGINE.claimants(NONSENSE), Vec::<&str>::new());
    }

    #[test]
    fn child_and_base_share_identity() {
        assert_eq!(
            ENGINE.parse(HOLLOW_720).expect("claimed").ruleset,
            "series-episodes",
            "the child requires 1080p, so the base claims this one"
        );
        assert_eq!(
            identity(HOLLOW_720),
            identity(HOLLOW_1080),
            "the identity names the root, not the claimant"
        );
    }

    #[test]
    fn same_episode_from_another_group_shares_identity() {
        assert_eq!(
            identity(HOLLOW_720),
            Identity {
                ruleset: "series-episodes".to_owned(),
                key: vec![
                    "the hollow meridian".to_owned(),
                    "4".to_owned(),
                    "6".to_owned(),
                ],
            },
            "a different group and resolution leave the episode unchanged"
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
            "a pack names its show and resolution, so the child claims it"
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

    /// Names a ruleset that narrows `inherits` and declares no field of its
    /// own, which is all a parent walk reads.
    fn narrowing(id: &str, inherits: Option<&str>) -> Ruleset {
        Ruleset {
            id: id.to_owned(),
            name: id.to_owned(),
            enabled: false,
            inherits: inherits.map(ToOwned::to_owned),
            fields: Vec::new(),
        }
    }

    #[test]
    fn a_parent_chain_that_loops_is_an_error() {
        let Err(error) = Engine::from_rulesets(vec![
            narrowing("first", Some("second")),
            narrowing("second", Some("first")),
        ]) else {
            panic!("a loop never compiles");
        };

        assert!(
            matches!(error, EngineError::Cycle { ref ruleset } if ruleset == "first"),
            "the walk stops rather than hanging: {error}"
        );
    }

    #[test]
    fn a_parent_no_ruleset_declares_is_an_error() {
        let Err(error) = Engine::from_rulesets(vec![narrowing("child", Some("absent"))]) else {
            panic!("an unknown parent never compiles");
        };

        assert!(
            matches!(
                error,
                EngineError::UnknownParent { ref ruleset, ref parent }
                    if ruleset == "child" && parent == "absent"
            ),
            "the message names both ends: {error}"
        );
    }
}
