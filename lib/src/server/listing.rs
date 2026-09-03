//! Why a stored title does or does not belong on the wanted list.
//!
//! A feed announces everything a tracker carries. A reader wants the small
//! part of that they do not already have and still watch for. This module
//! answers which part that is, and names the reason for every title it turns
//! away, so a page reports what it hid rather than dropping rows in silence.

use std::collections::HashSet;

use crate::rules::{Engine, Parsed};
use crate::ruleset::Part;

/// Where one title stands against the rulesets and the library.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// Claimed by an enabled ruleset and absent from the library.
    Wanted(Parsed),

    /// The library already holds this identity, so another copy adds nothing.
    Owned(Parsed),

    /// The claimant is switched off.
    Disabled(Parsed),

    /// No ruleset claims the title, so nothing is known about it.
    Unmatched,
}

impl Standing {
    /// What the claimant made of the title, or nothing when none claimed it.
    pub(super) fn parsed(&self) -> Option<&Parsed> {
        match self {
            Self::Wanted(parsed) | Self::Owned(parsed) | Self::Disabled(parsed) => Some(parsed),
            Self::Unmatched => None,
        }
    }

    pub(super) fn is_wanted(&self) -> bool {
        matches!(self, Self::Wanted(_))
    }

    /// Names why the row is not wanted, or nothing when it is.
    ///
    /// This is the badge text a hidden row carries, so a reader who asks to
    /// see everything learns why each extra row is there.
    pub(super) fn hidden_label(&self) -> Option<&'static str> {
        match self {
            Self::Wanted(_) => None,
            Self::Owned(_) => Some("owned"),
            Self::Disabled(_) => Some("disabled"),
            Self::Unmatched => Some("unmatched"),
        }
    }
}

/// One value the claiming ruleset read out of a title.
///
/// The part and the identity flag come from the field that captured the
/// value, so a row can tint each value and mark the ones that decide
/// sameness apart from the ones that only describe the release.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedValue {
    pub(super) name: String,
    pub(super) value: String,
    pub(super) part: Part,

    /// Whether this value takes part in the key that decides whether two
    /// releases are the same item.
    pub(super) identity: bool,
}

/// Resolves what a parse captured back to the fields that captured it.
///
/// The order is the ruleset's own field order, which is the order the parts
/// appear in a well-formed name. A captured name with no matching field is
/// dropped: the engine compiles from this same list, so none is expected.
pub(super) fn parsed_values(engine: &Engine, parsed: &Parsed) -> Vec<ParsedValue> {
    let Some(ruleset) = engine.ruleset(&parsed.ruleset) else {
        return Vec::new();
    };

    let fields = ruleset.resolved_fields(engine.parent(ruleset));

    parsed
        .values
        .iter()
        .filter_map(|(name, raw)| {
            let field = fields
                .iter()
                .find(|resolved| resolved.field.name == *name)?
                .field;

            Some(ParsedValue {
                name: field.name.clone(),
                value: raw.clone(),
                part: field.part,
                identity: field.identity,
            })
        })
        .collect()
}

/// Decides where `title` stands.
///
/// Interest follows the most specific claimant alone. A disabled child hides
/// its show even while its base stays enabled, because the child is what
/// describes this release and an enabled base never rescues it.
///
/// A release counts as owned when the library holds it or any span around it.
/// A stored season pack therefore owns each episode of that season, while a
/// stored episode never owns the pack, which carries the rest of the season
/// too.
pub(super) fn standing(
    engine: &Engine,
    enabled: &HashSet<String>,
    owned: &HashSet<String>,
    title: &str,
) -> Standing {
    let Some(parsed) = engine.parse(title) else {
        return Standing::Unmatched;
    };

    if !enabled.contains(&parsed.ruleset) {
        return Standing::Disabled(parsed);
    }

    if parsed
        .identity
        .spans()
        .iter()
        .any(|span| owned.contains(span))
    {
        return Standing::Owned(parsed);
    }

    Standing::Wanted(parsed)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{Standing, parsed_values, standing};
    use crate::ruleset::fixture::ENGINE;

    const HOLLOW_1080: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const HOLLOW_720: &str =
        "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
    const NONSENSE: &str = "just some words with no structure at all";
    /// A whole season announced as one release, named after its folder. It
    /// covers the season the episode constants above name.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S04.1080p.Broadcast.AAC.Stereo.H.264-PublicWave";

    fn parsed(title: &str) -> Standing {
        Standing::Wanted(ENGINE.parse(title).expect("claimed"))
    }

    fn owned_of(title: &str) -> HashSet<String> {
        HashSet::from([ENGINE.parse(title).expect("claimed").identity.to_string()])
    }

    fn enabled(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn enabled_claimant_and_empty_library_is_wanted() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes", "series-hollow-meridian"]),
                &HashSet::new(),
                HOLLOW_1080,
            ),
            parsed(HOLLOW_1080),
        );
    }

    #[test]
    fn identity_in_the_library_is_owned() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes", "series-hollow-meridian"]),
                &owned_of(HOLLOW_1080),
                HOLLOW_1080,
            ),
            Standing::Owned(expected),
        );
    }

    #[test]
    fn owned_season_pack_owns_each_episode() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes", "series-hollow-meridian"]),
                &owned_of(HOLLOW_PACK),
                HOLLOW_1080,
            ),
            Standing::Owned(expected),
            "the pack carries this episode, so the library already holds it"
        );
    }

    #[test]
    fn owned_episode_never_owns_its_pack() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes", "series-hollow-meridian"]),
                &owned_of(HOLLOW_1080),
                HOLLOW_PACK,
            ),
            parsed(HOLLOW_PACK),
            "a pack carries more than the one episode the library holds"
        );
    }

    #[test]
    fn disabled_child_hides_what_its_enabled_base_claims() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes"]),
                &HashSet::new(),
                HOLLOW_1080,
            ),
            Standing::Disabled(expected),
            "the child claims this title, so an enabled base changes nothing"
        );
    }

    #[test]
    fn enabled_base_wants_what_its_child_refuses() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-episodes"]),
                &HashSet::new(),
                HOLLOW_720,
            ),
            parsed(HOLLOW_720),
            "the child requires 1080p, so the base is the most specific claimant"
        );
    }

    #[test]
    fn title_no_ruleset_claims_is_unmatched() {
        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &HashSet::new(), NONSENSE),
            Standing::Unmatched,
        );
    }

    #[test]
    fn every_claimed_standing_carries_its_parse() {
        let claimed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        for standing in [
            Standing::Wanted(claimed.clone()),
            Standing::Owned(claimed.clone()),
            Standing::Disabled(claimed.clone()),
        ] {
            assert_eq!(standing.parsed(), Some(&claimed));
        }

        assert_eq!(Standing::Unmatched.parsed(), None);
    }

    #[test]
    fn a_parse_resolves_to_its_fields_in_order() {
        let parsed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        let values = parsed_values(&ENGINE, &parsed);
        let read: Vec<(&str, &str, bool)> = values
            .iter()
            .map(|value| (value.name.as_str(), value.value.as_str(), value.identity))
            .collect();

        assert_eq!(
            read,
            [
                ("show", "The.Hollow.Meridian", true),
                ("season", "04", true),
                ("episode", "06", true),
                ("resolution", "1080p", false),
                ("source", "Broadcast", false),
                ("audio", "AAC.Stereo", false),
                ("codec", "H.264", false),
                ("publisher", "PublicWave", false),
                ("extension", ".mkv", false),
            ],
        );
    }

    #[test]
    fn a_season_pack_lists_no_episode_value() {
        let parsed = ENGINE.parse(HOLLOW_PACK).expect("claimed");

        assert_eq!(
            parsed_values(&ENGINE, &parsed)
                .iter()
                .map(|value| value.name.clone())
                .collect::<Vec<_>>(),
            ["show", "season", "resolution", "source", "audio", "codec"],
            "a pack names no episode, and the publisher pattern needs an \
             extension after the group, which a folder name has not got"
        );
    }

    #[test]
    fn only_wanted_is_wanted_and_carries_no_label() {
        let claimed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        let labels: Vec<(bool, Option<&str>)> = [
            Standing::Wanted(claimed.clone()),
            Standing::Owned(claimed.clone()),
            Standing::Disabled(claimed),
            Standing::Unmatched,
        ]
        .iter()
        .map(|standing| (standing.is_wanted(), standing.hidden_label()))
        .collect();

        assert_eq!(
            labels,
            [
                (true, None),
                (false, Some("owned")),
                (false, Some("disabled")),
                (false, Some("unmatched")),
            ],
        );
    }
}
