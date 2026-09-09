//! The refusals that return before any output file exists.

use std::path::Path;

use super::{
    save_boot_observation, save_boot_summary_json, ObservationInputs, ObservationSaveError,
};

const PROCESS_EXIT_TOML: &str = r#"
[title]
content_id = "NPAA00001"
short_name = "proc-exit-fixture"
display_name = "Process-exit checkpoint fixture"
eboot_candidates = ["EBOOT.BIN"]
year = 2007
developer = "test-developer"
engine = "test-engine"
distribution = "psn-hdd"
system_ver = "4.93"

[checkpoint]
kind = "process-exit"
"#;

fn process_exit_title() -> crate::game::manifest::TitleManifest {
    crate::game::manifest::TitleManifest::load_from_text(
        PROCESS_EXIT_TOML,
        Path::new("title_manifests/proc-exit-fixture.toml"),
    )
    .expect("fixture manifest loads")
}

#[test]
fn an_inconsistent_boot_summary_is_refused_before_any_file_is_created() {
    let dir = cellgov_testkit::scratch::scratch_labeled("summary_inconsistent");
    let out = dir.join("boot_summary.json");

    let err = save_boot_summary_json(
        out.to_str().unwrap(),
        &process_exit_title(),
        cellgov_compare::BootOutcome::RsxWriteCheckpoint,
        1,
        cellgov_time::Budget::new(1),
        0,
        cellgov_compare::RunIdentity::default(),
    )
    .expect_err("an RSX-write outcome under a process-exit checkpoint is inconsistent");
    assert!(
        matches!(
            err,
            ObservationSaveError::InvalidBootSummary(
                cellgov_compare::BootSummaryError::RsxWriteOutcomeWithoutRsxCheckpoint { .. }
            )
        ),
        "expected InvalidBootSummary(RsxWriteOutcomeWithoutRsxCheckpoint), got {err:?}"
    );
    assert!(
        !out.exists(),
        "a refused summary must leave no file behind for an anchor to read"
    );
}

#[test]
fn an_image_with_no_pt_load_table_is_refused_before_any_file_is_created() {
    let dir = cellgov_testkit::scratch::scratch_labeled("pt_load_enum");
    let out = dir.join("observation.json");
    let spaces: cellgov_compare::SpaceSnapshots = [(
        cellgov_compare::AddressSpaceId::BOOT,
        cellgov_mem::GuestMemory::new(0x1000),
    )]
    .into_iter()
    .collect();

    let err = save_boot_observation(ObservationInputs {
        path: out.to_str().unwrap(),
        elf_data: &[],
        final_spaces: &spaces,
        outcome: cellgov_compare::BootOutcome::ProcessExit,
        steps: 0,
        manifest_regions: None,
        tty_log: &[],
        identity: &cellgov_compare::RunIdentity::default(),
    })
    .expect_err("an empty image enumerates no PT_LOAD table");
    assert!(
        matches!(
            err,
            ObservationSaveError::PtLoadEnum {
                source: cellgov_ppu::loader::LoadError::TooSmall
            }
        ),
        "expected PtLoadEnum(TooSmall), got {err:?}"
    );
    assert!(
        !out.exists(),
        "a refused default-region enumeration must leave no file behind"
    );
}
