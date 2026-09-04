//! What the headline row does with a summary whose firmware disagrees
//! with the other runner, or with the cell it sits in.

use super::*;
use cellgov_compare::{
    BootOutcome, BootSummary, ByteParity, CheckpointKind, Convergence, CrossRunnerSummary,
    FirmwareIdentity, GameIdentity, RunIdentity,
};
use cellgov_time::Budget;
use std::collections::BTreeMap;

use crate::game::manifest::{
    CellExpectation, CellKey, CheckpointTrigger, Distribution, GameSource, MatrixCell,
};

/// The firmware every title here declares its reference cell at.
const REFERENCE_FW: &str = "4.93";

struct Fixtures(cellgov_testkit::scratch::ScratchDir);

impl Fixtures {
    fn new(name: &str) -> Self {
        Self(cellgov_testkit::scratch::scratch_labeled(name))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, content_id: &str, fw: &str, summary: &CrossRunnerSummary) {
        let path =
            crate::paths::cross_runner_summary_path_in(self.path(), content_id, &cell_key(fw));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(summary).unwrap()).unwrap();
    }
}

fn cell_key(fw: &str) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: Some("base".to_string()),
    }
}

fn title(content_id: &str) -> TitleManifest {
    TitleManifest {
        content_id: content_id.to_string(),
        short_name: content_id.to_lowercase(),
        display_name: "Firmware Fixture".to_string(),
        eboot_candidates: vec!["EBOOT.elf".to_string()],
        year: 2008,
        developer: "Studio".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: vec![MatrixCell {
            key: cell_key(REFERENCE_FW),
            reference: true,
            expect: CellExpectation::Frontier,
            bench_max_steps: None,
            checkpoint: None,
            pending: None,
        }],
    }
}

fn summary(cellgov_fw: Option<&str>, rpcs3_fw: Option<&str>) -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Equivalent,
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
        identity: RunIdentity {
            firmware: cellgov_fw.map(|v| FirmwareIdentity {
                version: v.to_string(),
                image_version: "0x1".to_string(),
                pup_sha256: "ab".to_string(),
            }),
            game: Some(GameIdentity {
                title_id: "NPAA60001".to_string(),
                version: "base".to_string(),
                app_ver: "01.00".to_string(),
            }),
        },
        rpcs3_firmware: rpcs3_fw.map(str::to_string),
    }
}

#[test]
fn one_library_on_both_sides_renders_the_verdict() {
    let fixtures = Fixtures::new("fw-matched");
    fixtures.write(
        "NPAA60001",
        REFERENCE_FW,
        &summary(Some(REFERENCE_FW), Some(REFERENCE_FW)),
    );
    let row = render_row(&title("NPAA60001"), fixtures.path()).unwrap();
    assert!(row.contains("| Yes |"), "{row}");
    assert!(row.contains("| equivalent |"), "{row}");
}

#[test]
fn two_libraries_refuse_the_row_naming_both() {
    let fixtures = Fixtures::new("fw-crossed");
    fixtures.write(
        "NPAA60002",
        REFERENCE_FW,
        &summary(Some("4.93"), Some("4.92")),
    );
    match render_row(&title("NPAA60002"), fixtures.path()) {
        Err(SummaryLoadError::Parse { path, err }) => {
            assert!(path.ends_with("cross_runner_summary.json"), "{path:?}");
            let rendered = err.to_string();
            assert!(
                rendered.contains("4.93") && rendered.contains("4.92"),
                "{rendered}"
            );
        }
        other => panic!("expected a typed parse error, got {other:?}"),
    }
}

#[test]
fn a_stamped_summary_missing_the_other_runners_version_refuses_the_row() {
    let fixtures = Fixtures::new("fw-halfstamped");
    fixtures.write(
        "NPAA60003",
        REFERENCE_FW,
        &summary(Some(REFERENCE_FW), None),
    );
    assert!(matches!(
        render_row(&title("NPAA60003"), fixtures.path()),
        Err(SummaryLoadError::Parse { .. })
    ));
}

#[test]
fn another_cells_summary_does_not_answer_for_this_one() {
    let fixtures = Fixtures::new("fw-othercell");
    fixtures.write("NPAA60004", "3.55", &summary(Some("3.55"), Some("3.55")));
    let row = render_row(&title("NPAA60004"), fixtures.path()).unwrap();
    assert!(row.ends_with("| -- | -- |"), "{row}");
}

#[test]
fn a_summary_measured_elsewhere_does_not_answer_for_the_cell_it_sits_in() {
    let fixtures = Fixtures::new("fw-misfiled");
    fixtures.write(
        "NPAA60005",
        REFERENCE_FW,
        &summary(Some("2.76"), Some("2.76")),
    );
    match render_row(&title("NPAA60005"), fixtures.path()) {
        Err(SummaryLoadError::CellFirmwareMismatch {
            path,
            cell,
            recorded,
        }) => {
            assert!(path.ends_with("cross_runner_summary.json"), "{path:?}");
            assert_eq!(cell, REFERENCE_FW);
            assert_eq!(recorded, "2.76");
        }
        other => panic!("expected a mis-filed refusal, got {other:?}"),
    }
}

#[test]
fn an_anchor_measured_elsewhere_does_not_answer_for_the_cell_it_sits_in() {
    let fixtures = Fixtures::new("fw-misfiled-anchor");
    let mut anchor = BootSummary::new(
        CheckpointKind::ProcessExit,
        BootOutcome::ProcessExit,
        11_212,
        Budget::new(256),
    )
    .unwrap();
    anchor.identity = RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: "2.76".to_string(),
            image_version: "0x1".to_string(),
            pup_sha256: "ab".to_string(),
        }),
        game: None,
    };
    let path =
        crate::paths::boot_anchor_path_in(fixtures.path(), "NPAA60006", &cell_key(REFERENCE_FW));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&anchor).unwrap()).unwrap();
    assert!(matches!(
        render_row(&title("NPAA60006"), fixtures.path()),
        Err(SummaryLoadError::CellFirmwareMismatch { .. })
    ));
}

#[test]
fn a_summary_naming_no_firmware_still_renders_its_verdict() {
    let fixtures = Fixtures::new("fw-unstamped");
    fixtures.write("NPAA60007", REFERENCE_FW, &summary(None, None));
    let row = render_row(&title("NPAA60007"), fixtures.path()).unwrap();
    assert!(row.contains("| Yes |"), "{row}");
    assert!(row.contains("| equivalent |"), "{row}");
}
