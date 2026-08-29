//! Runtime state the whole application shares.

use std::{collections::HashSet, sync::Mutex};

use crate::mock::RULESETS;

/// Which rulesets currently run.
///
/// The set lives in memory. A restart returns every ruleset to the value
/// [`crate::mock::Ruleset::enabled`] declares.
pub(crate) struct RulesetSwitches {
    enabled: Mutex<HashSet<&'static str>>,
}

impl RulesetSwitches {
    /// Seeds the set from the value each ruleset declares.
    pub(crate) fn new() -> Self {
        Self {
            enabled: Mutex::new(
                RULESETS
                    .iter()
                    .filter(|ruleset| ruleset.enabled)
                    .map(|ruleset| ruleset.id)
                    .collect(),
            ),
        }
    }

    pub(crate) fn is_enabled(&self, id: &str) -> bool {
        self.lock().contains(id)
    }

    /// Flips one ruleset and returns the state it lands in.
    pub(crate) fn toggle(&self, id: &'static str) -> bool {
        let mut enabled = self.lock();

        if enabled.remove(id) {
            return false;
        }

        enabled.insert(id);
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<&'static str>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.enabled
            .lock()
            .expect("the ruleset switch lock is never poisoned")
    }
}
