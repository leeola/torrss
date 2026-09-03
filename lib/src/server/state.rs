//! Runtime state the whole application shares.

use std::{collections::HashSet, sync::Mutex};

use crate::rules::Engine;

/// Which rulesets currently run.
///
/// The set lives in memory. A restart returns every ruleset to the value
/// [`crate::ruleset::Ruleset::enabled`] declares.
pub(crate) struct RulesetSwitches {
    enabled: Mutex<HashSet<String>>,
}

impl RulesetSwitches {
    /// Seeds the set from the value each of `engine`'s rulesets declares.
    pub(crate) fn new(engine: &Engine) -> Self {
        Self {
            enabled: Mutex::new(
                engine
                    .rulesets()
                    .filter(|ruleset| ruleset.enabled)
                    .map(|ruleset| ruleset.id.clone())
                    .collect(),
            ),
        }
    }

    pub(crate) fn is_enabled(&self, id: &str) -> bool {
        self.lock().contains(id)
    }

    /// Copies the enabled set, so a caller reads the switches once.
    ///
    /// A listing asks about every row it renders. Answering each through
    /// [`Self::is_enabled`] would take the lock once per row for a set that
    /// cannot change mid-render anyway.
    pub(crate) fn snapshot(&self) -> HashSet<String> {
        self.lock().clone()
    }

    /// Flips one ruleset and returns the state it lands in.
    pub(crate) fn toggle(&self, id: &str) -> bool {
        let mut enabled = self.lock();

        if enabled.remove(id) {
            return false;
        }

        enabled.insert(id.to_owned());
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.enabled
            .lock()
            .expect("the ruleset switch lock is never poisoned")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::RulesetSwitches;
    use crate::ruleset::fixture::ENGINE;

    #[test]
    fn nothing_is_enabled_at_start() {
        assert_eq!(
            RulesetSwitches::new(&ENGINE).snapshot(),
            HashSet::new(),
            "a declared ruleset is a template until the reader switches it on"
        );
    }

    #[test]
    fn snapshot_reflects_a_toggle() {
        let switches = RulesetSwitches::new(&ENGINE);
        switches.toggle("archive-talks");

        assert_eq!(
            switches.snapshot(),
            HashSet::from(["archive-talks".to_owned()])
        );
    }
}
