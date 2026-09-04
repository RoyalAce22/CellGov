//! Handlers for the commands that read and write the content store:
//! `status`, `firmware`, `title`, `keys`, and `self decrypt`.

pub(crate) mod confirm;
#[cfg(feature = "decrypt")]
mod container;
mod error;
pub(crate) mod firmware;
pub(crate) mod keys_cmd;
#[cfg(feature = "decrypt")]
pub(crate) mod rap;
pub(crate) mod read;
mod registry;
pub(crate) mod self_decrypt;
pub(crate) mod title;
pub(crate) mod uninstall;

#[cfg(feature = "decrypt")]
pub(crate) use container::{container_label, map_container_or_die, megabytes, vault_or_die};
pub(crate) use error::StoreCliError;
pub(crate) use registry::registry_dir;

#[cfg(test)]
#[path = "tests/scratch.rs"]
pub(crate) mod scratch;

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;
