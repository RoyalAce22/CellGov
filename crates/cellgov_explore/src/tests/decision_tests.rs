//! Branching-point detection on decision points and counting over the decision log.

use super::*;

#[test]
fn single_runnable_is_not_branching() {
    let p = DecisionPoint {
        step: 0,

        runnable: vec![UnitId::new(0)],
        chosen: UnitId::new(0),
        footprint: StepFootprint::default(),
    };
    assert!(!p.is_branching());
}

#[test]
fn two_runnable_is_branching() {
    let p = DecisionPoint {
        step: 0,

        runnable: vec![UnitId::new(0), UnitId::new(1)],
        chosen: UnitId::new(0),
        footprint: StepFootprint::default(),
    };
    assert!(p.is_branching());
}

#[test]
fn branching_count_filters_correctly() {
    let mut log = DecisionLog::new();
    log.push(DecisionPoint {
        step: 0,

        runnable: vec![UnitId::new(0)],
        chosen: UnitId::new(0),
        footprint: StepFootprint::default(),
    });
    log.push(DecisionPoint {
        step: 1,

        runnable: vec![UnitId::new(0), UnitId::new(1)],
        chosen: UnitId::new(0),
        footprint: StepFootprint::default(),
    });
    log.push(DecisionPoint {
        step: 2,

        runnable: vec![UnitId::new(1)],
        chosen: UnitId::new(1),
        footprint: StepFootprint::default(),
    });
    assert_eq!(log.len(), 3);
    assert_eq!(log.branching_count(), 1);
}
