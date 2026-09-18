//! Synthetic titles and committed artifacts the generator's tests
//! render from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_compare::{
    AppVersion, BootOutcome, BootSummary, ByteParity, CheckpointKind, Convergence,
    ConvergenceFailure, CrossRunnerSummary, DivergenceClass, FirmwareIdentity, GameIdentity,
    ObservedOutcome, RunIdentity,
};
use cellgov_time::Budget;

use cellgov_boot::manifest::{
    CellExpectation, CellKey, CheckpointTrigger, Distribution, GameSource, MatrixCell,
    TitleManifest,
};

pub(crate) const REFERENCE_FW: &str = "4.93";
pub(crate) const BASE: &str = "base";

/// A scratch fixture tree that cleans itself up.
pub(crate) struct Fixtures(cellgov_testkit::scratch::ScratchDir);

impl Fixtures {
    pub(crate) fn new(name: &str) -> Self {
        Self(cellgov_testkit::scratch::scratch_labeled(name))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn write_anchor(&self, content_id: &str, cell: &CellKey, anchor: &BootSummary) {
        write_json(
            &crate::paths::boot_anchor_path_in(self.path(), content_id, cell),
            anchor,
        );
    }

    pub(crate) fn write_cross(
        &self,
        content_id: &str,
        cell: &CellKey,
        summary: &CrossRunnerSummary,
    ) {
        write_json(
            &crate::paths::cross_runner_summary_path_in(self.path(), content_id, cell),
            summary,
        );
    }
}

pub(crate) fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

pub(crate) fn anchor_path(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    crate::paths::boot_anchor_path_in(fixtures, content_id, cell)
}

pub(crate) fn cross_path(fixtures: &Path, content_id: &str, cell: &CellKey) -> PathBuf {
    crate::paths::cross_runner_summary_path_in(fixtures, content_id, cell)
}

pub(crate) fn cell_key(fw: &str, game_ver: Option<&str>) -> CellKey {
    CellKey {
        fw: fw.to_string(),
        game_ver: game_ver.map(str::to_string),
    }
}

/// The cell every title from [`title`] derives from its `system_ver`.
pub(crate) fn reference_key() -> CellKey {
    cell_key(REFERENCE_FW, Some(BASE))
}

pub(crate) fn matrix_cell(key: CellKey) -> MatrixCell {
    MatrixCell {
        key,
        expect: CellExpectation::Frontier,
        bench_max_steps: None,
        checkpoint: None,
        pending: None,
    }
}

pub(crate) fn title(content_id: &str, display: &str, year: u16, developer: &str) -> TitleManifest {
    TitleManifest {
        content_id: content_id.to_string(),
        short_name: content_id.to_lowercase(),
        display_name: display.to_string(),
        eboot_candidates: vec!["EBOOT.elf".to_string()],
        year,
        developer: developer.to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        system_ver: Some(REFERENCE_FW.to_string()),
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: vec![matrix_cell(reference_key())],
    }
}

/// A title shipped inside the firmware: no `system_ver`, and one
/// declared cell per firmware in `fws`.
pub(crate) fn firmware_exec_title(content_id: &str, display: &str, fws: &[&str]) -> TitleManifest {
    let mut t = title(content_id, display, 2006, "Studio");
    t.distribution = Distribution::FirmwareExec;
    t.source = GameSource::FirmwareExec {
        dir: PathBuf::from("dev_flash/vsh/module"),
    };
    t.system_ver = None;
    t.matrix = fws
        .iter()
        .map(|fw| matrix_cell(cell_key(fw, None)))
        .collect();
    t
}

pub(crate) fn boot(outcome: BootOutcome, steps: u64) -> BootSummary {
    BootSummary::new(
        CheckpointKind::ProcessExit,
        outcome,
        steps,
        Budget::new(256),
    )
    .unwrap()
}

pub(crate) fn boot_on(outcome: BootOutcome, steps: u64, fw: &str) -> BootSummary {
    let mut b = boot(outcome, steps);
    b.identity = RunIdentity {
        firmware: Some(firmware(fw)),
        game: None,
        overrides: Default::default(),
    };
    b
}

pub(crate) fn firmware(version: &str) -> FirmwareIdentity {
    FirmwareIdentity {
        version: version.to_string(),
        image_version: "0x1".to_string(),
        pup_sha256: "ab".to_string(),
    }
}

/// A converged summary with `bytes` classified divergent bytes.
pub(crate) fn converged(bytes: u64) -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: if bytes == 0 {
            ByteParity::Equivalent
        } else {
            ByteParity::NonSemantic { bytes }
        },
        per_class_bytes: if bytes == 0 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(DivergenceClass::ElfHeader, bytes)])
        },
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
        identity: RunIdentity::default(),
        rpcs3_firmware: None,
        oracle_gap_ordinals: None,
    }
}

pub(crate) fn converged_pending(non_semantic: u64, unclassified: u64) -> CrossRunnerSummary {
    CrossRunnerSummary {
        convergence: Convergence::Yes,
        byte_parity: ByteParity::Pending {
            non_semantic_bytes: non_semantic,
            unclassified_bytes: unclassified,
        },
        per_class_bytes: BTreeMap::from([
            (DivergenceClass::ElfHeader, non_semantic),
            (DivergenceClass::Unclassified, unclassified),
        ]),
        unclassified_bytes: unclassified,
        unclassified_runs: vec![cellgov_compare::UnclassifiedRun {
            region_name: "data".to_string(),
            offset: 0,
            length: unclassified,
        }],
        lowest_offset_class: None,
        identity: RunIdentity::default(),
        rpcs3_firmware: None,
        oracle_gap_ordinals: None,
    }
}

pub(crate) fn diverged() -> CrossRunnerSummary {
    let reason = ConvergenceFailure::OutcomeMismatch {
        cellgov: ObservedOutcome::Fault,
        rpcs3: ObservedOutcome::Completed,
    };
    CrossRunnerSummary {
        convergence: Convergence::No {
            reason: reason.clone(),
        },
        byte_parity: ByteParity::Diverge { reason },
        per_class_bytes: BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
        identity: RunIdentity::default(),
        rpcs3_firmware: None,
        oracle_gap_ordinals: None,
    }
}

/// The `Display` a divergent summary's reason renders as.
pub(crate) const DIVERGED_REASON: &str = "outcome: Fault vs Completed";

pub(crate) fn stamped(
    mut summary: CrossRunnerSummary,
    cellgov_fw: Option<&str>,
    rpcs3_fw: Option<&str>,
) -> CrossRunnerSummary {
    summary.identity = RunIdentity {
        firmware: cellgov_fw.map(firmware),
        game: Some(GameIdentity {
            title_id: "NPAA60001".to_string(),
            version: BASE.to_string(),
            app_version: Some(AppVersion::AppVer("01.00".to_string())),
        }),
        overrides: Default::default(),
    };
    summary.rpcs3_firmware = rpcs3_fw.map(str::to_string);
    summary
}
