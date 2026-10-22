//! The data-free halves of the data-feature suites: the ps3autotests
//! case table, manifest and verdict, and the title walks of the
//! installed-title suites, each driven with built inputs.
//!
//! The suites that read the external data stay behind their features;
//! these checks run on a fresh clone.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use std::path::Path;

use cellgov_compare::{Observation, ObservationMetadata, ObservedOutcome};
use cellgov_ps3_abi::format::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID;

#[path = "common/ps3autotests_cases.rs"]
mod cases;
use cases::{
    manifest_content, report_verdict, CASES, CPU_BASIC, CPU_PPU_BRANCH, LV2_SYS_SEMAPHORE,
};

#[path = "common/registry.rs"]
mod registry;
use registry::{ReferenceCell, TitleUnderTest};

#[path = "common/title_suite_checks.rs"]
mod title_suite_checks;
use title_suite_checks::{by_cost_from, check_titles, AuthorityWitness};

#[test]
fn every_generated_boot_manifest_parses_and_names_its_system_ver() {
    for case in CASES {
        let manifest = cellgov_boot::manifest::TitleManifest::load_from_text(
            &manifest_content(case, "4.93"),
            Path::new("ps3autotests-generated.toml"),
        )
        .unwrap_or_else(|error| {
            panic!(
                "ps3autotests {}/{} generated an invalid manifest: {error}",
                case.rel_dir, case.stem
            )
        });
        assert_eq!(manifest.system_ver.as_deref(), Some("4.93"));
    }
}

/// A case named twice would boot the same ELF under two test names and
/// read as broader coverage than the table holds.
#[test]
fn the_case_table_names_each_fixture_once() {
    let mut seen: std::collections::BTreeSet<(&str, &str)> = std::collections::BTreeSet::new();
    for case in CASES {
        assert!(
            seen.insert((case.rel_dir, case.stem)),
            "duplicate case {}/{}",
            case.rel_dir,
            case.stem
        );
    }
    assert_eq!(seen.len(), 5);
}

/// [`report_verdict`] reads only the outcome, the step count, and
/// `tty_log`; every other field stays empty.
fn observation_with(outcome: ObservedOutcome, steps: usize) -> Observation {
    Observation {
        outcome,
        memory_regions: Vec::new(),
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov-boot".into(),
            steps: Some(steps),
        },
        tty_log: Vec::new(),
        identity: cellgov_compare::RunIdentity::default(),
        runner_firmware: None,
    }
}

#[test]
#[should_panic(expected = "outcome=Fault")]
fn a_faulted_run_is_named_as_a_fault_not_compared_against_expected() {
    report_verdict(
        &CPU_BASIC,
        &observation_with(ObservedOutcome::Fault, 40),
        b"expected output",
    );
}

#[test]
#[should_panic(expected = "outcome=Timeout")]
fn a_capped_run_is_named_as_a_timeout_not_compared_against_expected() {
    report_verdict(
        &CPU_PPU_BRANCH,
        &observation_with(ObservedOutcome::Timeout, 195_312),
        b"expected output",
    );
}

#[test]
#[should_panic(expected = "outcome=Stalled")]
fn a_stalled_run_is_named_as_a_stall_not_compared_against_expected() {
    report_verdict(
        &LV2_SYS_SEMAPHORE,
        &observation_with(ObservedOutcome::Stalled, 500),
        b"expected output",
    );
}

#[test]
fn a_clean_exit_whose_tty_matches_reports_a_match() {
    let mut observation = observation_with(ObservedOutcome::ProcessExit, 83);
    observation.tty_log = b"hello\n".to_vec();
    report_verdict(&CPU_BASIC, &observation, b"hello\n");
}

/// Negative control for the match above: without it, a byte compare
/// that always succeeded would leave every test here green.
#[test]
#[should_panic(expected = "TTY divergence")]
fn a_clean_exit_whose_tty_differs_is_a_divergence() {
    let mut observation = observation_with(ObservedOutcome::ProcessExit, 83);
    observation.tty_log = b"hello\n".to_vec();
    report_verdict(&CPU_BASIC, &observation, b"goodbye\n");
}

fn title(short_name: &str, pending: Option<&str>) -> TitleUnderTest {
    TitleUnderTest {
        short_name: short_name.to_string(),
        content_id: format!("TEST-{short_name}"),
        max_steps: 1,
        system_ver: Some("1.50".to_string()),
        reference: ReferenceCell {
            fw: "1.50".to_string(),
            game_ver: Some("base".to_string()),
            pending: pending.map(str::to_string),
        },
    }
}

fn witness(force_system_authid: bool) -> AuthorityWitness {
    AuthorityWitness {
        program_authority_id: if force_system_authid {
            BDJ_SELF_PROGRAM_AUTHORITY_ID
        } else {
            1
        },
        authid_source: "self".to_string(),
        lwmutex_unknown_locks: u64::from(force_system_authid),
    }
}

#[test]
fn pending_cell_is_set_aside_while_runnable_sibling_executes() {
    let titles = [
        title("pending", Some("known boot refusal")),
        title("runnable", None),
    ];
    let mut calls = Vec::new();

    check_titles(&titles, |title, forced| {
        calls.push((title.short_name.clone(), forced));
        Some(witness(forced))
    });

    assert_eq!(
        calls,
        [
            ("runnable".to_string(), false),
            ("runnable".to_string(), true)
        ]
    );
}

#[test]
#[should_panic(
    expected = "installed-title-tests is enabled but none of the 2 registered title(s) is installed"
)]
fn only_pending_or_uninstalled_cells_fail_the_anti_vacuity_floor() {
    let titles = [
        title("pending", Some("known boot refusal")),
        title("uninstalled", None),
    ];

    check_titles(&titles, |_, _| None);
}

#[test]
fn pending_cells_are_set_aside_before_their_anchors_are_read() {
    let titles = vec![
        title("pending", Some("no anchor by contract")),
        title("later", None),
        title("earlier", None),
    ];
    let mut read = Vec::new();

    let sorted = by_cost_from(titles, |title| {
        read.push(title.short_name.clone());
        if title.short_name == "earlier" {
            10
        } else {
            20
        }
    });

    assert_eq!(read, ["later", "earlier"]);
    assert_eq!(
        sorted
            .iter()
            .map(|(_, title)| title.short_name.as_str())
            .collect::<Vec<_>>(),
        ["earlier", "later"]
    );
}
