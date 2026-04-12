//! Schedule decision detection and logging.
//!
//! A decision point occurs when the runtime has more than one runnable
//! unit at a scheduling boundary. The `DecisionLog` records every
//! decision point encountered during a run so the explorer can later
//! identify backtracking opportunities.

use crate::dependency::StepFootprint;
use cellgov_event::UnitId;

/// A single scheduling decision point.
#[derive(Debug, Clone)]
pub struct DecisionPoint {
    /// Step index within the run.
    pub step: usize,
    /// All units that were runnable when the decision was made.
    pub runnable: Vec<UnitId>,
    /// The unit the scheduler actually chose.
    pub chosen: UnitId,
    /// Summary of shared resources the chosen unit touched.
    pub footprint: StepFootprint,
}

impl DecisionPoint {
    /// Whether this decision point had a real choice (more than one
    /// runnable unit). Only branching points are interesting for
    /// schedule exploration.
    pub fn is_branching(&self) -> bool {
        self.runnable.len() > 1
    }
}

/// Log of all decision points from a single run.
#[derive(Debug, Clone, Default)]
pub struct DecisionLog {
    points: Vec<DecisionPoint>,
}

impl DecisionLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a decision point.
    pub fn push(&mut self, point: DecisionPoint) {
        self.points.push(point);
    }

    /// All recorded decision points.
    pub fn points(&self) -> &[DecisionPoint] {
        &self.points
    }

    /// Decision points where more than one unit was runnable.
    pub fn branching_points(&self) -> impl Iterator<Item = &DecisionPoint> {
        self.points.iter().filter(|p| p.is_branching())
    }

    /// Number of branching points (decision points with >1 runnable).
    pub fn branching_count(&self) -> usize {
        self.points.iter().filter(|p| p.is_branching()).count()
    }

    /// Total number of decision points recorded.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Compute the aggregate footprint for a unit across all its
    /// execution steps in this log. Returns `None` if the unit never
    /// ran.
    pub fn aggregate_footprint(&self, uid: UnitId) -> Option<StepFootprint> {
        let mut agg: Option<StepFootprint> = None;
        for p in &self.points {
            if p.chosen == uid {
                match &mut agg {
                    Some(fp) => fp.merge(&p.footprint),
                    None => agg = Some(p.footprint.clone()),
                }
            }
        }
        agg
    }
}

#[cfg(test)]
mod tests {
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
}
