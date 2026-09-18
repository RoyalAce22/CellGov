//! The decrypt of the LV2 kernel a firmware entry stores, and the
//! coverage state one attempt yields.
//!
//! Keys differ by firmware version, so a run over the store succeeds in
//! part by construction. The state names why a version yielded no ELF;
//! it never files a missing key as a failed decrypt, or a failed
//! decrypt as a missing key.

use std::path::{Path, PathBuf};

use crate::keys::{version_label, KeyVault};
use crate::manifest::{sha256_of, Sha256};
use crate::sce::{self, SceError};
use crate::store::record::KernelRecord;

/// A kernel the vault opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecryptedKernel {
    /// The plaintext ELF.
    pub elf: Vec<u8>,
    /// The version word of the kernel's program identification header.
    pub version: u64,
}

/// Why a stored kernel yielded no ELF.
#[derive(Debug, thiserror::Error)]
pub enum KernelDecryptError {
    /// The stored file could not be read.
    #[error("read {}: {source}", path.display())]
    Read {
        /// The stored kernel.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The decrypt refused.
    #[error("{source}")]
    Decrypt {
        /// The kernel's version word, when its header was readable.
        version: Option<u64>,
        /// The refusal.
        #[source]
        source: SceError,
    },
}

/// Decrypt the kernel `kernel` names under `entry_dir` with `keys`.
///
/// # Errors
///
/// - [`KernelDecryptError::Read`]: the stored file cannot be read.
/// - [`KernelDecryptError::Decrypt`]: every refusal of
///   [`sce::decrypt_self_to_elf`].
pub fn decrypt_stored_kernel(
    entry_dir: &Path,
    kernel: &KernelRecord,
    keys: &KeyVault,
) -> Result<DecryptedKernel, KernelDecryptError> {
    // The record gate proved the path stays inside the entry.
    let path = kernel
        .path
        .split('/')
        .fold(entry_dir.to_path_buf(), |dir, part| dir.join(part));
    let raw = std::fs::read(&path).map_err(|source| KernelDecryptError::Read { path, source })?;
    let version = sce::parse_program_identification(&raw)
        .ok()
        .map(|program| program.version);
    let elf = sce::decrypt_self_to_elf(&raw, keys)
        .map_err(|source| KernelDecryptError::Decrypt { version, source })?;
    let version = version.unwrap_or_else(|| {
        unreachable!("the decrypt parsed the program identification header before this")
    });
    Ok(DecryptedKernel { elf, version })
}

/// Whether a decrypt refusal names a key the vault lacks.
///
/// A padding or CBC refusal comes only from the envelope stage, where
/// the candidate keyset is the one thing under test.
#[must_use]
pub fn is_key_gap(e: &SceError) -> bool {
    matches!(
        e,
        SceError::Keys(_)
            | SceError::NoAppKey { .. }
            | SceError::NoNpdrmKey { .. }
            | SceError::NoLv2Key { .. }
            | SceError::NoCandidateOpensEnvelope { .. }
            | SceError::KeyEnvelopePadding
            | SceError::AesCbcDecryptFailed
    )
}

/// How one kernel came out of a coverage run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelCoverage {
    /// The vault opened it.
    Decrypted {
        /// The version word of the kernel's program identification header.
        version: u64,
        /// Plaintext ELF length.
        elf_len: usize,
        /// SHA-256 over the plaintext ELF.
        elf_sha256: Sha256,
    },
    /// The vault holds no keyset that opens it.
    NoKey {
        /// The kernel's version word, when its header was readable.
        version: Option<u64>,
        /// The key the vault lacks.
        missing: String,
    },
    /// The stored file could not be read.
    Unreadable {
        /// The read refusal.
        reason: String,
    },
    /// The vault opened the envelope, or the container never reached
    /// it, and no ELF came out.
    Failed {
        /// The kernel's version word, when its header was readable.
        version: Option<u64>,
        /// The refusal.
        reason: String,
    },
}

impl KernelCoverage {
    /// The state of one decrypt attempt.
    #[must_use]
    pub fn of(result: Result<DecryptedKernel, KernelDecryptError>) -> Self {
        match result {
            Ok(kernel) => KernelCoverage::Decrypted {
                version: kernel.version,
                elf_len: kernel.elf.len(),
                elf_sha256: Sha256(sha256_of(&kernel.elf)),
            },
            Err(KernelDecryptError::Read { path, source }) => KernelCoverage::Unreadable {
                reason: format!("read {}: {source}", path.display()),
            },
            Err(KernelDecryptError::Decrypt { version, source }) if is_key_gap(&source) => {
                KernelCoverage::NoKey {
                    version,
                    missing: missing_key(version, &source),
                }
            }
            Err(KernelDecryptError::Decrypt { version, source }) => KernelCoverage::Failed {
                version,
                reason: source.to_string(),
            },
        }
    }

    /// The state's stable label, as the coverage table spells it.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            KernelCoverage::Decrypted { .. } => "decrypted",
            KernelCoverage::NoKey { .. } => "no_key",
            KernelCoverage::Unreadable { .. } => "unreadable",
            KernelCoverage::Failed { .. } => "failed",
        }
    }
}

/// The key a refusal says the vault lacks, named by the firmware the
/// kernel's header carries.
fn missing_key(version: Option<u64>, refusal: &SceError) -> String {
    let firmware = version.map_or_else(
        || "an unreadable version".to_string(),
        |v| format!("firmware {}", version_label(v)),
    );
    match refusal {
        SceError::NoLv2Key { .. } => {
            format!("an LV2 keyset for {firmware} (the vault holds none)")
        }
        SceError::NoCandidateOpensEnvelope { tried, class, .. } => {
            format!(
                "one of the {tried} {class} keysets for {firmware} (none in the vault opens it)"
            )
        }
        SceError::KeyEnvelopePadding | SceError::AesCbcDecryptFailed => {
            format!("a keyset for {firmware} (the one candidate in the vault does not open it)")
        }
        other => format!("a keyset for {firmware}: {other}"),
    }
}

#[cfg(test)]
#[path = "tests/kernel_decrypt_tests.rs"]
mod tests;
