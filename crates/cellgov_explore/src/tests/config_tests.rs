//! Sanity bounds on the default exploration configuration.

use super::*;

#[test]
fn default_config_has_sane_bounds() {
    let c = ExplorationConfig::default();
    assert!(c.max_schedules > 0);
    assert!(c.max_steps_per_run > 0);
}
