//! Exploration bounds.

/// Upper bounds on an exploration run.
///
/// Exceeding either bound forces `OutcomeClass::Inconclusive` when no
/// divergence has been observed.
#[derive(Debug, Clone)]
pub struct ExplorationConfig {
    /// Maximum number of distinct alternate schedules to explore.
    pub max_schedules: usize,
    /// Maximum runtime steps per individual replay.
    pub max_steps_per_run: usize,
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            max_schedules: 256,
            max_steps_per_run: 10_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_bounds() {
        let c = ExplorationConfig::default();
        assert!(c.max_schedules > 0);
        assert!(c.max_steps_per_run > 0);
    }
}
