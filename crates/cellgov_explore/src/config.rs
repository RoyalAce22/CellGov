//! Exploration bounds.

/// [`ExplorationConfig::max_schedules`] where the caller names none.
pub const DEFAULT_MAX_SCHEDULES: usize = 256;

/// [`ExplorationConfig::max_steps_per_run`] where the caller names none.
pub const DEFAULT_MAX_STEPS_PER_RUN: usize = 10_000;

/// Upper bounds on an exploration run.
///
/// Exceeding either bound forces `OutcomeClass::Inconclusive` when no
/// divergence has been observed.
#[derive(Debug, Clone)]
pub struct ExplorationConfig {
    /// Maximum number of alternate schedules to record beyond the
    /// baseline.
    ///
    /// One class costs one execution under
    /// [`crate::optimal::explore_optimal`], so the bound there admits
    /// `max_schedules + 1` classes: the baseline's plus one per record.
    /// A search that explores more than one execution per class spends
    /// the same bound on fewer classes.
    pub max_schedules: usize,
    /// Maximum runtime steps per individual replay.
    pub max_steps_per_run: usize,
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            max_schedules: DEFAULT_MAX_SCHEDULES,
            max_steps_per_run: DEFAULT_MAX_STEPS_PER_RUN,
        }
    }
}

#[cfg(test)]
#[path = "tests/config_tests.rs"]
mod tests;
