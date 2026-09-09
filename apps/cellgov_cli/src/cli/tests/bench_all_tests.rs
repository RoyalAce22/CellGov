use super::*;

use crate::game::manifest::{CellKey, CheckpointTrigger};
use crate::game::{AnchorVerdict, BenchGate, BenchRunsOutcome, ThroughputVerdict};

fn cell(short_name: &str, fw: &str, game_ver: Option<&str>) -> DeclaredCell {
    DeclaredCell {
        short_name: short_name.to_string(),
        content_id: "CG_TEST".to_string(),
        cell: CellKey {
            fw: fw.to_string(),
            game_ver: game_ver.map(str::to_string),
        },
        max_steps: 4_000,
        checkpoint: CheckpointTrigger::ProcessExit,
        pending: None,
    }
}

fn outcome(
    gate: BenchGate,
    anchor: AnchorVerdict,
    determinism_failures: Vec<String>,
) -> BenchRunsOutcome {
    BenchRunsOutcome {
        runs: Vec::new(),
        throughput: ThroughputVerdict::Unmeasurable,
        gate,
        anchor,
        determinism_failures,
    }
}

#[test]
fn a_sweep_with_every_cell_matching_or_set_aside_exits_zero() {
    let verdicts = [
        CellVerdict::Matches,
        CellVerdict::NotInstalled("--fw \"1.50\" is not installed".to_string()),
        CellVerdict::Pending("the firmware cannot be obtained".to_string()),
        CellVerdict::NotCompared(vec!["--max-steps 10 differs".to_string()]),
        CellVerdict::NotChecked,
    ];
    assert_eq!(sweep_exit_code(&verdicts), 0);
    assert_eq!(sweep_exit_code(&[]), 0);
}

#[test]
fn the_worst_cell_decides_the_status() {
    let moved = CellVerdict::Moved(vec!["steps 10 != recorded 11".to_string()]);
    let broke = CellVerdict::DeterminismBreak(2);
    let failed = CellVerdict::BootFailed("subprocess exited nonzero".to_string());
    assert_eq!(
        sweep_exit_code(&[CellVerdict::Matches, CellVerdict::NotRecorded]),
        exit_codes::FAILED
    );
    assert_eq!(
        sweep_exit_code(&[CellVerdict::NotRecorded, CellVerdict::NoThroughputVerdict]),
        EXIT_SPREAD_EXCEEDED
    );
    assert_eq!(
        sweep_exit_code(&[CellVerdict::NoThroughputVerdict, failed.clone()]),
        exit_codes::DIVERGED
    );
    assert_eq!(
        sweep_exit_code(&[failed.clone(), moved.clone(), CellVerdict::Matches]),
        exit_codes::ANCHOR_MOVED,
        "a moved anchor outranks a boot that failed elsewhere"
    );
    assert_eq!(
        sweep_exit_code(&[moved, broke, failed]),
        exit_codes::DISAGREED,
        "a determinism break outranks everything"
    );
}

#[test]
fn a_passing_set_is_classified_by_its_anchor_verdict() {
    assert_eq!(
        classify(outcome(BenchGate::Pass, AnchorVerdict::Match, Vec::new())),
        CellVerdict::Matches
    );
    assert_eq!(
        classify(outcome(
            BenchGate::Pass,
            AnchorVerdict::NotRecorded("fw 4.93 x base".to_string()),
            Vec::new()
        )),
        CellVerdict::NotRecorded,
        "an unrecorded cell passes the single gate and is a finding here"
    );
    assert_eq!(
        classify(outcome(
            BenchGate::Pass,
            AnchorVerdict::NotComparable(vec!["retargeted".to_string()]),
            Vec::new()
        )),
        CellVerdict::NotCompared(vec!["retargeted".to_string()])
    );
    assert_eq!(
        classify(outcome(BenchGate::Pass, AnchorVerdict::Skipped, Vec::new())),
        CellVerdict::NotChecked
    );
}

#[test]
fn a_failing_set_is_classified_by_its_gate() {
    let failures = vec!["steps 10 != recorded 11".to_string()];
    assert_eq!(
        classify(outcome(
            BenchGate::AnchorDrift,
            AnchorVerdict::Drift(failures.clone()),
            Vec::new()
        )),
        CellVerdict::Moved(failures)
    );
    assert_eq!(
        classify(outcome(
            BenchGate::DeterminismBreak,
            AnchorVerdict::Match,
            vec!["a".to_string(), "b".to_string()]
        )),
        CellVerdict::DeterminismBreak(2)
    );
    assert_eq!(
        classify(outcome(
            BenchGate::SpreadExceeded,
            AnchorVerdict::Match,
            Vec::new()
        )),
        CellVerdict::NoThroughputVerdict
    );
}

#[test]
fn an_absent_store_half_is_not_installed_and_a_broken_one_is_not() {
    let absent = [
        ComposeError::Firmware(FirmwareSelectError::NotInstalled {
            asked: "1.50".to_string(),
            root: "store".to_string(),
            installed: vec!["4.93".to_string()],
        }),
        ComposeError::Firmware(FirmwareSelectError::NoneInstalled {
            root: "store".to_string(),
            disable_env: "X",
        }),
        ComposeError::GameVersion(GameVersionSelectError::NotInstalled {
            asked: "01.02".to_string(),
            title_id: "CG_TEST".to_string(),
            installed: vec!["base".to_string()],
        }),
        ComposeError::GameVersion(GameVersionSelectError::OrphanUpdates {
            title_id: "CG_TEST".to_string(),
            updates: vec!["01.02".to_string()],
        }),
        ComposeError::TitleNotInStore {
            title_id: "CG_TEST".to_string(),
            root: "store".to_string(),
        },
    ];
    for e in &absent {
        assert_eq!(
            not_installed_reason(e).as_deref(),
            Some(e.to_string().as_str())
        );
    }
    let broken = [
        ComposeError::Firmware(FirmwareSelectError::TreeMissing {
            version: "4.93".to_string(),
            root: "store".to_string(),
            dir: "store/firmware/4.93".to_string(),
        }),
        ComposeError::TreeMissing {
            title_id: "CG_TEST".to_string(),
            version: "base".to_string(),
            dir: "store/titles/CG_TEST".to_string(),
        },
        ComposeError::FirmwareRelativeWithoutEntry {
            short_name: "shipped".to_string(),
            dir: "dev_flash/shipped/module".to_string(),
        },
    ];
    for e in &broken {
        assert_eq!(not_installed_reason(e), None, "{e}");
    }
}

#[test]
fn a_summary_line_names_the_cell_and_details_a_movement() {
    let c = cell("synthetic", "4.93", Some("base"));
    assert_eq!(
        summary_lines(&c, &CellVerdict::Matches),
        vec!["synthetic fw 4.93 x base: matches"]
    );
    let moved = CellVerdict::Moved(vec![
        "steps 10 != recorded 11".to_string(),
        "witness x moved".to_string(),
    ]);
    assert_eq!(
        summary_lines(&c, &moved),
        vec![
            "synthetic fw 4.93 x base: moved (2 disagreement(s))",
            "    steps 10 != recorded 11",
            "    witness x moved",
        ]
    );
    assert_eq!(
        summary_lines(&c, &CellVerdict::NotRecorded),
        vec![
            "synthetic fw 4.93 x base: not recorded -- the registry declares the cell and nothing \
             gates it; record it with `dev record-anchors --title synthetic`"
        ]
    );
    let shipped = cell("shipped", "2.76", None);
    assert_eq!(
        summary_lines(&shipped, &CellVerdict::Pending("no module".to_string())),
        vec!["shipped fw 2.76: pending (no module)"]
    );
}

#[test]
fn a_multi_line_not_installed_reason_indents_its_continuation_lines() {
    let c = cell("synthetic", "4.93", Some("base"));
    let one = CellVerdict::NotInstalled("--fw \"1.50\" is not installed".to_string());
    assert_eq!(
        summary_lines(&c, &one),
        vec!["synthetic fw 4.93 x base: not installed (--fw \"1.50\" is not installed)"]
    );
    let many = CellVerdict::NotInstalled(
        "every eboot_candidate for title synthetic failed under USRDIR:\n    EBOOT.BIN: read failed\n    EBOOT.elf: read failed"
            .to_string(),
    );
    assert_eq!(
        summary_lines(&c, &many),
        vec![
            "synthetic fw 4.93 x base: not installed (every eboot_candidate for title synthetic failed under USRDIR:",
            "    EBOOT.BIN: read failed",
            "    EBOOT.elf: read failed)",
        ]
    );
}

#[test]
fn the_tally_counts_each_kind_once_in_first_seen_order() {
    let verdicts = [
        CellVerdict::Matches,
        CellVerdict::NotInstalled("x".to_string()),
        CellVerdict::Matches,
        CellVerdict::Pending("y".to_string()),
        CellVerdict::Moved(Vec::new()),
    ];
    assert_eq!(
        tally_line(&verdicts),
        "boot bench --all: 5 declared cell(s): 2 matched, 1 not installed, 1 pending, 1 moved"
    );
}

#[test]
fn the_header_counts_the_titles_the_kept_cells_span() {
    let mut other = cell("sibling", "3.70", Some("base"));
    other.content_id = "CG_OTHER".to_string();
    assert_eq!(titles_spanned(&[]), 0);
    assert_eq!(
        titles_spanned(&[
            cell("synthetic", "4.93", Some("base")),
            cell("synthetic", "1.50", Some("base"))
        ]),
        1,
        "two cells of one title are one title"
    );
    assert_eq!(
        titles_spanned(&[cell("synthetic", "4.93", Some("base")), other]),
        2
    );
}

#[test]
fn the_nothing_ran_refusal_names_pending_beside_not_installed() {
    let verdicts = [
        CellVerdict::NotInstalled("x".to_string()),
        CellVerdict::Pending("y".to_string()),
        CellVerdict::Pending("z".to_string()),
    ];
    assert_eq!(
        nothing_ran_line(&verdicts),
        "boot bench --all: none of the 3 declared cell(s) ran (1 not installed, 2 pending); \
         nothing gated"
    );
}

#[test]
fn only_a_measured_cell_counts_as_run() {
    assert!(CellVerdict::Matches.ran());
    assert!(CellVerdict::NotRecorded.ran());
    assert!(CellVerdict::BootFailed("x".to_string()).ran());
    assert!(!CellVerdict::NotInstalled("x".to_string()).ran());
    assert!(!CellVerdict::Pending("x".to_string()).ran());
}
