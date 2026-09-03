//! Whether a ruleset still reads a title the way the reader stated.
//!
//! A saved test is the reader's own statement about one title: this ruleset
//! claims it, and each named field reads this value out of it. This module
//! runs that statement against a draft and reports where the two disagree.
//!
//! The check is pure. It takes the rules and the test and touches nothing
//! else, so the editor runs every test on every keystroke.

use crate::ruleset::RulesetTest;
use crate::server::matches::{self, Rule};

/// What a draft made of one saved test.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Every named field read what the test expects.
    Pass,

    /// The rules do not claim the title at all.
    ///
    /// Kept apart from a failure, because the two send the reader to
    /// different places. This one says a rule stopped matching the name;
    /// a failure says the rules read it and read it differently.
    Unclaimed,

    /// The rules claim the title and disagree about what it holds.
    Failed(Vec<Mismatch>),
}

/// One field the draft and the test disagree about.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Mismatch {
    pub(super) field: String,
    pub(super) expected: String,

    /// What the draft read, or [`None`] when the field read nothing and when
    /// no rule carries that name at all.
    ///
    /// The two cases read the same to the reader. The value they asked for
    /// is not there. Renaming a field is the ordinary way to reach the
    /// second, and it reports an unmet expectation rather than an error.
    pub(super) actual: Option<String>,
}

/// Reports whether `rules` read `test`'s title the way the test says.
///
/// A field the test does not name is not checked, so a test pins down the one
/// value the reader cares about and stays silent about the rest.
pub(super) fn verdict(rules: &[Rule], test: &RulesetTest) -> Verdict {
    let Some(values) = matches::values(rules, &test.title) else {
        return Verdict::Unclaimed;
    };

    let mismatches: Vec<Mismatch> = test
        .expected
        .iter()
        .filter_map(|(field, expected)| {
            let actual = values
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, value)| value.clone());

            (actual.as_ref() != Some(expected)).then(|| Mismatch {
                field: field.clone(),
                expected: expected.clone(),
                actual,
            })
        })
        .collect();

    if mismatches.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Failed(mismatches)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Mismatch, Verdict, verdict};
    use crate::ruleset::RulesetTest;
    use crate::server::matches::tests::{TITLE, declared, resolved, saved};

    /// A test over [`TITLE`] expecting `field` to read `expected`.
    fn expecting(field: &str, expected: &str) -> RulesetTest {
        RulesetTest {
            title: TITLE.to_owned(),
            expected: BTreeMap::from([(field.to_owned(), expected.to_owned())]),
        }
    }

    #[test]
    fn a_matching_value_passes() {
        let declared = declared();

        assert_eq!(
            verdict(&saved(&resolved(&declared)), &expecting("season", "4")),
            Verdict::Pass,
            "the season kind drops the leading zero, which is what the test names"
        );
    }

    #[test]
    fn a_differing_value_reports_what_the_draft_read() {
        let declared = declared();

        assert_eq!(
            verdict(&saved(&resolved(&declared)), &expecting("season", "04")),
            Verdict::Failed(vec![Mismatch {
                field: "season".to_owned(),
                expected: "04".to_owned(),
                actual: Some("4".to_owned()),
            }]),
            "a test written against the raw capture fails against the normalized one"
        );
    }

    #[test]
    fn a_field_no_rule_carries_reads_nothing() {
        let declared = declared();

        assert_eq!(
            verdict(&saved(&resolved(&declared)), &expecting("codec", "h 264")),
            Verdict::Failed(vec![Mismatch {
                field: "codec".to_owned(),
                expected: "h 264".to_owned(),
                actual: None,
            }]),
            "an expectation on a field the ruleset dropped goes unmet, not unreported"
        );
    }

    #[test]
    fn a_title_the_rules_refuse_is_unclaimed() {
        let declared = declared();
        let test = RulesetTest {
            title: "just some words".to_owned(),
            expected: BTreeMap::from([("season".to_owned(), "4".to_owned())]),
        };

        assert_eq!(
            verdict(&saved(&resolved(&declared)), &test),
            Verdict::Unclaimed,
            "a rule stopped matching the name, which is not the same as reading it wrong"
        );
    }
}
