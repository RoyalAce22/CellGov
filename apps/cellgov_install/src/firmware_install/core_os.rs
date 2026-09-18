//! Unpacks the LV2 kernel out of `CORE_OS_PACKAGE.pkg` into `core_os/`
//! beside `dev_flash/`.
//!
//! Nothing here fails an install. Every way the kernel can fail to land
//! is a [`CoreOsOmission`] the record carries by name, so a PUP whose
//! CoreOS package yields no kernel still installs its `dev_flash` tree.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the package decrypt is gated; the feature-on build lints these"
    )
)]

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::core_os::{CORE_OS_PACKAGE_NAME, LV2_KERNEL_SELF};

use crate::core_os::{self, CoreOsTableError};
use crate::keys::KeyVault;
use crate::manifest;
use crate::store::layout::CORE_OS_DIR;
use crate::store::record::{CoreOsFileRecord, CoreOsRecord, KernelRecord};
use crate::{sce, tar};

/// Why the unpack wrote no kernel.
#[derive(Debug, thiserror::Error)]
pub enum CoreOsOmission {
    /// The `update_files` TAR carries no CoreOS package.
    #[error("update_files carries no {CORE_OS_PACKAGE_NAME}")]
    NoPackage,
    /// The package's SCE envelope did not open under any package keyset.
    #[error("{CORE_OS_PACKAGE_NAME}: {source}")]
    Undecryptable {
        /// Why the decrypt failed.
        #[source]
        source: sce::SceError,
    },
    /// The decrypted image's file table did not parse.
    #[error("{CORE_OS_PACKAGE_NAME}: {source}")]
    Table {
        /// Why `parse_table` refused it.
        #[source]
        source: CoreOsTableError,
    },
    /// The table parsed and names no kernel.
    #[error("{CORE_OS_PACKAGE_NAME} names no {LV2_KERNEL_SELF} among its {files} file(s)")]
    NoKernel {
        /// Entries the table held.
        files: usize,
    },
    /// The write of the kernel into the entry failed.
    #[error("write {}: {source}", path.display())]
    Write {
        /// Where the kernel was to land.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// The kernel's path relative to the entry directory, `/`-separated.
pub(crate) fn kernel_rel_path() -> String {
    format!("{CORE_OS_DIR}/{LV2_KERNEL_SELF}")
}

/// What one unpack produced: the table as read, and the kernel or the
/// reason there is none.
#[derive(Debug)]
pub(super) struct CoreOsUnpack {
    /// Every entry the table held; empty when the table did not parse.
    pub files: Vec<CoreOsFileRecord>,
    /// The kernel, or why the unpack wrote none.
    pub kernel: Result<KernelRecord, CoreOsOmission>,
}

impl CoreOsUnpack {
    fn omitted(files: Vec<CoreOsFileRecord>, omission: CoreOsOmission) -> Self {
        Self {
            files,
            kernel: Err(omission),
        }
    }

    /// The block the install record carries.
    pub fn into_record(self) -> CoreOsRecord {
        match self.kernel {
            Ok(kernel) => CoreOsRecord {
                files: self.files,
                kernel: Some(kernel),
                omission: None,
            },
            Err(omission) => CoreOsRecord {
                files: self.files,
                kernel: None,
                omission: Some(omission.to_string()),
            },
        }
    }
}

/// The CoreOS package in the outer TAR, matched on its bare name.
pub(super) fn find_package(outer: &[tar::TarEntry]) -> Option<&tar::TarEntry> {
    outer
        .iter()
        .find(|e| e.name.rsplit('/').next() == Some(CORE_OS_PACKAGE_NAME))
}

/// Read a decrypted image's table and write its kernel under
/// `entry_root`.
pub(super) fn unpack_image(image: &[u8], entry_root: &Path) -> CoreOsUnpack {
    let table = match core_os::parse_table(image) {
        Ok(table) => table,
        Err(source) => return CoreOsUnpack::omitted(Vec::new(), CoreOsOmission::Table { source }),
    };
    let files: Vec<CoreOsFileRecord> = table
        .entries
        .iter()
        .map(|e| CoreOsFileRecord {
            name: e.name.clone(),
            size: e.size,
        })
        .collect();
    let Some(kernel) = table.find(LV2_KERNEL_SELF) else {
        let omission = CoreOsOmission::NoKernel { files: files.len() };
        return CoreOsUnpack::omitted(files, omission);
    };
    let bytes = kernel
        .payload(image)
        .expect("invariant: parse_table bounded every entry against this image");
    let dest = entry_root.join(CORE_OS_DIR).join(LV2_KERNEL_SELF);
    let kernel = match write_whole(&dest, bytes) {
        Ok(()) => Ok(KernelRecord {
            path: kernel_rel_path(),
            stored_sha256: manifest::Sha256(manifest::sha256_of(bytes)),
        }),
        Err(source) => Err(CoreOsOmission::Write { path: dest, source }),
    };
    CoreOsUnpack { files, kernel }
}

/// Write `bytes` to `dest` through a `.part` sibling, so a fault mid-write
/// leaves no half-written file under the final name.
pub(super) fn write_whole(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let Some(parent) = dest.parent() else {
        return Err(std::io::Error::other(
            "the destination path names no directory",
        ));
    };
    std::fs::create_dir_all(parent)?;
    let part = parent.join(format!(
        ".{}.part",
        dest.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default()
    ));
    let landed = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, dest));
    if landed.is_err() {
        // Best effort: the write refusal is the one the caller gets.
        let _ = std::fs::remove_file(&part);
    }
    landed
}

/// Open the CoreOS package in `outer` and write its kernel under
/// `entry_root`.
#[cfg(feature = "decrypt")]
pub(super) fn unpack(outer: &[tar::TarEntry], entry_root: &Path, keys: &KeyVault) -> CoreOsUnpack {
    let Some(package) = find_package(outer) else {
        return CoreOsUnpack::omitted(Vec::new(), CoreOsOmission::NoPackage);
    };
    match sce::decrypt_package(&package.data, keys) {
        Ok(image) => unpack_image(&image, entry_root),
        Err(source) => CoreOsUnpack::omitted(Vec::new(), CoreOsOmission::Undecryptable { source }),
    }
}

#[cfg(test)]
#[path = "tests/core_os_tests.rs"]
mod tests;
