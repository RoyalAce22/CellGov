//! The committed `pup.tsv` and `kernel.tsv`, compiled in once for every
//! command that reads them.

use cellgov_install::manifest::{sha256_of, Sha256};
use cellgov_lv2_archive::{self as archive, PupRow, PupTsvError};

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/pup.tsv"
));

/// The committed `kernel.tsv`, which holds the SHA-256 of each PUP's
/// decrypted kernel ELF.
pub(crate) const KERNEL_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/kernel.tsv"
));

/// The compiled-in `pup.tsv` does not parse or breaks a row invariant.
#[derive(Debug, thiserror::Error)]
#[error("compiled pup.tsv: {0}")]
pub(crate) struct CommittedPupError(#[from] PupTsvError);

/// The compiled-in `pup.tsv` rows, checked.
///
/// # Errors
///
/// [`CommittedPupError`] when the committed table is unusable.
pub(crate) fn committed_pup_rows() -> Result<Vec<PupRow>, CommittedPupError> {
    Ok(archive::checked_pup_rows(PUP_TSV)?)
}

/// Why a kernel ELF is not the kernel `kernel.tsv` names for a PUP.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommittedKernelError {
    #[error("parse compiled kernel.tsv: {source}")]
    KernelTable {
        #[source]
        source: archive::ArchiveError,
    },
    #[error(transparent)]
    Verdict(#[from] archive::KernelElfError),
}

/// Refuses `elf` unless `kernel.tsv` names it as the kernel of
/// `pup_sha256`.
///
/// This keeps a PUP's rows to its own kernel.
///
/// # Errors
///
/// Returns the [`CommittedKernelError`] that applies.
pub(crate) fn check_committed_kernel(
    pup_sha256: &str,
    elf: &[u8],
) -> Result<(), CommittedKernelError> {
    let kernels = archive::parse(&archive::KERNEL, KERNEL_TSV)
        .and_then(|table| archive::kernel_rows(&table))
        .map_err(|source| CommittedKernelError::KernelTable { source })?;
    let elf_sha256 = Sha256(sha256_of(elf)).to_hex();
    Ok(archive::check_kernel_elf(
        &kernels,
        pup_sha256,
        &elf_sha256,
    )?)
}
