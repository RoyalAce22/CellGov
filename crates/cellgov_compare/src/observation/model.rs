//! Aggregate [`Observation`] and [`ObservationMetadata`] structs.
//! Constituent types live in sibling submodules ([`super::event`],
//! [`super::hashes`], [`super::memory`], [`super::outcome`]).

use serde::{Deserialize, Serialize};

use crate::observation::event::ObservedEvent;
use crate::observation::hashes::ObservedHashes;
use crate::observation::memory::NamedMemoryRegion;
use crate::observation::outcome::ObservedOutcome;

/// Per-run metadata recorded alongside an [`Observation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationMetadata {
    /// Runner identifier (e.g. `"cellgov"`, `"cellgov-boot"`,
    /// `"rpcs3-interpreter"`). Compared verbatim in cross-runner
    /// diffs, so do not include host-environment noise.
    pub runner: String,
    /// `None` when the runner does not expose a step count.
    pub steps: Option<usize>,
}

/// Normalized observation from a single test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// How the run terminated.
    pub outcome: ObservedOutcome,
    /// End-of-run snapshots of the regions declared in the manifest,
    /// in manifest order.
    pub memory_regions: Vec<NamedMemoryRegion>,
    /// Observable events emitted during the run, in dispatch order.
    pub events: Vec<ObservedEvent>,
    /// `None` for runners that do not expose internal state hashes
    /// (e.g., RPCS3).
    pub state_hashes: Option<ObservedHashes>,
    /// Per-run metadata: runner identity and optional step count.
    pub metadata: ObservationMetadata,
    /// `sys_tty_write` byte stream in dispatch order; empty when no
    /// TTY output was captured.
    #[serde(default)]
    pub tty_log: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::event::ObservedEventKind;
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
    fn observation_with_hashes_roundtrips() {
        let obs = sample_observation();
        let json = serde_json::to_string(&obs).expect("serialize");
        let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(obs, loaded);
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
    fn observation_without_tty_log_field_loads_with_empty_default() {
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
