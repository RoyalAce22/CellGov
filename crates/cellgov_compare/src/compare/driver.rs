//! Field-by-field driver functions: `compare` for a single pair and
//! `compare_multi` for a single CellGov observation against multiple
//! oracle baselines.

use crate::compare::events::find_event_divergence;
use crate::compare::memory::find_memory_divergence;
use crate::compare::types::{Classification, CompareMode, CompareResult, MultiCompareResult};
use crate::observation::Observation;

/// Compare two observations under `mode`, returning the first differing field.
pub fn compare(expected: &Observation, actual: &Observation, mode: CompareMode) -> CompareResult {
    let outcome_mismatch = if expected.outcome != actual.outcome {
        Some((expected.outcome, actual.outcome))
    } else {
        None
    };

    let memory_divergence = match mode {
        CompareMode::Strict | CompareMode::Memory => {
            find_memory_divergence(&expected.memory_regions, &actual.memory_regions)
        }
        CompareMode::Events | CompareMode::Prefix => None,
    };

    let event_divergence = match mode {
        CompareMode::Strict | CompareMode::Events => {
            find_event_divergence(&expected.events, &actual.events, false)
        }
        CompareMode::Prefix => find_event_divergence(&expected.events, &actual.events, true),
        CompareMode::Memory => None,
    };

    let classification = if outcome_mismatch.is_none()
        && memory_divergence.is_none()
        && event_divergence.is_none()
    {
        Classification::Match
    } else {
        Classification::Divergence
    };

    CompareResult {
        classification,
        mode,
        outcome_mismatch,
        memory_divergence,
        event_divergence,
    }
}

/// Compare a CellGov observation against multiple baselines.
///
/// # Panics
///
/// Panics if `baselines` is empty.
pub fn compare_multi(
    baselines: &[Observation],
    cellgov: &Observation,
    mode: CompareMode,
) -> MultiCompareResult {
    assert!(!baselines.is_empty(), "at least one baseline required");

    for i in 1..baselines.len() {
        let oracle_cmp = compare(&baselines[0], &baselines[i], mode);
        if oracle_cmp.classification == Classification::Divergence {
            return MultiCompareResult {
                classification: Classification::UnsettledOracle,
                mode,
                oracle_divergence: Some(oracle_cmp),
                cellgov_result: None,
            };
        }
    }

    let result = compare(&baselines[0], cellgov, mode);
    let classification = result.classification;
    MultiCompareResult {
        classification,
        mode,
        oracle_divergence: None,
        cellgov_result: Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{ObservedEventKind, ObservedOutcome};
    use crate::test_support::{event, obs, region};

    #[test]
    fn identical_observations_match_in_all_modes() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1, 2, 3])],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = a.clone();
        for mode in [
            CompareMode::Strict,
            CompareMode::Memory,
            CompareMode::Events,
            CompareMode::Prefix,
        ] {
            let r = compare(&a, &b, mode);
            assert_eq!(r.classification, Classification::Match, "mode: {mode:?}");
        }
    }

    #[test]
    fn outcome_mismatch_is_divergence() {
        let a = obs(ObservedOutcome::Completed, vec![], vec![]);
        let b = obs(ObservedOutcome::Timeout, vec![], vec![]);
        let r = compare(&a, &b, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Divergence);
        assert_eq!(
            r.outcome_mismatch,
            Some((ObservedOutcome::Completed, ObservedOutcome::Timeout))
        );
    }

    #[test]
    fn memory_mode_ignores_event_differences() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![event(ObservedEventKind::UnitBlock, 5, 0)],
        );
        let r = compare(&a, &b, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Match);
        assert!(r.event_divergence.is_none());
    }

    #[test]
    fn events_mode_ignores_memory_differences() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![99])],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let r = compare(&a, &b, CompareMode::Events);
        assert_eq!(r.classification, Classification::Match);
        assert!(r.memory_divergence.is_none());
    }

    #[test]
    fn strict_mode_catches_both_memory_and_event_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![2])],
            vec![event(ObservedEventKind::UnitBlock, 0, 0)],
        );
        let r = compare(&a, &b, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Divergence);
        assert!(r.memory_divergence.is_some());
        assert!(r.event_divergence.is_some());
    }

    #[test]
    fn empty_observations_match() {
        let a = obs(ObservedOutcome::Completed, vec![], vec![]);
        let b = obs(ObservedOutcome::Completed, vec![], vec![]);
        let r = compare(&a, &b, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Match);
    }

    #[test]
    fn multi_single_baseline_match() {
        let baseline = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![],
        );
        let cellgov = baseline.clone();
        let r = compare_multi(&[baseline], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Match);
        assert!(r.oracle_divergence.is_none());
        assert!(r.cellgov_result.is_some());
    }

    #[test]
    fn multi_single_baseline_divergence() {
        let baseline = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![],
        );
        let cellgov = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![2])],
            vec![],
        );
        let r = compare_multi(&[baseline], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        assert!(r.cellgov_result.is_some());
    }

    #[test]
    fn multi_agreeing_oracles_match_cellgov() {
        let b1 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![0xAA])],
            vec![],
        );
        let b2 = b1.clone();
        let cellgov = b1.clone();
        let r = compare_multi(&[b1, b2], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Match);
        assert!(r.oracle_divergence.is_none());
    }

    #[test]
    fn multi_agreeing_oracles_cellgov_diverges() {
        let b1 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![0xAA])],
            vec![],
        );
        let b2 = b1.clone();
        let cellgov = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![0xBB])],
            vec![],
        );
        let r = compare_multi(&[b1, b2], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        assert!(r.oracle_divergence.is_none());
        let cg = r.cellgov_result.unwrap();
        assert_eq!(cg.memory_divergence.unwrap().expected, 0xAA);
    }

    #[test]
    fn multi_disagreeing_oracles_unsettled() {
        let b1 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![],
        );
        let b2 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![2])],
            vec![],
        );
        let cellgov = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![],
        );
        let r = compare_multi(&[b1, b2], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::UnsettledOracle);
        assert!(r.oracle_divergence.is_some());
        assert!(r.cellgov_result.is_none());
    }

    #[test]
    fn multi_three_baselines_third_disagrees() {
        let b1 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![5])],
            vec![],
        );
        let b2 = b1.clone();
        let b3 = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![9])],
            vec![],
        );
        let cellgov = b1.clone();
        let r = compare_multi(&[b1, b2, b3], &cellgov, CompareMode::Memory);
        assert_eq!(r.classification, Classification::UnsettledOracle);
    }

    #[test]
    fn multi_outcome_disagreement_is_unsettled() {
        let b1 = obs(ObservedOutcome::Completed, vec![], vec![]);
        let b2 = obs(ObservedOutcome::Fault, vec![], vec![]);
        let cellgov = obs(ObservedOutcome::Completed, vec![], vec![]);
        let r = compare_multi(&[b1, b2], &cellgov, CompareMode::Strict);
        assert_eq!(r.classification, Classification::UnsettledOracle);
    }

    #[test]
    #[should_panic(expected = "at least one baseline")]
    fn multi_no_baselines_panics() {
        let cellgov = obs(ObservedOutcome::Completed, vec![], vec![]);
        compare_multi(&[], &cellgov, CompareMode::Memory);
    }
}
