//! The committed `pup.tsv`, compiled in once for every command that
//! reads it.

use cellgov_lv2_archive::{self as archive, PupRow, PupTsvError};

const PUP_TSV: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/lv2/tables/pup.tsv"
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
