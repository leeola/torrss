//! Why a stored title does or does not belong on the wanted list.
//!
//! A feed announces everything a tracker carries. A reader wants the small
//! part of that they do not already have and still watch for. This module
//! answers which part that is, and names the reason for every title it turns
//! away, so a page reports what it hid rather than dropping rows in silence.

use std::collections::HashSet;

use crate::engine::{Engine, Identity, Parsed};
use crate::store::StoredItem;

/// Where one title stands against the searches and the library.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Standing {
    /// Claimed by an enabled search and absent from the library.
    Wanted(Parsed),

    /// The library already holds this identity, so another copy adds nothing.
    Owned(Parsed),

    /// The release is a span, such as a season pack, and the library holds a
    /// release inside it.
    ///
    /// The pack repeats part of what the reader has, while the rest of the
    /// season stays missing. The row hides for the overlap, and the page does
    /// not call it owned.
    Overlapping(Parsed),

    /// The last grab found the tracker no longer serving the release, so
    /// another copy of its identity is wanted in its place.
    Gone(Parsed),

    /// The matched search is switched off.
    Disabled(Parsed),

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
            | Self::Overlapping(parsed)
            | Self::Gone(parsed)
            | Self::Disabled(parsed) => Some(parsed),
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
            Self::Overlapping(_) => Some("overlapping"),
            Self::Gone(_) => Some("gone"),
            Self::Disabled(_) => Some("paused"),
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

/// What the client holds, in the two forms a standing asks about.
pub(super) struct Holdings {
    /// Each held identity, in the rendered form [`Identity::spans`] returns.
    identities: HashSet<String>,

    /// Every span a held identity falls inside, from [`Identity::spans`].
    spans: HashSet<String>,
}

impl Holdings {
    pub(super) fn new(held: &[Parsed]) -> Self {
        let mut identities = HashSet::new();
        let mut spans = HashSet::new();
        for parsed in held {
            identities.insert(parsed.identity.to_string());
            spans.extend(parsed.identity.spans());
        }

        Self { identities, spans }
    }

    /// Whether the library holds the release or a span around it.
    ///
    /// A held season pack therefore owns each episode of its season.
    pub(super) fn owns(&self, identity: &Identity) -> bool {
        identity
            .spans()
            .iter()
            .any(|span| self.identities.contains(span))
    }

    /// Whether a held release falls inside `identity`.
    ///
    /// It is also true when the library holds `identity` itself, so a caller
    /// tests [`Self::owns`] first.
    pub(super) fn overlaps(&self, identity: &Identity) -> bool {
        self.spans.contains(&identity.to_string())
    }
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
///
/// A span counts as overlapping when the library holds a release inside it.
/// An announced season pack therefore hides once the client holds one episode
/// of that season. An announced episode never hides for a held neighbor,
/// because an episode holds nothing.
///
/// `gone` says the last grab of the release found the tracker no longer
/// serving it. An owned release stays owned, because the library holds it
/// whatever the tracker serves. A gone span reads as gone rather than as
/// overlapping, because the library holds no copy of it and the tracker no
/// longer serves it.
pub(super) fn standing(
    engine: &Engine,
    enabled: &HashSet<String>,
    holdings: &Holdings,
    gone: bool,
    title: &str,
) -> Standing {
    let Some(parsed) = engine.parse(title) else {
        return Standing::Unmatched;
    };

    if !enabled.contains(&parsed.search) {
        return Standing::Disabled(parsed);
    }

    if holdings.owns(&parsed.identity) {
        return Standing::Owned(parsed);
    }

    if gone {
        return Standing::Gone(parsed);
    }

    if holdings.overlaps(&parsed.identity) {
        return Standing::Overlapping(parsed);
    }

    Standing::Wanted(parsed)
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
    use std::collections::HashSet;

    use chrono::DateTime;
    use url::Url;

    use super::{Holdings, Standing, distinct_titles, parsed_values, standing, title_contains};
    use crate::feed::fake;
    use crate::search::fixture::ENGINE;
    use crate::store::StoredItem;

    const HOLLOW_1080: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const HOLLOW_720: &str =
        "The.Hollow.Meridian.S04E06.720p.Broadcast.AAC.Stereo.H.264-OtherGroup.mkv";
    const HOLLOW_NEXT: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const NONSENSE: &str = "just some words with no structure at all";
    /// A whole season announced as one release, named after its folder. It
    /// covers the season the episode constants above name.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S04.1080p.Broadcast.AAC.Stereo.H.264-PublicWave";

    fn parsed(title: &str) -> Standing {
        Standing::Wanted(ENGINE.parse(title).expect("claimed"))
    }

    fn holdings_of(title: &str) -> Holdings {
        Holdings::new(&[ENGINE.parse(title).expect("claimed")])
    }

    fn nothing() -> Holdings {
        Holdings::new(&[])
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
                &nothing(),
                false,
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
                &holdings_of(HOLLOW_1080),
                false,
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
                &holdings_of(HOLLOW_PACK),
                false,
                HOLLOW_1080,
            ),
            Standing::Owned(expected),
            "the pack carries this episode, so the library already holds it"
        );
    }

    #[test]
    fn a_held_episode_hides_the_pack_of_its_season() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
                &holdings_of(HOLLOW_1080),
                false,
                HOLLOW_PACK,
            ),
            Standing::Overlapping(ENGINE.parse(HOLLOW_PACK).expect("claimed")),
            "the pack repeats an episode the client holds"
        );
    }

    #[test]
    fn a_held_episode_never_hides_another_episode() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
                &holdings_of(HOLLOW_1080),
                false,
                HOLLOW_NEXT,
            ),
            parsed(HOLLOW_NEXT),
            "an episode holds nothing, so a held neighbor says nothing about it"
        );
    }

    #[test]
    fn a_gone_release_is_hidden() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
                &nothing(),
                true,
                HOLLOW_1080,
            ),
            Standing::Gone(expected),
            "the tracker deleted the torrent, so another copy is wanted in its place"
        );
    }

    #[test]
    fn an_owned_release_stays_owned_when_gone() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
                &holdings_of(HOLLOW_1080),
                true,
                HOLLOW_1080,
            ),
            Standing::Owned(expected),
            "the library holds the release, whatever the tracker serves"
        );
    }

    #[test]
    fn a_gone_pack_stays_gone_when_it_overlaps() {
        assert_eq!(
            standing(
                &ENGINE,
                &enabled(&["series-hollow-meridian"]),
                &holdings_of(HOLLOW_1080),
                true,
                HOLLOW_PACK,
            ),
            Standing::Gone(ENGINE.parse(HOLLOW_PACK).expect("claimed")),
            "the tracker no longer serves the pack, and the library holds no copy of it"
        );
    }

    #[test]
    fn a_disabled_search_hides_what_it_claims() {
        let Standing::Wanted(expected) = parsed(HOLLOW_1080) else {
            unreachable!()
        };

        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &nothing(), false, HOLLOW_1080),
            Standing::Disabled(expected),
            "the search that claims this title is switched off"
        );
    }

    #[test]
    fn a_title_no_search_wants_is_unmatched() {
        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &nothing(), false, HOLLOW_720),
            Standing::Unmatched,
            "the parser reads the 720p name, and no search admits that resolution"
        );
    }

    #[test]
    fn title_no_search_claims_is_unmatched() {
        assert_eq!(
            standing(&ENGINE, &HashSet::new(), &nothing(), false, NONSENSE),
            Standing::Unmatched,
        );
    }

    #[test]
    fn every_claimed_standing_carries_its_parse() {
        let claimed = ENGINE.parse(HOLLOW_1080).expect("claimed");

        for standing in [
            Standing::Wanted(claimed.clone()),
            Standing::Owned(claimed.clone()),
            Standing::Overlapping(claimed.clone()),
            Standing::Gone(claimed.clone()),
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
            Standing::Overlapping(claimed.clone()),
            Standing::Gone(claimed.clone()),
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
                (false, Some("overlapping")),
                (false, Some("gone")),
                (false, Some("paused")),
                (false, Some("unmatched")),
            ],
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
