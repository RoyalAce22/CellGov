//! Fixtures for the `bench` test files: a run set, and a committed
//! anchor with the identity triple it embeds.

use std::time::Duration;

use cellgov_time::Budget;

use crate::boot_summary::{BootSummary, CheckpointKind};
use crate::identity::RunIdentity;
use crate::runner_cellgov::BootOutcome;

use super::result_line::BenchBootResult;

/// A run of `wall` that agrees with every other run this helper
/// builds, so a case that varies only the wall isolates the throughput
/// half.
pub(super) fn run_of(run_index: usize, wall: Duration) -> BenchBootResult {
    BenchBootResult {
        run_index,
        steps: 10,
        wall,
        budget: Budget::new(256),
        outcome: BootOutcome::ProcessExit,
    }
}

/// A set whose walls are `walls`, indexed in order.
pub(super) fn set_of(walls: &[Duration]) -> Vec<BenchBootResult> {
    walls
        .iter()
        .enumerate()
        .map(|(i, w)| run_of(i, *w))
        .collect()
}

/// The stop condition [`anchor_fixture`] records.
pub(super) const TEST_CHECKPOINT: CheckpointKind = CheckpointKind::ProcessExit;

/// The identity triple [`anchor_fixture`] embeds.
pub(super) fn test_identity() -> RunIdentity {
    anchor_fixture(0).identity
}

/// Mirrors a committed `boot_summary.json`, so the fixture format and
/// the comparison are exercised through the deserializer production
/// uses.
pub(super) fn anchor_fixture(breaks: u64) -> BootSummary {
    serde_json::from_str(&format!(
        r#"{{
          "checkpoint": {{ "kind": "process_exit" }},
          "outcome": "MaxSteps",
          "steps": 390099,
          "budget": 256,
          "host_invariant_breaks": {breaks},
          "witnesses": {{
            "host_invariant_breaks": {{ "value": {breaks}, "class": "exact" }},
            "ldarx": {{ "value": 100, "class": "at-least" }},
            "stdcx": {{ "value": 0, "class": "at-least" }},
            "lwarx": {{ "value": 0, "class": "at-least" }},
            "stwcx": {{ "value": 0, "class": "at-least" }}
          }},
          "firmware": {{
            "version": "4.93",
            "image_version": "0x0000000000010b94",
            "pup_sha256": "00"
          }},
          "game": {{
            "title_id": "CG_TEST",
            "version": "base",
            "app_ver": "01.00"
          }}
        }}"#
    ))
    .expect("anchor fixture parses")
}
