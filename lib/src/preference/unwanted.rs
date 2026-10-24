//! The values a reader refuses, by field.
//!
//! A preference list ranks the copies of a wanted release and hides none of
//! them. A refused value is the one statement that hides a copy. A release
//! whose field reads it is never wanted, whichever search claims it.
//!
//! A refusal is keyed by field name rather than by search, as a list is, so
//! a reader refuses a `CAM` source once and every search that reads a source
//! follows it.

use std::collections::{BTreeMap, BTreeSet};

use crate::engine::{Engine, Parsed};

/// The values a reader refuses, by field name.
///
/// A set has no order of its own. It iterates in value order, which is the
/// order a page lists the values in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Unwanted {
    values: BTreeMap<String, BTreeSet<String>>,
}

impl Unwanted {
    pub(crate) fn new(values: BTreeMap<String, BTreeSet<String>>) -> Self {
        Self { values }
    }

    /// The values refused for `field`, in value order.
    pub(crate) fn values(&self, field: &str) -> impl Iterator<Item = &str> {
        self.values
            .get(field)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// Whether a field of `parsed` reads a refused value.
    ///
    /// Both sides normalize through the field's kind before they compare, so
    /// an entry `cam` refuses a read `CAM`.
    ///
    /// An identity field refuses nothing. Every copy of one identity reads the
    /// same values there, such as its show and season. A refusal there judges
    /// the identity rather than a copy, and a search decides whether an
    /// identity is wanted.
    ///
    /// A parser the engine no longer holds refuses nothing, because no field
    /// then says which kind a value reads with.
    pub(crate) fn refuses(&self, engine: &Engine, parsed: &Parsed) -> bool {
        let Some(parser) = engine.parser(&parsed.parser) else {
            return false;
        };

        // A field the set does not name is skipped before its read value
        // normalizes, so an empty set costs a listing no allocation per row.
        parser
            .fields
            .iter()
            .filter(|field| !field.identity && self.values.contains_key(&field.name))
            .any(|field| {
                let Some((_, read)) = parsed.values.iter().find(|(name, _)| *name == field.name)
                else {
                    return false;
                };

                let read = field.kind.normalize(read);

                self.values(&field.name)
                    .any(|refused| field.kind.normalize(refused) == read)
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::Unwanted;
    use crate::engine::Parsed;
    use crate::search::fixture::ENGINE;

    /// An invented film the fixture's film search claims. Its source reads
    /// `Remaster`.
    const FILM: &str = "Coastal.Drift.2024.2160p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    /// An invented episode the fixture's Hollow Meridian search claims.
    const EPISODE: &str =
        "The.Hollow.Meridian.S04E05.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";

    fn parsed(title: &str) -> Parsed {
        ENGINE.parse(title).expect("a fixture search claims it")
    }

    fn refusing(field: &str, values: &[&str]) -> Unwanted {
        Unwanted::new(BTreeMap::from([(
            field.to_owned(),
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<BTreeSet<_>>(),
        )]))
    }

    #[test]
    fn a_refused_value_refuses_the_release() {
        assert!(
            refusing("source", &["Remaster"]).refuses(&ENGINE, &parsed(FILM)),
            "the film reads the refused source"
        );
        assert!(
            !refusing("source", &["Archive"]).refuses(&ENGINE, &parsed(FILM)),
            "the film reads another source"
        );
    }

    #[test]
    fn both_sides_normalize_before_they_compare() {
        assert!(
            refusing("source", &["remaster"]).refuses(&ENGINE, &parsed(FILM)),
            "the title reads Remaster, which the kind normalizes to meet the entry"
        );
    }

    #[test]
    fn an_empty_set_refuses_nothing() {
        assert!(!Unwanted::default().refuses(&ENGINE, &parsed(FILM)));
    }

    #[test]
    fn an_identity_field_refuses_nothing() {
        assert!(
            !refusing("show", &["The Hollow Meridian"]).refuses(&ENGINE, &parsed(EPISODE)),
            "every copy of the episode reads that show, so the refusal would judge the identity"
        );
    }
}
