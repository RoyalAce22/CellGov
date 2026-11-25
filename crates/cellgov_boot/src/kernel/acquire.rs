//! The stored kernel of the selected firmware, decrypted and mapped.

use std::path::PathBuf;

use cellgov_install::sce::SceError;
use cellgov_install::self_image::{into_plaintext_elf, KeyPolicy};
use cellgov_install::store::KernelAbsence;

use super::map::{map_kernel, KernelImage, KernelMapError};
use crate::compose::FirmwareChoice;
use crate::KeyVaultSource;

/// The kernel of record, mapped.
#[derive(Debug)]
pub struct LoadedKernel {
    /// The firmware version whose store entry held it.
    pub firmware: String,
    /// The kernel at its link address.
    pub image: KernelImage,
}

/// Why a boot has no kernel of record.
#[derive(Debug, thiserror::Error)]
pub enum KernelUnavailable {
    /// The boot runs with no firmware.
    #[error("the boot runs with no firmware, so it has no LV2 kernel")]
    NoFirmware,
    /// The firmware is a tree outside the store, and only a store
    /// record names a stored kernel.
    #[error("firmware tree {} is outside the store, which names no LV2 kernel for it", dir.display())]
    UnmanagedFirmware {
        /// The tree the boot named.
        dir: PathBuf,
    },
    /// The entry's record predates stored kernels.
    #[error("firmware {firmware}: the install record predates stored kernels")]
    NotRecorded {
        /// The selected firmware version.
        firmware: String,
    },
    /// The install stored no kernel.
    #[error("firmware {firmware}: the install stored no LV2 kernel ({})", reason.as_deref().unwrap_or("it recorded no reason"))]
    Omitted {
        /// The selected firmware version.
        firmware: String,
        /// The reason the install recorded, when it named one.
        reason: Option<String>,
    },
    /// The read of the stored kernel failed.
    #[error("firmware {firmware}: read {}: {source}", path.display())]
    Read {
        /// The selected firmware version.
        firmware: String,
        /// The stored kernel.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// The operator's key vault did not load.
    #[error("firmware {firmware}: key vault: {reason}")]
    Vault {
        /// The selected firmware version.
        firmware: String,
        /// The vault loader's own account of the refusal.
        reason: String,
    },
    /// This build has no `decrypt` feature.
    #[error("firmware {firmware}: this build cannot decrypt the LV2 kernel; rebuild with `--features decrypt`")]
    DecryptDisabled {
        /// The selected firmware version.
        firmware: String,
    },
    /// The vault holds no keyset that opens the kernel's envelope.
    #[error("firmware {firmware}: no key for the LV2 kernel: {source}")]
    NoKey {
        /// The selected firmware version.
        firmware: String,
        /// The decrypt's refusal.
        source: SceError,
    },
    /// The decrypt refused for any other reason.
    #[error("firmware {firmware}: decrypt the LV2 kernel: {source}")]
    Decrypt {
        /// The selected firmware version.
        firmware: String,
        /// The decrypt's refusal.
        source: SceError,
    },
    /// The plaintext ELF did not map.
    #[error("firmware {firmware}: {source}")]
    Map {
        /// The selected firmware version.
        firmware: String,
        /// The mapper's refusal.
        source: KernelMapError,
    },
}

/// The stored kernel of `firmware`, decrypted under `keys` and mapped.
///
/// # Errors
///
/// Returns the first [`KernelUnavailable`] that applies. The checks run
/// in variant order.
pub fn kernel_of_record(
    firmware: &FirmwareChoice,
    keys: &dyn KeyVaultSource,
) -> Result<LoadedKernel, KernelUnavailable> {
    let managed = match firmware {
        FirmwareChoice::Managed(managed) => managed,
        FirmwareChoice::Unmanaged { dir } => {
            return Err(KernelUnavailable::UnmanagedFirmware { dir: dir.clone() })
        }
        FirmwareChoice::None => return Err(KernelUnavailable::NoFirmware),
    };
    let entry = &managed.entry;
    let version = || entry.version.clone();
    let record = entry.stored_kernel().map_err(|absence| match absence {
        KernelAbsence::NotRecorded => KernelUnavailable::NotRecorded {
            firmware: version(),
        },
        KernelAbsence::Omitted(reason) => KernelUnavailable::Omitted {
            firmware: version(),
            reason: reason.map(str::to_string),
        },
    })?;
    let path = record.path_in(&entry.entry_dir);
    let raw = std::fs::read(&path).map_err(|source| KernelUnavailable::Read {
        firmware: version(),
        path,
        source,
    })?;
    let vault = keys.vault_for(&raw).map_err(|e| KernelUnavailable::Vault {
        firmware: version(),
        reason: e.to_string(),
    })?;
    let elf =
        into_plaintext_elf(raw, vault, KeyPolicy::AppOnly).map_err(|source| match source {
            SceError::DecryptFeatureDisabled => KernelUnavailable::DecryptDisabled {
                firmware: version(),
            },
            source if is_lv2_key_gap(&source) => KernelUnavailable::NoKey {
                firmware: version(),
                source,
            },
            source => KernelUnavailable::Decrypt {
                firmware: version(),
                source,
            },
        })?;
    let image = map_kernel(&elf).map_err(|source| KernelUnavailable::Map {
        firmware: version(),
        source,
    })?;
    Ok(LoadedKernel {
        firmware: version(),
        image,
    })
}

/// Whether a decrypt refusal names an LV2 keyset the vault lacks.
///
/// A padding or CBC refusal comes only from the envelope stage. That
/// stage tests the candidate keyset and nothing else, so the refusal
/// means the vault lacks the key. `kernel_decrypt::is_key_gap` in
/// `cellgov_install` files the same refusals as a key gap. A build
/// without `decrypt` does not compile that function.
fn is_lv2_key_gap(source: &SceError) -> bool {
    source.is_key_vault_refusal()
        || matches!(
            source,
            SceError::NoCandidateOpensEnvelope { .. }
                | SceError::KeyEnvelopePadding
                | SceError::AesCbcDecryptFailed
        )
}
