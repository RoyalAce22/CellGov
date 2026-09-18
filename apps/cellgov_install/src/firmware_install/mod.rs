//! Firmware-install orchestration: turn a PUP the operator supplies
//! into a committed store entry and the [`crate::store`] record that
//! names it, or complete an installed entry with the kernel that PUP
//! carries.

mod complete;
mod core_os;
mod error;
mod install;
mod manifest_build;
mod prune;
mod version_txt;

pub use complete::KernelCompletionOutcome;
pub use core_os::CoreOsOmission;
pub use error::{FirmwareInstallError, PackageFailure};
pub use install::{installed_record, FirmwareInstallOutcome, PackageSummary};
pub use manifest_build::ManifestOmission;

#[cfg(feature = "decrypt")]
pub use complete::complete_kernel;
#[cfg(feature = "decrypt")]
pub use install::install_pup;
