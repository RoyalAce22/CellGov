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
pub(crate) use container::{container_label, install_caps, map_container, megabytes, vault};
pub(crate) use error::StoreCliError;
pub(crate) use registry::registry_dir;

/// Print the note for renames that landed only after a retry.
///
/// `retries` sums every rename the command made: a PKG install renames
/// the RAP and the tree, and an `--all` uninstall renames each entry.
pub(crate) fn report_rename_retries(retries: u32) {
    if retries > 0 {
        eprintln!(
            "  note: {retries} rename refusal(s) were outwaited before the renames landed; \
             a handle was open on a path a rename touched, typically an on-access scanner's"
        );
    }
}

#[cfg(test)]
#[path = "tests/scratch.rs"]
pub(crate) mod scratch;

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;
