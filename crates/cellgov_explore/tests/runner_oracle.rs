//! The canonical testkit runner is the oracle for the optimal search's
//! baseline execution.

use cellgov_explore::{explore_window, ExplorationConfig};
use cellgov_testkit::{
    fixtures,
    runner::{run, ScenarioOutcome},
};

#[test]
fn the_optimal_baseline_matches_the_testkit_runner() {
    let direct = run(fixtures::store_order_scenario(2));
    let explored = explore_window(
        || fixtures::store_order_scenario(2).build_runtime(),
        &ExplorationConfig::default(),
    );

    assert_eq!(direct.outcome, ScenarioOutcome::Stalled);
    assert_eq!(
        explored.baseline_stop,
        cellgov_explore::util::StopReason::Stalled,
    );
    assert_eq!(explored.baseline_steps, direct.steps_taken);
    assert_eq!(explored.baseline_hash, direct.final_memory_hash.raw());
    assert_eq!(
        explored.classes_explored,
        Some(6),
        "four conflicting stores have six order classes",
    );
}
