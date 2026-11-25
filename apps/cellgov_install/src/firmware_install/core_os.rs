//! Unpacks the LV2 kernel and the hypervisor out of
//! `CORE_OS_PACKAGE.pkg` into `core_os/` beside `dev_flash/`, each
//! SCE-wrapped as shipped.
//!
//! Nothing here fails an install. Every way either SELF can fail to
//! land is a [`CoreOsOmission`] the record carries by name, so a PUP
//! whose CoreOS package yields no kernel still installs its `dev_flash`
//! tree.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the package decrypt is gated; the feature-on build lints these"
    )
)]

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::format::core_os::{CORE_OS_PACKAGE_NAME, LV1_SELF, LV2_KERNEL_SELF};

use crate::core_os::{self, CoreOsTableError};
use crate::keys::KeyVault;
use crate::manifest;
use crate::store::layout::CORE_OS_DIR;
use crate::store::record::{CoreOsFileRecord, CoreOsRecord, KernelRecord};
use crate::{sce, tar};

/// Why the unpack wrote no kernel, or no hypervisor.
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
    /// The write of the kernel or the hypervisor into the entry failed.
    #[error("write {}: {source}", path.display())]
    Write {
        /// Where the file was to land.
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

/// The hypervisor's path relative to the entry directory, `/`-separated.
pub(crate) fn lv1_rel_path() -> String {
    format!("{CORE_OS_DIR}/{LV1_SELF}")
}

/// What one unpack produced: the table as read, the kernel or the
/// reason there is none, and the hypervisor.
#[derive(Debug)]
pub(super) struct CoreOsUnpack {
    /// Every entry the table held; empty when the table did not parse.
    pub files: Vec<CoreOsFileRecord>,
    /// The kernel, or why the unpack wrote none.
    pub kernel: Result<KernelRecord, CoreOsOmission>,
    /// The hypervisor, or why the unpack wrote none; `None` when no table
    /// parsed or the table names no hypervisor.
    pub lv1: Option<Result<KernelRecord, CoreOsOmission>>,
}

impl CoreOsUnpack {
    fn omitted(files: Vec<CoreOsFileRecord>, omission: CoreOsOmission) -> Self {
        Self {
            files,
            kernel: Err(omission),
            lv1: None,
        }
    }

    /// Why the unpack wrote no kernel, as text; `None` when it wrote one.
    pub fn kernel_absence(&self) -> Option<String> {
        self.kernel.as_ref().err().map(ToString::to_string)
    }

    /// Why the unpack wrote no hypervisor, as text; `None` when it wrote
    /// one.
    ///
    /// A package that yielded no table stores neither SELF, so the
    /// kernel's omission is also the hypervisor's reason.
    pub fn lv1_absence(&self) -> Option<String> {
        match (&self.lv1, &self.kernel) {
            (Some(Ok(_)), _) => None,
            (Some(Err(omission)), _) => Some(omission.to_string()),
            (
                None,
                Err(
                    omission @ (CoreOsOmission::NoPackage
                    | CoreOsOmission::Undecryptable { .. }
                    | CoreOsOmission::Table { .. }),
                ),
            ) => Some(omission.to_string()),
            (None, Ok(_) | Err(CoreOsOmission::NoKernel { .. } | CoreOsOmission::Write { .. })) => {
                Some(format!("{CORE_OS_PACKAGE_NAME} names no {LV1_SELF}"))
            }
        }
    }

    /// The block the install record carries.
    pub fn into_record(self) -> CoreOsRecord {
        let (kernel, omission) = match self.kernel {
            Ok(kernel) => (Some(kernel), None),
            Err(omission) => (None, Some(omission.to_string())),
        };
        let (lv1, lv1_omission) = match self.lv1 {
            Some(Ok(lv1)) => (Some(lv1), None),
            Some(Err(omission)) => (None, Some(omission.to_string())),
            None => (None, None),
        };
        CoreOsRecord {
            files: self.files,
            kernel,
            omission,
            lv1,
            lv1_omission,
        }
    }
}

/// The CoreOS package in the outer TAR, matched on its bare name.
pub(super) fn find_package(outer: &[tar::TarEntry]) -> Option<&tar::TarEntry> {
    outer
        .iter()
        .find(|e| e.name.rsplit('/').next() == Some(CORE_OS_PACKAGE_NAME))
}

/// Read a decrypted image's table and write its kernel and its
/// hypervisor under `entry_root`.
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
    let store = |entry: &core_os::CoreOsEntry, name: &str, rel_path: String| {
        let bytes = entry
            .payload(image)
            .expect("invariant: parse_table bounded every entry against this image");
        let dest = entry_root.join(CORE_OS_DIR).join(name);
        match write_whole(&dest, bytes) {
            Ok(()) => Ok(KernelRecord {
                path: rel_path,
                stored_sha256: manifest::Sha256(manifest::sha256_of(bytes)),
            }),
            Err(source) => Err(CoreOsOmission::Write { path: dest, source }),
        }
    };
    let lv1 = table
        .find(LV1_SELF)
        .map(|entry| store(entry, LV1_SELF, lv1_rel_path()));
    let kernel = match table.find(LV2_KERNEL_SELF) {
        Some(entry) => store(entry, LV2_KERNEL_SELF, kernel_rel_path()),
        None => Err(CoreOsOmission::NoKernel { files: files.len() }),
    };
    CoreOsUnpack { files, kernel, lv1 }
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

/// Open the CoreOS package in `outer` and write its kernel and its
/// hypervisor under `entry_root`.
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

#[cfg(test)]
#[path = "tests/core_os_lv1_tests.rs"]
mod lv1_tests;
