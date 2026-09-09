//! Fixtures shared by more than one of the `bench` submodule test
//! files.

use std::time::Duration;

use cellgov_compare::witness_parse::{parse_witness_lines, ParsedWitnesses};
use cellgov_compare::{BootOutcome, BootSummary, RunIdentity};
use cellgov_time::Budget;

use super::anchor::MeasuredRun;
use super::options::{AnchorPlan, BenchOptions, SelectionArgs};
use super::throughput::ThroughputPolicy;
use super::types::BenchBootResult;
use crate::game::manifest::{self, CellKey};

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

pub(super) fn reporting() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: false,
    }
}

pub(super) fn strict() -> ThroughputPolicy {
    ThroughputPolicy {
        runs: 3,
        strict: true,
    }
}

/// The stop condition [`anchor_fixture`] records.
pub(super) const TEST_CHECKPOINT: manifest::CheckpointTrigger =
    manifest::CheckpointTrigger::ProcessExit;

/// A run that reproduces [`anchor_fixture`] exactly, as the set hands
/// it to the anchor check.
pub(super) fn measured_run(stderr: &str) -> MeasuredRun<'_> {
    MeasuredRun {
        checkpoint: TEST_CHECKPOINT,
        steps: 1,
        budget: Budget::new(256),
        outcome: "MaxSteps".to_string(),
        stderr,
    }
}

/// The cell [`anchor_fixture`] is filed under.
pub(super) fn test_cell() -> CellKey {
    CellKey {
        fw: "4.93".to_string(),
        game_ver: Some("base".to_string()),
    }
}

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

pub(super) fn observed_stderr(breaks: u64, ldarx: u64) -> ParsedWitnesses {
    parse_witness_lines(&format!(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={breaks}\n\
         BENCH_ATOMIC_WITNESS: ldarx={ldarx} stdcx=0 lwarx=0 stwcx=0\n"
    ))
    .expect("synthetic witness lines parse")
}

pub(super) fn bench_manifest(bench_max_steps: Option<u64>) -> crate::game::manifest::TitleManifest {
    use crate::game::manifest::{Distribution, GameSource};
    crate::game::manifest::TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        system_ver: Some("4.93".to_string()),
        checkpoint: manifest::CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

/// A run of `title` in `cell`, at exactly what the registry declares
/// for it.
pub(super) fn bench_options<'a>(
    title: &'a crate::game::manifest::TitleManifest,
    cell: Option<&'a CellKey>,
    guest_args: &'a [String],
) -> BenchOptions<'a> {
    let max_steps = crate::paths::cell_max_steps(title, None);
    BenchOptions {
        title,
        elf_path: "EBOOT.BIN",
        max_steps: max_steps as usize,
        plan: AnchorPlan {
            cell,
            max_steps,
            checkpoint: title.checkpoint_trigger(),
        },
        firmware_dir: None,
        composed_mounts: &[],
        identity: &cellgov_compare::RunIdentity {
            firmware: None,
            game: None,
        },
        selection: SelectionArgs::default(),
        strict_reserved: false,
        checkpoint_override: None,
        budget_override: None,
        prescan: false,
        guest_args,
        check_anchor: true,
        run_index: 0,
    }
}
