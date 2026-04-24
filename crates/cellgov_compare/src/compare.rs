//! Field-by-field diff between two `Observation` values, returning the first point of divergence.

use crate::observation::{NamedMemoryRegion, Observation, ObservedEvent, ObservedOutcome};

/// Which fields to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareMode {
    /// Outcome + memory + full event sequence.
    Strict,
    /// Outcome + memory; events ignored.
    Memory,
    /// Outcome + events; memory ignored.
    Events,
    /// Outcome + events up to the shorter sequence length.
    Prefix,
}

/// Overall classification of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Classification {
    /// All compared fields agree.
    Match,
    /// One or more compared fields differ.
    Divergence,
    /// CellGov has no matching scenario for this test.
    Unsupported,
    /// Baselines disagree with each other; CellGov result is inconclusive.
    UnsettledOracle,
}

/// First byte-level difference between two named memory regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDivergence {
    /// Region name from the manifest.
    pub region: String,
    /// Byte offset of the first differing byte.
    pub offset: usize,
    /// Byte in the expected observation.
    pub expected: u8,
    /// Byte in the actual observation.
    pub actual: u8,
}

/// First difference between two event sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDivergence {
    /// Index of the first differing event.
    pub index: usize,
    /// Event in the expected observation, if present.
    pub expected: Option<ObservedEvent>,
    /// Event in the actual observation, if present.
    pub actual: Option<ObservedEvent>,
}

/// Result of comparing CellGov against multiple baselines.
///
/// Unsettled when baselines disagree: if any two baselines differ under `mode`,
/// classification is `UnsettledOracle` regardless of CellGov, and
/// `cellgov_result` is `None`. Otherwise CellGov is compared against the first
/// baseline (all baselines are equivalent under `mode` when the oracle settles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiCompareResult {
    /// Overall classification.
    pub classification: Classification,
    /// Mode used for every sub-comparison.
    pub mode: CompareMode,
    /// First pairwise baseline disagreement, if oracles did not settle.
    pub oracle_divergence: Option<CompareResult>,
    /// CellGov vs. first baseline, if oracles settled.
    pub cellgov_result: Option<CompareResult>,
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

/// Structured result of comparing two observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareResult {
    /// Overall classification.
    pub classification: Classification,
    /// Mode used.
    pub mode: CompareMode,
    /// Set when outcomes differ.
    pub outcome_mismatch: Option<(ObservedOutcome, ObservedOutcome)>,
    /// First memory divergence, if any.
    pub memory_divergence: Option<MemoryDivergence>,
    /// First event divergence, if any.
    pub event_divergence: Option<EventDivergence>,
}

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

/// Regions match by name; a region in one side but not the other diverges at offset 0.
fn find_memory_divergence(
    expected: &[NamedMemoryRegion],
    actual: &[NamedMemoryRegion],
) -> Option<MemoryDivergence> {
    for exp in expected {
        let act = actual.iter().find(|r| r.name == exp.name);
        match act {
            None => {
                return Some(MemoryDivergence {
                    region: exp.name.clone(),
                    offset: 0,
                    expected: exp.data.first().copied().unwrap_or(0),
                    actual: 0,
                });
            }
            Some(act) => {
                let len = exp.data.len().max(act.data.len());
                for i in 0..len {
                    let e = exp.data.get(i).copied().unwrap_or(0);
                    let a = act.data.get(i).copied().unwrap_or(0);
                    if e != a {
                        return Some(MemoryDivergence {
                            region: exp.name.clone(),
                            offset: i,
                            expected: e,
                            actual: a,
                        });
                    }
                }
            }
        }
    }
    for act in actual {
        if !expected.iter().any(|r| r.name == act.name) {
            return Some(MemoryDivergence {
                region: act.name.clone(),
                offset: 0,
                expected: 0,
                actual: act.data.first().copied().unwrap_or(0),
            });
        }
    }
    None
}

/// In prefix mode, compares only up to the shorter sequence length.
fn find_event_divergence(
    expected: &[ObservedEvent],
    actual: &[ObservedEvent],
    prefix: bool,
) -> Option<EventDivergence> {
    let compare_len = if prefix {
        expected.len().min(actual.len())
    } else {
        expected.len().max(actual.len())
    };

    for i in 0..compare_len {
        let e = expected.get(i);
        let a = actual.get(i);
        match (e, a) {
            (Some(e), Some(a)) => {
                if e.kind != a.kind || e.unit != a.unit {
                    return Some(EventDivergence {
                        index: i,
                        expected: Some(*e),
                        actual: Some(*a),
                    });
                }
            }
            (Some(e), None) => {
                return Some(EventDivergence {
                    index: i,
                    expected: Some(*e),
                    actual: None,
                });
            }
            (None, Some(a)) => {
                return Some(EventDivergence {
                    index: i,
                    expected: None,
                    actual: Some(*a),
                });
            }
            (None, None) => break,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::ObservedEventKind;
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
    fn memory_divergence_reports_first_differing_byte() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1, 2, 3])],
            vec![],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1, 2, 99])],
            vec![],
        );
        let r = compare(&a, &b, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.memory_divergence.unwrap();
        assert_eq!(d.region, "r");
        assert_eq!(d.offset, 2);
        assert_eq!(d.expected, 3);
        assert_eq!(d.actual, 99);
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
    fn event_divergence_reports_first_differing_event() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![
                event(ObservedEventKind::MailboxSend, 0, 0),
                event(ObservedEventKind::UnitWake, 1, 1),
            ],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![
                event(ObservedEventKind::MailboxSend, 0, 0),
                event(ObservedEventKind::UnitBlock, 1, 1),
            ],
        );
        let r = compare(&a, &b, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.event_divergence.unwrap();
        assert_eq!(d.index, 1);
        assert_eq!(d.expected.unwrap().kind, ObservedEventKind::UnitWake);
        assert_eq!(d.actual.unwrap().kind, ObservedEventKind::UnitBlock);
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
    fn prefix_mode_matches_when_shorter_prefix_agrees() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![
                event(ObservedEventKind::MailboxSend, 0, 0),
                event(ObservedEventKind::UnitWake, 1, 1),
            ],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![
                event(ObservedEventKind::MailboxSend, 0, 0),
                event(ObservedEventKind::UnitWake, 1, 1),
                event(ObservedEventKind::MailboxReceive, 1, 2),
            ],
        );
        let r = compare(&a, &b, CompareMode::Prefix);
        assert_eq!(r.classification, Classification::Match);
    }

    #[test]
    fn prefix_mode_diverges_when_prefix_differs() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![event(ObservedEventKind::UnitBlock, 0, 0)],
        );
        let r = compare(&a, &b, CompareMode::Prefix);
        assert_eq!(r.classification, Classification::Divergence);
    }

    #[test]
    fn strict_mode_diverges_on_different_event_lengths() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![
                event(ObservedEventKind::MailboxSend, 0, 0),
                event(ObservedEventKind::UnitWake, 1, 1),
            ],
        );
        let r = compare(&a, &b, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.event_divergence.unwrap();
        assert_eq!(d.index, 1);
        assert!(d.expected.is_none());
        assert!(d.actual.is_some());
    }

    #[test]
    fn missing_memory_region_is_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1])],
            vec![],
        );
        let b = obs(ObservedOutcome::Completed, vec![], vec![]);
        let r = compare(&a, &b, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.memory_divergence.unwrap();
        assert_eq!(d.region, "r");
    }

    #[test]
    fn extra_memory_region_in_actual_is_divergence() {
        let a = obs(ObservedOutcome::Completed, vec![], vec![]);
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("extra", vec![1])],
            vec![],
        );
        let r = compare(&a, &b, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.memory_divergence.unwrap();
        assert_eq!(d.region, "extra");
    }

    #[test]
    fn different_length_memory_regions_diverge() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1, 2])],
            vec![],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", vec![1, 2, 3])],
            vec![],
        );
        let r = compare(&a, &b, CompareMode::Memory);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.memory_divergence.unwrap();
        assert_eq!(d.offset, 2);
        assert_eq!(d.expected, 0);
        assert_eq!(d.actual, 3);
    }

    #[test]
    fn event_unit_mismatch_is_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![event(ObservedEventKind::MailboxSend, 0, 0)],
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![],
            vec![event(ObservedEventKind::MailboxSend, 1, 0)],
        );
        let r = compare(&a, &b, CompareMode::Events);
        assert_eq!(r.classification, Classification::Divergence);
        let d = r.event_divergence.unwrap();
        assert_eq!(d.index, 0);
        assert_eq!(d.expected.unwrap().unit, 0);
        assert_eq!(d.actual.unwrap().unit, 1);
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
