//! A base's `system_ver` floor, read from the PARAM.SFO its installed
//! tree carries.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::param_sfo::PS3_SYSTEM_VER_KEY;

use crate::manifest::{sha256_of, Sha256};
use crate::param_sfo::{self, SfoError};
use crate::store::layout::StoreLayout;
use crate::store::record::{InstallRecord, TitleRecord};
use crate::system_ver::{firmware_version_key, SystemVerError};

/// Why [`base_system_ver`] found no floor.
#[derive(Debug, thiserror::Error)]
pub enum FloorReadError {
    /// Reading the PARAM.SFO failed.
    #[error("read {}: {source}", sfo.display())]
    Read {
        /// The table's path.
        sfo: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The table hashes to something other than the digest the record
    /// holds for it, so it belongs to some other install of the title.
    #[error("{}: SHA-256 {} is not the recorded {}", sfo.display(), found.to_hex(), recorded.to_hex())]
    DigestMismatch {
        /// The table's path.
        sfo: PathBuf,
        /// The digest of the bytes read.
        found: Sha256,
        /// The digest the record holds.
        recorded: Sha256,
    },
    /// The bytes are not a PARAM.SFO.
    #[error("{}: {source}", sfo.display())]
    Parse {
        /// The table's path.
        sfo: PathBuf,
        /// The parser's refusal.
        #[source]
        source: SfoError,
    },
    /// The table holds no `PS3_SYSTEM_VER` string.
    #[error("{}: no {PS3_SYSTEM_VER_KEY} string", sfo.display())]
    NoSystemVer {
        /// The table's path.
        sfo: PathBuf,
    },
    /// The `PS3_SYSTEM_VER` value names no firmware version key.
    #[error("{}: {source}", sfo.display())]
    Shape {
        /// The table's path.
        sfo: PathBuf,
        /// The value's refusal.
        #[source]
        source: SystemVerError,
    },
}

/// The PARAM.SFO a title-base record's installed tree carries, below
/// `store_root`.
///
/// The record's `store_path` must satisfy the precondition of
/// [`StoreLayout::resolve_store_path`]; a record from
/// [`InstallRecord::parse`] does.
#[must_use]
pub fn installed_param_sfo(
    store_root: &Path,
    record: &InstallRecord,
    title: &TitleRecord,
) -> PathBuf {
    let tree_dir = StoreLayout::new(store_root).resolve_store_path(&record.artifact.store_path);
    title.tree().param_sfo_in(&tree_dir)
}

/// The installed tree's `PS3_SYSTEM_VER` as a firmware version key
/// (`01.5000` becomes `1.50`).
///
/// The floor is read from the tree's own table. When the record's
/// `[files]` holds a digest for that table, the bytes must match it:
/// the uninstall gate holds the tree to those digests, so a table that
/// hashes differently is some other install's.
///
/// # Errors
///
/// A [`FloorReadError`] naming the table and why it gave no floor.
pub fn base_system_ver(
    store_root: &Path,
    record: &InstallRecord,
    title: &TitleRecord,
) -> Result<String, FloorReadError> {
    let sfo = installed_param_sfo(store_root, record, title);
    let recorded = record.files.get(&title.param_sfo_rel());
    let bytes = match std::fs::read(&sfo) {
        Ok(bytes) => bytes,
        Err(source) => return Err(FloorReadError::Read { sfo, source }),
    };
    if let Some(recorded) = recorded {
        let found = Sha256(sha256_of(&bytes));
        if found != *recorded {
            return Err(FloorReadError::DigestMismatch {
                sfo,
                found,
                recorded: *recorded,
            });
        }
    }
    let table = match param_sfo::parse(&bytes) {
        Ok(table) => table,
        Err(source) => return Err(FloorReadError::Parse { sfo, source }),
    };
    let Some(raw) = table.get_string(PS3_SYSTEM_VER_KEY) else {
        return Err(FloorReadError::NoSystemVer { sfo });
    };
    firmware_version_key(raw).map_err(|source| FloorReadError::Shape { sfo, source })
}

#[cfg(test)]
#[path = "tests/floor_tests.rs"]
mod tests;
