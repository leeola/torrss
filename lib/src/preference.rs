//! Which copy of one release the reader wants.
//!
//! A search decides whether a release is wanted at all. A preference decides
//! between the copies of one that is. Two qualities of one episode are one
//! identity, and only one of them is worth taking.
//!
//! A preference is one ordered list of values for one parser field name,
//! best first. Keying by field name rather than by search means a reader
//! states once that 2160p beats 1080p, and every search reading that field
//! follows it.
//!
//! A value the list does not name is ranked below every value it does, so a
//! short list is a statement about what the reader wants rather than a
//! filter that hides the rest.

pub(crate) mod store;

use std::collections::BTreeMap;

use crate::engine::{Engine, Parsed};

/// The ordered lists a reader has stated, by field name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Preferences {
    lists: BTreeMap<String, Vec<String>>,
}

impl Preferences {
    pub(crate) fn new(lists: BTreeMap<String, Vec<String>>) -> Self {
        Self { lists }
    }

    /// The values preferred for `field`, best first, or an empty slice when
    /// the reader has stated nothing about it.
    pub(crate) fn values(&self, field: &str) -> &[String] {
        self.lists.get(field).map_or(&[], Vec::as_slice)
    }

    /// Every field the reader has stated a list for, in name order.
    pub(crate) fn fields(&self) -> impl Iterator<Item = &str> {
        self.lists.keys().map(String::as_str)
    }

    /// Places `parsed` in the stated lists, one place per ranked field.
    ///
    /// The smaller vector, compared lexicographically, is the better
    /// release. Comparing two vectors is only meaningful when both come
    /// from one parser, which two copies of one identity always do.
    ///
    /// The walk follows the parser's field order rather than the lists', so
    /// that order is what ranks one field above another. A resolution
    /// declared before a source therefore settles a pair that the source
    /// alone leaves tied.
    ///
    /// A field with no list is skipped, so stating nothing about a field
    /// keeps it out of the comparison entirely.
    ///
    /// A field the release read nothing for ranks at the list's length, as
    /// does a value the list does not name. The two sit below every named
    /// value without being ranked against each other.
    ///
    /// Both sides normalize through the field's kind before they compare, so
    /// a list entry `WEB-DL` meets a read `web dl`.
    pub(crate) fn rank(&self, engine: &Engine, parsed: &Parsed) -> Vec<usize> {
        let Some(parser) = engine.parser(&parsed.parser) else {
            return Vec::new();
        };

        parser
            .fields
            .iter()
            .filter_map(|field| {
                let preferred = self.values(&field.name);

                if preferred.is_empty() {
                    return None;
                }

                let read = parsed
                    .values
                    .iter()
                    .find(|(name, _)| *name == field.name)
                    .map(|(_, value)| field.kind.normalize(value));

                let place = read.and_then(|read| {
                    preferred
                        .iter()
                        .position(|wanted| field.kind.normalize(wanted) == read)
                });

                Some(place.unwrap_or(preferred.len()))
            })
            .collect()
    }
}

/// Every field name a preference list ranks by, in first-seen order.
///
/// The Preferences page lists one card per entry, so the order here is the
/// order a reader meets the fields in.
///
/// An identity field is left out. Every copy of one identity carries the
/// same show and season by construction, so a list over those ranks nothing
/// and only invites the reader to state something with no effect.
pub(crate) fn rankable_fields(engine: &Engine) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();

    for parser in engine.parsers() {
        for field in &parser.fields {
            if field.identity || fields.iter().any(|seen| seen == &field.name) {
                continue;
            }

            fields.push(field.name.clone());
        }
    }

    fields
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Preferences, rankable_fields};
    use crate::engine::Parsed;
    use crate::search::fixture::ENGINE;

    /// One invented film in each resolution the fixture parser reads, plus
    /// one with no resolution at all. The source is required, so every title
    /// carries it.
    const UHD: &str = "Coastal.Drift.2024.2160p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const HD: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const SD: &str = "Coastal.Drift.2024.720p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const NO_RESOLUTION: &str = "Coastal.Drift.2024.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";

    fn parsed(title: &str) -> Parsed {
        ENGINE.parse(title).expect("the film search claims it")
    }

    fn stated(lists: &[(&str, &[&str])]) -> Preferences {
        Preferences::new(
            lists
                .iter()
                .map(|(field, values)| {
                    (
                        (*field).to_owned(),
                        values.iter().map(|one| (*one).to_owned()).collect(),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn a_listed_value_ranks_by_its_place() {
        let wanted = stated(&[("resolution", &["2160p", "1080p"])]);

        assert_eq!(wanted.rank(&ENGINE, &parsed(UHD)), vec![0]);
        assert_eq!(
            wanted.rank(&ENGINE, &parsed(HD)),
            vec![1],
            "the two titles differ only in resolution, so only the place differs"
        );
    }

    #[test]
    fn an_unnamed_value_and_an_absent_one_both_rank_last() {
        let wanted = stated(&[("resolution", &["2160p", "1080p"])]);

        assert_eq!(
            wanted.rank(&ENGINE, &parsed(SD)),
            vec![2],
            "a resolution the list does not name sits below every one it does"
        );
        assert_eq!(
            wanted.rank(&ENGINE, &parsed(NO_RESOLUTION)),
            vec![2],
            "and so does a release that read no resolution at all"
        );
    }

    #[test]
    fn both_sides_normalize_before_they_compare() {
        assert_eq!(
            stated(&[("source", &["remaster"])]).rank(&ENGINE, &parsed(UHD)),
            vec![0],
            "the title reads Remaster, which the kind normalizes to meet the entry"
        );
    }

    #[test]
    fn the_parsers_field_order_ranks_one_field_above_another() {
        let wanted = stated(&[
            ("source", &["Archive", "Remaster"]),
            ("resolution", &["2160p", "1080p"]),
        ]);

        assert_eq!(
            wanted.rank(&ENGINE, &parsed(HD)),
            vec![1, 1],
            "resolution leads because the parser declares it first, whatever the list order"
        );
    }

    #[test]
    fn a_field_with_no_list_is_left_out_of_the_comparison() {
        assert_eq!(
            stated(&[("codec", &[])]).rank(&ENGINE, &parsed(UHD)),
            Vec::<usize>::new(),
            "an empty list states nothing, so it ranks nothing"
        );
    }

    #[test]
    fn rankable_fields_reads_every_non_identity_name_once() {
        assert_eq!(
            rankable_fields(&ENGINE),
            [
                "resolution",
                "source",
                "audio",
                "codec",
                "publisher",
                "extension",
                "checksum"
            ],
            "first-seen order across the parsers, with the identity fields left out"
        );
    }
}
