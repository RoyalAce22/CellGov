//! Owns the `pup.tsv` rows that identify the archive's installed firmware.

use super::firmware::is_date;
use super::spec::PUP;
use super::table::{ArchiveError, Table, NONE};
use cellgov_ps3_abi::format::pup::PUP_HEADER_SIZE;

/// Describes one `pup.tsv` entry for a PUP image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PupRow {
    /// Covers all bytes of the PUP file.
    pub pup_sha256: String,
    /// References the matching `firmware.tsv` row.
    pub fw: String,
    /// Includes the fixed PUP header.
    pub size_bytes: u64,
    /// Uses the PUP-header word's zero-padded hexadecimal form.
    pub image_version: String,
    /// Uses a general provenance label without a URL.
    pub source_note: String,
    /// Uses `None` if the source gives no acquisition date.
    pub acquired: Option<String>,
}

/// Decodes rows after loader validation against [`PUP`].
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`PUP`].
pub fn pup_rows(table: &Table) -> Vec<PupRow> {
    assert_eq!(table.spec.name, PUP.name, "not a PUP table");
    table
        .rows
        .iter()
        .map(|cells| PupRow {
            pup_sha256: cells[0].clone(),
            fw: cells[1].clone(),
            size_bytes: cells[2]
                .parse()
                .unwrap_or_else(|_| panic!("the loader passed {:?} as an integer", cells[2])),
            image_version: cells[3].clone(),
            source_note: cells[4].clone(),
            acquired: (cells[5] != NONE).then(|| cells[5].clone()),
        })
        .collect()
}

/// Why a decoded `pup.tsv` row is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PupTableError {
    /// The primary key is not a lowercase SHA-256 digest.
    #[error("pup.tsv row {pup_sha256:?}: pup_sha256 is not 64 lowercase hexadecimal digits")]
    BadSha256 {
        /// Contains the malformed digest.
        pup_sha256: String,
    },
    /// The PUP byte length is zero.
    #[error("pup.tsv row {pup_sha256:?}: size_bytes is zero")]
    ZeroSize {
        /// Identifies the invalid row.
        pup_sha256: String,
    },
    /// The PUP byte length cannot hold the fixed header.
    #[error(
        "pup.tsv row {pup_sha256:?}: size_bytes {size_bytes} is smaller than the fixed PUP header"
    )]
    TooSmall {
        /// Identifies the invalid row.
        pup_sha256: String,
        /// Contains the byte length below the fixed header size.
        size_bytes: u64,
    },
    /// The image version is not a zero-padded PUP-header word.
    #[error("pup.tsv row {pup_sha256:?}: image_version {image_version:?} is not 0x plus 16 lowercase hexadecimal digits")]
    BadImageVersion {
        /// Identifies the invalid row.
        pup_sha256: String,
        /// Contains the malformed image version.
        image_version: String,
    },
    /// The acquisition date is not a Gregorian calendar day.
    #[error("pup.tsv row {pup_sha256:?}: acquired date {acquired:?} is not YYYY-MM-DD")]
    BadAcquired {
        /// Identifies the invalid row.
        pup_sha256: String,
        /// Contains the malformed acquisition date.
        acquired: String,
    },
}

fn is_lower_hex(text: &str, digits: usize) -> bool {
    text.len() == digits
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Checks the PUP-row invariants that the generic loader cannot verify.
///
/// An empty table is valid.
///
/// # Errors
///
/// Returns the first [`PupTableError`] for an invalid row.
pub fn check_pup_rows(rows: &[PupRow]) -> Result<(), PupTableError> {
    for row in rows {
        if !is_lower_hex(&row.pup_sha256, 64) {
            return Err(PupTableError::BadSha256 {
                pup_sha256: row.pup_sha256.clone(),
            });
        }
        if row.size_bytes == 0 {
            return Err(PupTableError::ZeroSize {
                pup_sha256: row.pup_sha256.clone(),
            });
        }
        // The shared PUP parser refuses a buffer shorter than its fixed header.
        if row.size_bytes < PUP_HEADER_SIZE as u64 {
            return Err(PupTableError::TooSmall {
                pup_sha256: row.pup_sha256.clone(),
                size_bytes: row.size_bytes,
            });
        }
        let image_digits = row.image_version.strip_prefix("0x").unwrap_or_default();
        if !is_lower_hex(image_digits, 16) {
            return Err(PupTableError::BadImageVersion {
                pup_sha256: row.pup_sha256.clone(),
                image_version: row.image_version.clone(),
            });
        }
        if let Some(acquired) = &row.acquired {
            if !is_date(acquired) {
                return Err(PupTableError::BadAcquired {
                    pup_sha256: row.pup_sha256.clone(),
                    acquired: acquired.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Why a `pup.tsv` text is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PupTsvError {
    /// The text breaks the table's schema.
    #[error(transparent)]
    Parse(ArchiveError),
    /// A row breaks a PUP-row invariant.
    #[error(transparent)]
    Rows(PupTableError),
}

/// The rows of a `pup.tsv` text, parsed and checked.
///
/// # Errors
///
/// [`PupTsvError`] when the text does not parse or a row breaks
/// [`check_pup_rows`].
pub fn checked_pup_rows(text: &str) -> Result<Vec<PupRow>, PupTsvError> {
    let table = super::table::parse(&PUP, text).map_err(PupTsvError::Parse)?;
    let rows = pup_rows(&table);
    check_pup_rows(&rows).map_err(PupTsvError::Rows)?;
    Ok(rows)
}

#[cfg(test)]
#[path = "tests/pup_tests.rs"]
mod tests;
