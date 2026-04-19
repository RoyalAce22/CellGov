//! Normalized observation types for cross-runner comparison.
//!
//! Both runners (CellGov and RPCS3) produce an `Observation` with the
//! same shape. The comparison layer operates only on these types, never
//! on runner-specific internals. Each runner's adapter is responsible
//! for coalescing raw outputs into this schema.

use cellgov_trace::StateHash;
use serde::{Deserialize, Serialize};

/// How a test run terminated.
///
/// Both runners must map their native outcome into one of these
/// variants. The comparison layer treats outcome mismatch as a
/// divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObservedOutcome {
    /// Reached expected terminal state. All units finished or
    /// explicitly blocked with no pending wakes.
    Completed,
    /// No runnable units, but the run did not reach a clean terminal
    /// state (pending events remain, blocked receivers with no sender).
    Stalled,
    /// Max steps exceeded (CellGov) or wall-clock timeout (RPCS3).
    Timeout,
    /// Explicit runtime or architectural fault.
    Fault,
}

/// Semantic event kinds for normalized comparison.
///
/// Raw trace events from CellGov and instrumented output from RPCS3
/// are coalesced into these kinds by each runner's adapter. Timing
/// values are stripped during normalization -- only kind, unit, and
/// relative order survive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObservedEventKind {
    /// A unit sent a mailbox message.
    MailboxSend,
    /// A unit received a mailbox message.
    MailboxReceive,
    /// A DMA transfer completed.
    DmaComplete,
    /// A unit was woken from a blocked state.
    UnitWake,
    /// A unit blocked on a sync primitive.
    UnitBlock,
}

/// A single normalized event in the observation sequence.
///
/// Events are ordered by `sequence` (monotonic index within the
/// observation). This is not a guest tick -- CellGov and RPCS3 have
/// different time models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservedEvent {
    /// What happened.
    pub kind: ObservedEventKind,
    /// Which unit was involved.
    pub unit: u64,
    /// Monotonic index within the observation.
    pub sequence: u32,
}

/// A named memory region snapshot taken at end of run.
///
/// All observed memory regions must be test-owned and write-complete:
/// the test explicitly allocates the region, fully initializes it to a
/// known value (typically zero), and writes its result into it before
/// terminating. No comparison may depend on uninitialized or
/// partially-written memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedMemoryRegion {
    /// Region name from the manifest.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Raw bytes captured at end of run.
    pub data: Vec<u8>,
}

/// Serde bridge for `StateHash` without adding serde to `cellgov_trace`.
mod state_hash_serde {
    use cellgov_trace::StateHash;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(hash: &StateHash, s: S) -> Result<S::Ok, S::Error> {
        hash.raw().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<StateHash, D::Error> {
        u64::deserialize(d).map(StateHash::new)
    }
}

/// CellGov-side state hashes carried through from `ScenarioResult`.
///
/// These are CellGov-internal and used for replay comparison
/// (CellGov-vs-CellGov), not for cross-runner comparison. The RPCS3
/// adapter sets this to `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedHashes {
    /// Hash of committed guest memory.
    #[serde(with = "state_hash_serde")]
    pub memory: StateHash,
    /// Hash of all unit status values.
    #[serde(with = "state_hash_serde")]
    pub unit_status: StateHash,
    /// Hash of sync primitive state.
    #[serde(with = "state_hash_serde")]
    pub sync: StateHash,
}

/// Metadata about who produced this observation and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationMetadata {
    /// Which runner produced this observation.
    pub runner: String,
    /// Number of steps taken, or `None` when step counts are unavailable
    /// for the runner.
    pub steps: Option<usize>,
}

/// A complete normalized observation from a single test run.
///
/// Both runners produce this same shape. The comparison layer diffs
/// two observations field by field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// How the test ended.
    pub outcome: ObservedOutcome,
    /// Captured memory region snapshots.
    pub memory_regions: Vec<NamedMemoryRegion>,
    /// Normalized event sequence.
    pub events: Vec<ObservedEvent>,
    /// CellGov-side state hashes (None for RPCS3).
    pub state_hashes: Option<ObservedHashes>,
    /// Runner identity and run metadata.
    pub metadata: ObservationMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sample_observation;

    #[test]
    fn observations_with_same_fields_are_equal() {
        let a = sample_observation();
        let b = sample_observation();
        assert_eq!(a, b);
    }

    #[test]
    fn different_outcome_is_not_equal() {
        let a = sample_observation();
        let mut b = sample_observation();
        b.outcome = ObservedOutcome::Timeout;
        assert_ne!(a, b);
    }

    #[test]
    fn different_memory_region_data_is_not_equal() {
        let a = sample_observation();
        let mut b = sample_observation();
        b.memory_regions[0].data = vec![0, 0, 0, 2];
        assert_ne!(a, b);
    }

    #[test]
    fn different_event_sequence_is_not_equal() {
        let a = sample_observation();
        let mut b = sample_observation();
        b.events[1].kind = ObservedEventKind::UnitBlock;
        assert_ne!(a, b);
    }

    #[test]
    fn missing_hashes_differs_from_present() {
        let a = sample_observation();
        let mut b = sample_observation();
        b.state_hashes = None;
        assert_ne!(a, b);
    }

    #[test]
    fn rpcs3_style_observation_has_no_hashes_or_steps() {
        let obs = Observation {
            outcome: ObservedOutcome::Completed,
            memory_regions: vec![NamedMemoryRegion {
                name: "result".into(),
                addr: 0x10000,
                data: vec![0, 0, 0, 1],
            }],
            events: vec![],
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: "rpcs3".into(),
                steps: None,
            },
        };
        assert!(obs.state_hashes.is_none());
        assert!(obs.metadata.steps.is_none());
    }

    #[test]
    fn event_kind_variants_are_distinct() {
        let kinds = [
            ObservedEventKind::MailboxSend,
            ObservedEventKind::MailboxReceive,
            ObservedEventKind::DmaComplete,
            ObservedEventKind::UnitWake,
            ObservedEventKind::UnitBlock,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn outcome_variants_are_distinct() {
        let outcomes = [
            ObservedOutcome::Completed,
            ObservedOutcome::Stalled,
            ObservedOutcome::Timeout,
            ObservedOutcome::Fault,
        ];
        for (i, a) in outcomes.iter().enumerate() {
            for (j, b) in outcomes.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn observation_without_hashes_roundtrips() {
        let obs = Observation {
            outcome: ObservedOutcome::Completed,
            memory_regions: vec![],
            events: vec![],
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: "rpcs3".into(),
                steps: None,
            },
        };
        let json = serde_json::to_string(&obs).expect("serialize");
        let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(obs, loaded);
    }

    /// Pin the rule that `null` on a required non-Option field is a
    /// parse error, not a silent default. A regression here (e.g.,
    /// someone adding `#[serde(default)]` to `memory_regions`) would
    /// let malformed observations deserialize to empty vectors and
    /// the compare-observations harness would report a trivially
    /// vacuous MATCH. Explicit test so the property does not drift.
    #[test]
    fn null_on_required_field_rejects() {
        let json = r#"{
            "outcome": "Completed",
            "memory_regions": null,
            "events": [],
            "state_hashes": null,
            "metadata": { "runner": "rpcs3", "steps": null }
        }"#;
        assert!(
            serde_json::from_str::<Observation>(json).is_err(),
            "null memory_regions must fail to deserialize, not default to empty"
        );
    }
}
