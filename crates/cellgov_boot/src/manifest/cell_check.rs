//! Whether a run's identity belongs to the cell a result is filed under.
//!
//! `dev record-anchors` holds a fresh run against the cell it would
//! record, and `dev titles-gen` holds a committed file against the cell
//! whose directory it sits in. Both ask the same question of the same
//! two values, and each decides what an identity that names nothing
//! means for it.

use cellgov_compare::{GameIdentity, RunIdentity};

use super::matrix::CellKey;

/// One way a run's identity contradicts a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellDisagreement {
    /// The run applied boot overrides, and no anchor is recorded under
    /// one.
    Overridden {
        /// The overrides, as the identity's wire form names them.
        names: Vec<String>,
    },
    /// The run composed another firmware.
    FirmwareMismatch {
        /// Firmware version the cell names.
        cell: String,
        /// Firmware version the identity states.
        recorded: String,
    },
    /// The identity names no managed firmware.
    NoFirmware {
        /// Firmware version the cell names.
        cell: String,
    },
    /// The run composed another game version.
    GameVersionMismatch {
        /// Game version the cell names, in the identity's spelling
        /// ([`GameIdentity::version_of`]); `None` for a title shipped
        /// inside the firmware.
        cell: Option<String>,
        /// Game version the identity states.
        recorded: String,
    },
    /// The identity names no game entry where the cell names a version.
    NoGameVersion {
        /// Game version the cell names, in the identity's spelling.
        cell: String,
    },
}

impl CellKey {
    /// Every way `identity` contradicts this cell: boot overrides first,
    /// then the firmware, then the game version.
    ///
    /// Firmware versions compare directly: both sides are the store key
    /// of a `vfs/firmware/<key>/` entry. A game version is compared in
    /// the identity's spelling, since the composition records a selected
    /// update as `update:<key>`.
    #[must_use]
    pub fn disagreements(&self, identity: &RunIdentity) -> Vec<CellDisagreement> {
        let mut out = Vec::new();
        if !identity.overrides.is_empty() {
            out.push(CellDisagreement::Overridden {
                names: identity.overrides.names(),
            });
        }
        match &identity.firmware {
            Some(f) if f.version == self.fw => {}
            Some(f) => out.push(CellDisagreement::FirmwareMismatch {
                cell: self.fw.clone(),
                recorded: f.version.clone(),
            }),
            None => out.push(CellDisagreement::NoFirmware {
                cell: self.fw.clone(),
            }),
        }
        let want = self.game_ver.as_deref().map(GameIdentity::version_of);
        match (&identity.game, want) {
            (Some(g), want) if want.as_deref() != Some(g.version.as_str()) => {
                out.push(CellDisagreement::GameVersionMismatch {
                    cell: want,
                    recorded: g.version.clone(),
                });
            }
            (None, Some(cell)) => out.push(CellDisagreement::NoGameVersion { cell }),
            (Some(_), _) | (None, None) => {}
        }
        out
    }
}

#[cfg(test)]
#[path = "tests/cell_check_tests.rs"]
mod tests;
