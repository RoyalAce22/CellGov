//! Golden snapshot save/load for baseline regression testing.
//!
//! Observations can be serialized to JSON and saved to disk, then
//! loaded later for comparison without re-running the other runner.
//! This enables CellGov regression testing without RPCS3 installed
//! and fast CI iteration against saved oracle baselines.

use crate::observation::Observation;
use std::io;
use std::path::Path;

/// Why a baseline operation failed.
#[derive(Debug)]
pub enum BaselineError {
    /// File system error during save or load.
    Io(io::Error),
    /// JSON serialization or deserialization error.
    Json(serde_json::Error),
}

impl From<io::Error> for BaselineError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for BaselineError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Save an observation to a JSON file at `path`.
pub fn save(observation: &Observation, path: &Path) -> Result<(), BaselineError> {
    let json = serde_json::to_string_pretty(observation)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load an observation from a JSON file at `path`.
pub fn load(path: &Path) -> Result<Observation, BaselineError> {
    let data = std::fs::read_to_string(path)?;
    let obs = serde_json::from_str(&data)?;
    Ok(obs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{ObservationMetadata, ObservedOutcome};
    use crate::test_support::{sample_observation, TempDir};

    #[test]
    fn roundtrip_through_json() {
        let obs = sample_observation();
        let json = serde_json::to_string_pretty(&obs).expect("serialize");
        let loaded: Observation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(obs, loaded);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let obs = sample_observation();
        let dir = TempDir::new("baseline_test");
        let path = dir.file("test_baseline.json");

        save(&obs, &path).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(obs, loaded);
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = load(Path::new("/nonexistent/path/baseline.json"));
        assert!(result.is_err());
    }

    #[test]
    fn save_load_compare_pipeline() {
        // End-to-end: observe a scenario, save baseline, load it, compare.
        use crate::compare::{compare, Classification, CompareMode};
        use crate::runner_cellgov::{observe_with_determinism_check, RegionDescriptor};
        use cellgov_testkit::fixtures;

        let factory = || fixtures::mailbox_send_scenario(5);
        let regions: Vec<RegionDescriptor> = vec![];

        let obs = observe_with_determinism_check(factory, &regions).expect("observe");

        let dir = TempDir::new("pipeline_test");
        let path = dir.file("pipeline_baseline.json");

        save(&obs, &path).expect("save");
        let loaded = load(&path).expect("load");

        let result = compare(&loaded, &obs, CompareMode::Strict);
        assert_eq!(result.classification, Classification::Match);
    }

    #[test]
    fn multi_baseline_pipeline() {
        // Save two baselines from the same scenario, load both, run
        // compare_multi, verify oracle agreement and match.
        use crate::compare::{compare_multi, Classification, CompareMode};
        use crate::runner_cellgov::{observe_with_determinism_check, RegionDescriptor};
        use cellgov_testkit::fixtures;

        let factory = || fixtures::mailbox_roundtrip_scenario(0x42);
        let regions: Vec<RegionDescriptor> = vec![];

        let obs1 = observe_with_determinism_check(factory, &regions).expect("obs1");
        let obs2 = observe_with_determinism_check(factory, &regions).expect("obs2");

        let dir = TempDir::new("multi_pipeline_test");
        let p1 = dir.file("baseline_interp.json");
        let p2 = dir.file("baseline_llvm.json");

        save(&obs1, &p1).expect("save 1");
        save(&obs2, &p2).expect("save 2");

        let b1 = load(&p1).expect("load 1");
        let b2 = load(&p2).expect("load 2");

        // Both baselines come from the same deterministic scenario,
        // so oracles agree and CellGov matches.
        let cellgov = observe_with_determinism_check(factory, &regions).expect("cellgov");
        let result = compare_multi(&[b1, b2], &cellgov, CompareMode::Strict);
        assert_eq!(result.classification, Classification::Match);
        assert!(result.oracle_divergence.is_none());
    }

    #[test]
    fn rpcs3_tty_baseline_roundtrip() {
        // Parse a real RPCS3 TTY output, build an Observation, save
        // as JSON baseline, load it back, verify contents.
        use crate::runner_rpcs3::TtyRegion;

        let tty_path =
            std::path::Path::new("../../baselines/spu_fixed_value/rpcs3_interpreter.tty");
        if !tty_path.exists() {
            // Skip if the baseline file doesn't exist (CI without RPCS3).
            return;
        }

        let regions = vec![TtyRegion {
            name: "result".into(),
            size: 8,
            guest_addr: 0,
        }];
        let memory_regions =
            crate::runner_rpcs3::parse_tty_log(tty_path, &regions).expect("parse tty");
        assert_eq!(memory_regions.len(), 1);
        assert_eq!(memory_regions[0].name, "result");
        assert_eq!(memory_regions[0].data.len(), 8);
        // status=0 (4 bytes) + value=0 (4 bytes)
        assert_eq!(&memory_regions[0].data[..4], &[0, 0, 0, 0]);

        let obs = Observation {
            outcome: ObservedOutcome::Completed,
            memory_regions,
            events: vec![],
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: "rpcs3-interpreter".into(),
                steps: None,
            },
        };

        let dir = TempDir::new("rpcs3_baseline_test");
        let path = dir.file("rpcs3_interp.json");

        save(&obs, &path).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(obs, loaded);
    }

    #[test]
    fn compare_cellgov_vs_rpcs3_baseline() {
        // Load the RPCS3 baseline and compare against a CellGov
        // observation. The CellGov mailbox_send scenario does not
        // produce the same memory layout as the SPU fixed-value
        // test, so this should classify as Divergence on memory.
        use crate::compare::{compare, Classification, CompareMode};
        use crate::runner_cellgov::{observe_with_determinism_check, RegionDescriptor};
        use cellgov_testkit::fixtures;

        let baseline_path =
            std::path::Path::new("../../baselines/spu_fixed_value/rpcs3_interpreter.json");
        if !baseline_path.exists() {
            return;
        }
        let rpcs3_obs = load(baseline_path).expect("load baseline");

        // Run the CellGov scenario named in the manifest.
        let factory = || fixtures::mailbox_send_scenario(1);
        let regions: Vec<RegionDescriptor> = vec![];
        let cellgov_obs =
            observe_with_determinism_check(factory, &regions).expect("cellgov observe");

        // Both should complete, but memory regions will differ
        // (CellGov has none, RPCS3 has the result region).
        assert_eq!(rpcs3_obs.outcome, ObservedOutcome::Completed);
        assert_eq!(cellgov_obs.outcome, ObservedOutcome::Completed);

        let result = compare(&rpcs3_obs, &cellgov_obs, CompareMode::Memory);
        assert_eq!(result.classification, Classification::Divergence);
        assert!(result.memory_divergence.is_some());
        assert!(result.outcome_mismatch.is_none());
    }
}
