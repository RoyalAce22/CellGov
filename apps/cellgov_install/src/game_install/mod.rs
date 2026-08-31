//! Game-install orchestration: turn a container the operator supplies
//! into a committed tree on disk and the [`crate::store`] record that
//! names it.
//!
//! A base goes into the guest-visible `dev_hdd0` / `dev_bdvd` tree; an
//! update goes into its own store entry under
//! [`crate::store::StoreLayout::entry_dir`].

mod base;
mod error;
mod staging;
mod update;

pub use error::GameInstallError;
pub use staging::InstallOptions;

pub use base::GameInstallOutcome;
#[cfg(feature = "decrypt")]
pub use base::{install_iso, install_pkg};

#[cfg(feature = "decrypt")]
pub use update::install_update_pkg;
pub use update::UpdateInstallOutcome;

pub(crate) use staging::sha256_of;
