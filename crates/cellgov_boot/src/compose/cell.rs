//! The cell a composed boot runs in, what the registry declares for it,
//! and the run inputs that move a run off the trajectory the cell's
//! anchor recorded.

use std::path::PathBuf;

use cellgov_compare::BootOverrides;
use cellgov_install::store::select::GameVersion;
use cellgov_time::Budget;

use super::composition::{BootComposition, FirmwareChoice, GameChoice};
use crate::manifest::{CellKey, CheckpointTrigger, TitleManifest, BASE_GAME_VER};

/// The `sys/external` modules inside a firmware entry, relative to the
/// entry's `dev_flash` mount.
const FIRMWARE_EXTERNAL: [&str; 2] = ["sys", "external"];

/// The `sys/external` directory the firmware loader reads its modules
/// from, or `None` for a boot with no firmware.
pub fn firmware_module_dir(composition: &BootComposition) -> Option<PathBuf> {
    match &composition.firmware {
        FirmwareChoice::Managed(managed) => Some(
            FIRMWARE_EXTERNAL
                .iter()
                .fold(managed.entry.dev_flash_dir(), |d, part| d.join(part)),
        ),
        // `--firmware-dir` names a `sys/external` tree directly, so
        // this arm joins nothing onto it.
        FirmwareChoice::Unmanaged { dir } => Some(dir.clone()),
        FirmwareChoice::None => None,
    }
}

/// The cell this composition puts the run in.
///
/// Returns `None` when the composition names no key an anchor could be
/// filed under:
///
/// - an unmanaged or absent firmware carries no version;
/// - a title the store does not hold has no game-version axis;
/// - an executable outside the selected firmware entry belongs to no
///   entry.
pub fn composed_cell(composition: &BootComposition) -> Option<CellKey> {
    let fw = composition.firmware.version()?.to_string();
    let game_ver = match &composition.game {
        GameChoice::Stored(stored) => Some(match &stored.version {
            GameVersion::Base => BASE_GAME_VER.to_string(),
            GameVersion::Update(v) => v.clone(),
        }),
        // The manifest named an absolute `firmware-exec` path, and the
        // composition keeps it as written. The executable therefore did
        // not come from the firmware entry this key would name.
        GameChoice::Firmware {
            unmanaged_path: true,
            ..
        } => return None,
        // A firmware-shipped executable has no version axis of its own,
        // so its cell is the firmware alone.
        GameChoice::Firmware { .. } => None,
        GameChoice::Unstored => return None,
    };
    Some(CellKey { fw, game_ver })
}

/// What the registry declares for the cell this run composed.
///
/// The run and the cell's anchor use the same cap and the same
/// checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlan {
    /// The composed cell; see [`composed_cell`].
    pub cell: Option<CellKey>,
    /// Instruction cap the cell's anchor is recorded under.
    pub max_steps: u64,
    /// Checkpoint the cell's anchor is recorded under.
    pub checkpoint: CheckpointTrigger,
}

impl ResolvedPlan {
    /// The plan `composition` puts `title` in.
    pub fn resolve(title: &TitleManifest, composition: &BootComposition) -> Self {
        let cell = composed_cell(composition);
        // A cell the matrix does not declare takes the title's own
        // defaults.
        let declared = cell.as_ref().and_then(|k| title.cell(k));
        Self {
            max_steps: title.cell_max_steps(declared),
            checkpoint: title.cell_checkpoint(declared),
            cell,
        }
    }

    /// The cap as a step count, or `None` when it does not fit this
    /// host's `usize`.
    pub fn max_steps_usize(&self) -> Option<usize> {
        usize::try_from(self.max_steps).ok()
    }
}

/// The execution inputs both `boot run` and `boot bench` take that can
/// move a run off the trajectory its cell's anchor recorded.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionOverrides<'a> {
    /// A budget that replaces the manifest's.
    pub budget: Option<Budget>,
    /// Whether a reserved-region write faults instead of being served.
    pub strict_reserved: bool,
    /// Guest argv for the primary thread; the anchor is recorded with none.
    pub guest_args: &'a [String],
    /// The boot overrides the run applies.
    pub boot: &'a BootOverrides,
}

/// One execution input that moves a run off its anchor's trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrajectoryOverride<'a> {
    /// A budget other than the manifest's.
    Budget(Budget),
    /// Reserved-region writes fault.
    StrictReserved,
    /// This many guest argv entries.
    GuestArgs(usize),
    /// At least one boot override.
    Boot(&'a BootOverrides),
}

impl<'a> ExecutionOverrides<'a> {
    /// Every input here that moves the run, in a fixed order: budget,
    /// strict reserved, guest argv, boot overrides.
    pub fn trajectory_overrides(&self) -> Vec<TrajectoryOverride<'a>> {
        let mut out = Vec::new();
        if let Some(b) = self.budget {
            out.push(TrajectoryOverride::Budget(b));
        }
        if self.strict_reserved {
            out.push(TrajectoryOverride::StrictReserved);
        }
        if !self.guest_args.is_empty() {
            out.push(TrajectoryOverride::GuestArgs(self.guest_args.len()));
        }
        if !self.boot.is_empty() {
            out.push(TrajectoryOverride::Boot(self.boot));
        }
        out
    }
}

#[cfg(test)]
#[path = "tests/cell_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/shipped_firmware_cell_tests.rs"]
mod shipped_firmware_cell_tests;
