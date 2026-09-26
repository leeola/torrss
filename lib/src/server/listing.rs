//! Why a stored title does or does not belong on the wanted list.
//!
//! A feed announces everything a tracker carries. A reader wants the small
//! part of that they do not already have and still watch for. This module
//! answers which part that is, and names the reason for every title it turns
//! away, so a page reports what it hid rather than dropping rows in silence.

use std::collections::{BTreeMap, HashSet};

use crate::engine::{Engine, Parsed};
use crate::preference::Preferences;
use crate::store::StoredItem;

/// Where one title stands against the searches and the library.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// Claimed by an enabled search and absent from the library.
    Wanted(Parsed),

    /// The library already holds this identity, so another copy adds nothing.
    Owned(Parsed),

    /// The matched search is switched off.
    Disabled(Parsed),

    /// A better copy of this identity is wanted, so this one is not.
    ///
    /// The reader stated which values they prefer, and another release of
    /// the same episode reads better ones. Both are wanted by the same
    /// search, so only the ranking separates them.
    Outranked(Parsed),

    /// No search claims the title, so nothing is known about it.
    Unmatched,
}

impl Standing {
    /// What the matched search made of the title, or nothing when none
    /// claimed it.
    pub(super) fn parsed(&self) -> Option<&Parsed> {
        match self {
            Self::Wanted(parsed)
            | Self::Owned(parsed)
            | Self::Disabled(parsed)
            | Self::Outranked(parsed) => Some(parsed),
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
            Self::Disabled(_) => Some("paused"),
            Self::Outranked(_) => Some("outranked"),
            Self::Unmatched => Some("unmatched"),
        }
    }
}

/// One value the claiming search read out of a title.
///
/// The position and the identity flag come from the field that captured the
/// value, so a row tints each value and marks the ones that decide sameness
/// apart from the ones that only describe the release.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedValue {
    pub(super) name: String,
    pub(super) value: String,

    /// Where the capturing field sits among the parser's fields, which is
    /// what tints the value.
    pub(super) position: usize,

    /// Whether this value takes part in the key that decides whether two
    /// releases are the same item.
    pub(super) identity: bool,
}

/// Resolves what a parse captured back to the fields that captured it.
///
/// The order is the parser's own field order, which is the order the parts
/// appear in a well-formed name. A captured name with no matching field is
/// dropped: the engine compiles from this same list, so none is expected.
pub(super) fn parsed_values(engine: &Engine, parsed: &Parsed) -> Vec<ParsedValue> {
    let Some(parser) = engine.parser(&parsed.parser) else {
        return Vec::new();
    };

    parsed
        .values
        .iter()
        .filter_map(|(name, raw)| {
            let position = parser.fields.iter().position(|field| field.name == *name)?;
            let field = &parser.fields[position];

            Some(ParsedValue {
                name: field.name.clone(),
                value: raw.clone(),
                position,
                identity: field.identity,
            })
        })
        .collect()
}

/// Decides where `title` stands.
///
/// Interest follows the matched search alone. A parser claims nothing, so it
/// never appears among the matches and the enabled set never names one.
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

    if !enabled.contains(&parsed.search) {
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

/// Demotes every wanted release a better copy of its identity outranks.
///
/// A tracker announces one episode in several qualities, and one search
/// wants them all. The reader stated which values they prefer, so only the
/// best copy stays wanted and the rest say why they are hidden.
///
/// A release is outranked only by a strictly better one. Two copies the
/// lists rank alike both stay wanted, because nothing the reader stated
/// separates them.
///
/// Only a wanted release takes part. An owned copy is hidden for a better
/// reason already, and ranking a paused or unmatched row answers a question
/// the reader never asked.
pub(super) fn demote_outranked(
    engine: &Engine,
    preferences: &Preferences,
    standings: &mut [Standing],
) {
    // Keyed in order, so one listing demotes the same rows every time.
    let mut ranked: BTreeMap<String, Vec<(usize, Vec<usize>)>> = BTreeMap::new();

    for (index, standing) in standings.iter().enumerate() {
        let Standing::Wanted(parsed) = standing else {
            continue;
        };

        ranked
            .entry(parsed.identity.to_string())
            .or_default()
            .push((index, preferences.rank(engine, parsed)));
    }

    for group in ranked.values() {
        let Some(best) = group.iter().map(|(_, rank)| rank).min() else {
            continue;
        };

        for (index, _) in group.iter().filter(|(_, rank)| rank > best) {
            if let Standing::Wanted(parsed) = &standings[*index] {
                let parsed = parsed.clone();

                standings[*index] = Standing::Outranked(parsed);
            }
        }
    }
}

/// Whether every word of `query` appears somewhere in `title`.
///
/// Both sides lowercase, and `.`, `_`, and `-` become spaces, because a
/// release name separates its words with those rather than with a space.
/// The words match in any order and anywhere in the title, so a reader types
/// what they remember of a show rather than the front of its release name.
///
/// An empty query holds for every title, which is what an empty box means.
pub(super) fn title_contains(query: &str, title: &str) -> bool {
    let separated = |text: &str| text.to_lowercase().replace(['.', '_', '-'], " ");
    let title = separated(title);

    separated(query)
        .split_whitespace()
        .all(|word| title.contains(word))
}

/// Keeps the first row of every title and drops the rest.
///
/// Two feeds that carry one release store it once each, and the listing
/// shows it once. Rows arrive newest first from [`crate::store::items`], so
/// the kept row is the newest announcement, and its link is what a grab
/// downloads.
///
/// A title two feeds carry under different links is two torrents under one
/// name, and only one of them lists. The feed check warns about each such
/// pair and names both feeds, so the operator learns of the one left out.
pub(super) fn distinct_titles(mut items: Vec<StoredItem>) -> Vec<StoredItem> {
    let mut seen = HashSet::new();
    items.retain(|stored| seen.insert(stored.item.title.clone()));

    items
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use chrono::DateTime;
    use url::Url;

    use super::{
        Standing, demote_outranked, distinct_titles, parsed_values, standing, title_contains,
    };
    use crate::feed::fake;
    use crate::preference::Preferences;
    use crate::search::fixture::ENGINE;
    use crate::store::StoredItem;

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
    fn enabled_search_and_empty_library_is_wanted() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
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
                &enabled(&["series-hollow-meridian"]),
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
                &enabled(&["series-hollow-meridian"]),
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
                &enabled(&["series-hollow-meridian"]),
                &owned_of(HOLLOW_1080),
                HOLLOW_PACK,
            ),
            parsed(HOLLOW_PACK),
            "a pack carries more than the one episode the library holds"
        );
    }

    #[test]
    fn a_disabled_search_hides_what_it_claims() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &HashSet::new(), HOLLOW_1080),
            Standing::Disabled(expected),
            "the search that claims this title is switched off"
        );
    }

    #[test]
    fn a_title_no_search_wants_is_unmatched() {
        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &HashSet::new(), HOLLOW_720),
            Standing::Unmatched,
            "the parser reads the 720p name, and no search admits that resolution"
        );
    }

    #[test]
    fn title_no_search_claims_is_unmatched() {
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
            Standing::Outranked(claimed.clone()),
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
                ("episodeNumber", "06", true),
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
            [
                "show",
                "season",
                "resolution",
                "source",
                "audio",
                "codec",
                "publisher"
            ],
            "a pack names no episode, and the extension is the one other \
             component a folder name leaves out"
        );
    }

    #[test]
    fn only_wanted_is_wanted_and_carries_no_label() {
        let claimed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        let labels: Vec<(bool, Option<&str>)> = [
            Standing::Wanted(claimed.clone()),
            Standing::Owned(claimed.clone()),
            Standing::Disabled(claimed.clone()),
            Standing::Outranked(claimed),
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
                (false, Some("paused")),
                (false, Some("outranked")),
                (false, Some("unmatched")),
            ],
        );
    }

    /// Two invented copies of one film, which the film search claims
    /// whatever their resolution because it writes no condition.
    const FILM_UHD: &str = "Coastal.Drift.2024.2160p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const FILM_HD: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";

    /// A different film, so a different identity.
    const OTHER_FILM: &str = "Tidal.Register.2019.1080p.Archive.AAC.Stereo.H.264-MeridianPress.mkv";

    fn prefers(field: &str, values: &[&str]) -> Preferences {
        Preferences::new(BTreeMap::from([(
            field.to_owned(),
            values.iter().map(|one| (*one).to_owned()).collect(),
        )]))
    }

    fn outranked(title: &str) -> Standing {
        Standing::Outranked(ENGINE.parse(title).expect("matched"))
    }

    fn demoted(preferences: &Preferences, titles: &[&str]) -> Vec<Standing> {
        let mut standings: Vec<Standing> = titles.iter().map(|title| parsed(title)).collect();

        demote_outranked(&ENGINE, preferences, &mut standings);

        standings
    }

    #[test]
    fn a_better_copy_of_one_identity_demotes_the_worse() {
        assert_eq!(
            demoted(
                &prefers("resolution", &["2160p", "1080p"]),
                &[FILM_HD, FILM_UHD]
            ),
            vec![outranked(FILM_HD), parsed(FILM_UHD)],
            "one film in two resolutions, and only the preferred one stays wanted"
        );
    }

    #[test]
    fn two_copies_the_lists_rank_alike_both_stay_wanted() {
        assert_eq!(
            demoted(&prefers("resolution", &["4320p"]), &[FILM_HD, FILM_UHD]),
            vec![parsed(FILM_HD), parsed(FILM_UHD)],
            "neither resolution is named, so nothing the reader stated separates them"
        );
    }

    #[test]
    fn an_owned_copy_neither_demotes_nor_is_demoted() {
        let mut standings = vec![
            Standing::Owned(ENGINE.parse(FILM_UHD).expect("matched")),
            parsed(FILM_HD),
        ];

        demote_outranked(
            &ENGINE,
            &prefers("resolution", &["2160p", "1080p"]),
            &mut standings,
        );

        assert_eq!(
            standings,
            vec![
                Standing::Owned(ENGINE.parse(FILM_UHD).expect("matched")),
                parsed(FILM_HD),
            ],
            "the owned copy is hidden for a better reason and takes no part"
        );
    }

    #[test]
    fn two_identities_never_outrank_each_other() {
        assert_eq!(
            demoted(
                &prefers("resolution", &["2160p", "1080p"]),
                &[FILM_UHD, OTHER_FILM]
            ),
            vec![parsed(FILM_UHD), parsed(OTHER_FILM)],
            "a better resolution of one film says nothing about another film"
        );
    }

    #[test]
    fn empty_preferences_demote_nothing() {
        assert_eq!(
            demoted(&Preferences::default(), &[FILM_HD, FILM_UHD]),
            vec![parsed(FILM_HD), parsed(FILM_UHD)],
            "a reader who stated nothing sees every copy"
        );
    }

    #[test]
    fn a_title_holds_every_word_of_the_query_in_any_order() {
        for (query, holds) in [
            ("hollow meridian", true),
            ("meridian hollow", true),
            ("HoLLoW MeRiDiAn", true),
            ("hollow atlantic", false),
            ("", true),
            ("publicwave", true),
        ] {
            assert_eq!(
                title_contains(query, HOLLOW_1080),
                holds,
                "{query:?} against the 1080p title"
            );
        }
    }

    fn stored(id: i64, title: &str) -> StoredItem {
        StoredItem {
            id,
            feed_url: Url::parse("https://tracker.invalid/rss").expect("the test URL parses"),
            item: fake::item(title),
            first_seen: DateTime::UNIX_EPOCH,
            last_seen: DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn duplicate_titles_keep_the_first_row() {
        assert_eq!(
            distinct_titles(vec![stored(3, "A"), stored(2, "B"), stored(1, "A")]),
            vec![stored(3, "A"), stored(2, "B")],
            "the first row of a title is its newest announcement"
        );
    }
}
