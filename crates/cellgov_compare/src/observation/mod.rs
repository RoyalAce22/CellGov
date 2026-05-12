//! Normalized observation types for cross-runner comparison. The
//! aggregate [`Observation`] / [`ObservationMetadata`] live here; the
//! constituent types are decomposed across submodules by concern.

mod event;
mod hashes;
mod memory;
mod outcome;

pub use event::{ObservedEvent, ObservedEventKind};
pub use hashes::ObservedHashes;
pub use memory::NamedMemoryRegion;
pub use outcome::ObservedOutcome;

use serde::{Deserialize, Serialize};

/// Metadata about who produced this observation and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationMetadata {
    /// Which runner produced this observation.
    pub runner: String,
    /// Number of steps taken, or `None` when unavailable for the runner.
    pub steps: Option<usize>,
}

/// A complete normalized observation from a single test run.
///
/// Downstream consumers (baseline storage, the compare layer, divergence
/// reports) depend on field stability and the serde schema. Adding or
/// renaming fields is a breaking change for stored baselines; prefer
/// `#[serde(default)]` only for fields where a missing value has a
/// defined meaning, and never for required slices (see the
/// `null_on_required_field_rejects` test).
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
    /// `sys_tty_write` byte stream captured in dispatch order. Empty
    /// when the runner did not capture TTY output, or when the guest
    /// emitted none. `#[serde(default)]` lets pre-Step-1 baselines
    /// load with an empty default.
    #[serde(default)]
    pub tty_log: Vec<u8>,
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
            tty_log: Vec::new(),
        };
        assert!(obs.state_hashes.is_none());
        assert!(obs.metadata.steps.is_none());
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
            tty_log: Vec::new(),
        };
        let json = serde_json::to_string(&obs).expect("serialize");
        let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(obs, loaded);
    }

    #[test]
    fn pre_step1_baseline_without_tty_log_field_loads_with_empty_default() {
        // Baselines saved before TTY capture existed serialize without
        // a tty_log field. `#[serde(default)]` must let them load.
        let json = r#"{
            "outcome": "Completed",
            "memory_regions": [],
            "events": [],
            "state_hashes": null,
            "metadata": { "runner": "rpcs3-interpreter", "steps": null }
        }"#;
        let obs: Observation = serde_json::from_str(json).expect("legacy baseline must load");
        assert!(obs.tty_log.is_empty());
    }

    #[test]
    fn tty_log_difference_breaks_observation_equality() {
        let a = sample_observation();
        let mut b = sample_observation();
        b.tty_log.push(b'!');
        assert_ne!(a, b);
    }

    // Guards against `#[serde(default)]` creeping onto required slice
    // fields: a `null` memory_regions would silently become an empty
    // vector and compare to MATCH vacuously.
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
