//! How many invented releases each mock ruleset claimed.
//!
//! The admin page shows a match count beside every ruleset, so the editor
//! reads as though rules had run over a feed. One entry stands for one
//! claimed release.
//!
//! The filenames these entries once carried are gone. The feed page renders
//! stored items now, and nothing else read them.

use super::Release;

pub(crate) const RELEASES: &[Release] = &[
    Release {
        ruleset: "series-episodes",
    },
    Release {
        ruleset: "series-episodes",
    },
    Release {
        ruleset: "series-episodes",
    },
    Release {
        ruleset: "series-episodes",
    },
    Release {
        ruleset: "feature-films",
    },
    Release {
        ruleset: "feature-films",
    },
    Release {
        ruleset: "archive-talks",
    },
    Release {
        ruleset: "archive-talks",
    },
];
