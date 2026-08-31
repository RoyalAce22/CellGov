//! Firmware-install orchestration: turn a PUP the operator supplies
//! into a committed store entry and the [`crate::store`] record that
//! names it.

mod error;
mod install;
mod manifest_build;
mod prune;
mod version_txt;

pub use error::{FirmwareInstallError, PackageFailure};
pub use install::{FirmwareInstallOutcome, PackageSummary, DEV_FLASH_MOUNT};
pub use manifest_build::ManifestOmission;

#[cfg(feature = "decrypt")]
pub use install::install_pup;
