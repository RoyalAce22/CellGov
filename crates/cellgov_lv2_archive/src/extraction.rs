//! The rules that fold one PUP's extracted kernel rows into the archive.
//!
//! `cellgov_ppu` classifies a kernel; the caller maps that
//! classification into the row types in [`super::census`] and hands them
//! here. The archive does not depend on `cellgov_ppu`, so the caller
//! does the mapping. The archive's own encodings of a classification,
//! such as a gate's `reads` cell, are [`selector_slot_name`] and
//! [`control_flags1_read`].
//!
//! The caller supplies the SHA-256 function every digest uses.

use std::collections::{BTreeMap, BTreeSet};

use super::census::{gate_tsv, subentry_tsv, GateRow, KernelRow, StubRow, SubentryRow};
use super::pup::PupRow;
use super::spec::{CAPABILITY_GATE, KERNEL, SUBENTRY};
use super::table::ArchiveError;

/// The extracted kernel rows the archive holds, across every PUP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedRows {
    /// `kernel.tsv`.
    pub kernels: Vec<KernelRow>,
    /// `stub.tsv`.
    pub stubs: Vec<StubRow>,
    /// `subentry.tsv`.
    pub subentries: Vec<SubentryRow>,
    /// `gate.tsv`.
    pub gates: Vec<GateRow>,
}

/// One PUP's freshly extracted kernel rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PupExtraction {
    /// The kernel row, its digests included.
    pub kernel: KernelRow,
    /// The kernel's stub rows.
    pub stubs: Vec<StubRow>,
    /// The kernel's subentry rows.
    pub subentries: Vec<SubentryRow>,
    /// The kernel's capability-gate rows.
    pub gates: Vec<GateRow>,
}

/// Why the archive refuses an extraction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractionError {
    /// The PUP is not recorded in `pup.tsv`.
    #[error("PUP {pup_sha256} is not recorded in pup.tsv")]
    UnknownPup {
        /// The requested PUP digest.
        pup_sha256: String,
    },
    /// `pup.tsv` records the PUP under another firmware version.
    #[error("PUP {pup_sha256} belongs to firmware {recorded}, not {requested}")]
    FirmwareMismatch {
        /// The requested PUP digest.
        pup_sha256: String,
        /// The version `pup.tsv` records.
        recorded: String,
        /// The version the caller named.
        requested: String,
    },
    /// Two kernel rows of one firmware version record different census digests.
    #[error("firmware {fw} kernel rows disagree on their census digest")]
    DigestConflict {
        /// The firmware version.
        fw: String,
    },
    /// Re-extraction of a PUP with held rows produced different rows.
    #[error("PUP {pup_sha256} re-extracted different kernel or stub rows")]
    ExtractionConflict {
        /// The PUP digest.
        pup_sha256: String,
    },
    /// Re-extraction of a PUP with held rows read a different kernel image.
    #[error(
        "PUP {pup_sha256} re-extracted kernel digest {extracted}, not its recorded digest {recorded}"
    )]
    KernelDigestConflict {
        /// The PUP digest.
        pup_sha256: String,
        /// The kernel digest the archive records.
        recorded: String,
        /// The kernel digest just extracted.
        extracted: String,
    },
    /// A row names a PUP that `pup.tsv` does not record.
    #[error("existing {table}.tsv row names PUP {pup_sha256}, which is absent from pup.tsv")]
    ExistingPupReference {
        /// The table holding the row.
        table: &'static str,
        /// The PUP digest the row names.
        pup_sha256: String,
    },
    /// An extracted row names a PUP with no kernel row.
    #[error("existing extracted row names PUP {pup_sha256}, which is absent from kernel.tsv")]
    ExistingKernelReference {
        /// The PUP digest the row names.
        pup_sha256: String,
    },
    /// A PUP's gate rows no longer hash to the digest its kernel row records.
    #[error(
        "existing gate.tsv rows for PUP {pup_sha256} hash to {extracted}, not kernel.tsv digest {recorded}"
    )]
    ExistingGateDigest {
        /// The PUP digest.
        pup_sha256: String,
        /// The gate digest the kernel row records.
        recorded: String,
        /// The digest of the gate rows as held.
        extracted: String,
    },
    /// The version's census file holds other text, and the caller did not ask for a replacement.
    #[error("firmware {fw} census differs from the existing census file")]
    CensusConflict {
        /// The firmware version.
        fw: String,
    },
    /// A table did not render, so its digest is unknown.
    #[error("render {table}: {source}")]
    Render {
        /// The table.
        table: &'static str,
        /// Why the render failed.
        #[source]
        source: ArchiveError,
    },
}

/// The selector argument's register name for a zero-based argument slot:
/// slot 0 is `r3`.
pub fn selector_slot_name(slot: usize) -> String {
    format!("r{}", slot + 3)
}

/// A gate row's `reads` cell for a gate that tests `ctrl_flags1`
/// against `mask`.
pub fn control_flags1_read(mask: u32) -> String {
    format!("ctrl_flags1_0x{mask:08x}")
}

/// A kernel row's `gate_sha256`: the digest of the PUP's gate rows as
/// `gate.tsv` renders them.
///
/// # Errors
///
/// [`ExtractionError::Render`].
pub fn gate_digest(
    gates: &[GateRow],
    sha256_hex: &dyn Fn(&[u8]) -> String,
) -> Result<String, ExtractionError> {
    let text = gate_tsv(gates).map_err(|source| ExtractionError::Render {
        table: CAPABILITY_GATE.name,
        source,
    })?;
    Ok(sha256_hex(text.as_bytes()))
}

/// A kernel row's `subentry_sha256`: the digest of the PUP's subentry
/// rows as `subentry.tsv` renders them.
///
/// # Errors
///
/// [`ExtractionError::Render`].
pub fn subentry_digest(
    subentries: &[SubentryRow],
    sha256_hex: &dyn Fn(&[u8]) -> String,
) -> Result<String, ExtractionError> {
    let text = subentry_tsv(subentries).map_err(|source| ExtractionError::Render {
        table: SUBENTRY.name,
        source,
    })?;
    Ok(sha256_hex(text.as_bytes()))
}

/// The `pup.tsv` row for `pup_sha256`, held to the firmware version the
/// caller names.
///
/// # Errors
///
/// [`ExtractionError::UnknownPup`] or [`ExtractionError::FirmwareMismatch`].
pub fn select_pup<'a>(
    pups: &'a [PupRow],
    pup_sha256: &str,
    fw: &str,
) -> Result<&'a PupRow, ExtractionError> {
    let pup = pups
        .iter()
        .find(|row| row.pup_sha256 == pup_sha256)
        .ok_or_else(|| ExtractionError::UnknownPup {
            pup_sha256: pup_sha256.to_string(),
        })?;
    if pup.fw != fw {
        return Err(ExtractionError::FirmwareMismatch {
            pup_sha256: pup_sha256.to_string(),
            recorded: pup.fw.clone(),
            requested: fw.to_string(),
        });
    }
    Ok(pup)
}

/// Check the held rows before a merge.
///
/// - Every kernel row names a PUP in `pup.tsv`.
/// - Every stub, subentry and gate row names a PUP with a kernel row.
/// - Each PUP's gate rows, as rendered, hash to its kernel row's gate
///   digest.
///
/// # Errors
///
/// [`ExtractionError::ExistingPupReference`],
/// [`ExtractionError::ExistingKernelReference`],
/// [`ExtractionError::ExistingGateDigest`], or
/// [`ExtractionError::Render`].
pub fn validate_existing(
    existing: &ExtractedRows,
    pups: &[PupRow],
    sha256_hex: &dyn Fn(&[u8]) -> String,
) -> Result<(), ExtractionError> {
    let valid_pups: BTreeSet<&str> = pups.iter().map(|row| row.pup_sha256.as_str()).collect();
    let kernel_pups: BTreeSet<&str> = existing
        .kernels
        .iter()
        .map(|row| row.pup_sha256.as_str())
        .collect();
    if let Some(kernel) = existing
        .kernels
        .iter()
        .find(|row| !valid_pups.contains(row.pup_sha256.as_str()))
    {
        return Err(ExtractionError::ExistingPupReference {
            table: KERNEL.name,
            pup_sha256: kernel.pup_sha256.clone(),
        });
    }
    let orphan = existing
        .stubs
        .iter()
        .map(|row| &row.pup_sha256)
        .chain(existing.subentries.iter().map(|row| &row.pup_sha256))
        .chain(existing.gates.iter().map(|row| &row.pup_sha256))
        .find(|pup| !kernel_pups.contains(pup.as_str()));
    if let Some(pup_sha256) = orphan {
        return Err(ExtractionError::ExistingKernelReference {
            pup_sha256: pup_sha256.clone(),
        });
    }
    for kernel in &existing.kernels {
        let pup_gates: Vec<GateRow> = existing
            .gates
            .iter()
            .filter(|row| row.pup_sha256 == kernel.pup_sha256)
            .cloned()
            .collect();
        let extracted = gate_digest(&pup_gates, sha256_hex)?;
        if extracted != kernel.gate_sha256 {
            return Err(ExtractionError::ExistingGateDigest {
                pup_sha256: kernel.pup_sha256.clone(),
                recorded: kernel.gate_sha256.clone(),
                extracted,
            });
        }
    }
    Ok(())
}

/// Refuse a re-extraction of a PUP whose held rows disagree with it.
///
/// It refuses a different kernel image whatever `allow_movement` says.
/// When `allow_movement` is false, it also refuses any other change to
/// the kernel row or to the stub, subentry or gate rows.
///
/// # Errors
///
/// [`ExtractionError::KernelDigestConflict`] or
/// [`ExtractionError::ExtractionConflict`].
fn refuse_extraction_conflict(
    existing: &ExtractedRows,
    new: &PupExtraction,
    allow_movement: bool,
) -> Result<(), ExtractionError> {
    let pup_sha256 = new.kernel.pup_sha256.as_str();
    let Some(existing_kernel) = existing
        .kernels
        .iter()
        .find(|row| row.pup_sha256 == pup_sha256)
    else {
        return Ok(());
    };
    if existing_kernel.kernel_elf_sha256 != new.kernel.kernel_elf_sha256 {
        return Err(ExtractionError::KernelDigestConflict {
            pup_sha256: pup_sha256.to_string(),
            recorded: existing_kernel.kernel_elf_sha256.clone(),
            extracted: new.kernel.kernel_elf_sha256.clone(),
        });
    }
    if allow_movement {
        return Ok(());
    }
    let mut existing_stubs: Vec<&StubRow> = existing
        .stubs
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_stubs.sort_by_key(|row| row.descriptor);
    let mut replacement_stubs: Vec<&StubRow> = new.stubs.iter().collect();
    replacement_stubs.sort_by_key(|row| row.descriptor);
    let mut existing_subentries: Vec<&SubentryRow> = existing
        .subentries
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_subentries.sort_by_key(|row| (row.ordinal, row.packet));
    let mut replacement_subentries: Vec<&SubentryRow> = new.subentries.iter().collect();
    replacement_subentries.sort_by_key(|row| (row.ordinal, row.packet));
    let mut existing_gates: Vec<&GateRow> = existing
        .gates
        .iter()
        .filter(|row| row.pup_sha256 == pup_sha256)
        .collect();
    existing_gates.sort_by_key(|row| row.ordinal);
    let mut replacement_gates: Vec<&GateRow> = new.gates.iter().collect();
    replacement_gates.sort_by_key(|row| row.ordinal);
    if *existing_kernel != new.kernel
        || existing_stubs != replacement_stubs
        || existing_subentries != replacement_subentries
        || existing_gates != replacement_gates
    {
        return Err(ExtractionError::ExtractionConflict {
            pup_sha256: pup_sha256.to_string(),
        });
    }
    Ok(())
}

/// Remove the rows of every PUP `pup.tsv` records under `fw`, and
/// return how many kernel rows it removed for PUPs other than
/// `selected_pup`.
fn remove_version_rows(
    existing: &mut ExtractedRows,
    fw: &str,
    selected_pup: &str,
    pups: &[PupRow],
) -> usize {
    let replaced_pups: BTreeSet<&str> = pups
        .iter()
        .filter(|row| row.fw == fw)
        .map(|row| row.pup_sha256.as_str())
        .collect();
    let removed = existing
        .kernels
        .iter()
        .filter(|row| {
            row.pup_sha256 != selected_pup && replaced_pups.contains(row.pup_sha256.as_str())
        })
        .count();
    existing.retain_pups(|pup| !replaced_pups.contains(pup));
    removed
}

/// Refuse kernel rows of one firmware version that record different
/// census digests.
///
/// # Errors
///
/// [`ExtractionError::ExistingPupReference`] for a kernel row whose PUP
/// `pup.tsv` does not record, or [`ExtractionError::DigestConflict`].
fn refuse_digest_conflict(kernels: &[KernelRow], pups: &[PupRow]) -> Result<(), ExtractionError> {
    let firmware_by_pup: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let mut digest_by_firmware = BTreeMap::new();
    for kernel in kernels {
        let fw = firmware_by_pup
            .get(kernel.pup_sha256.as_str())
            .ok_or_else(|| ExtractionError::ExistingPupReference {
                table: KERNEL.name,
                pup_sha256: kernel.pup_sha256.clone(),
            })?;
        if digest_by_firmware
            .insert(*fw, kernel.census_sha256.as_str())
            .is_some_and(|digest| digest != kernel.census_sha256)
        {
            return Err(ExtractionError::DigestConflict {
                fw: (*fw).to_string(),
            });
        }
    }
    Ok(())
}

/// Fold one PUP's extraction into `existing` and report how many other
/// same-version kernel rows a replacement removed.
///
/// In order: `refuse_extraction_conflict`; with `replace_version`,
/// `remove_version_rows`; the new rows replace the PUP's old rows; then
/// `refuse_digest_conflict` over the result. On a refusal, `existing`
/// can hold part of the fold: the caller discards it and writes
/// nothing.
///
/// # Errors
///
/// Any refusal of those rules.
pub fn merge_extraction(
    existing: &mut ExtractedRows,
    new: PupExtraction,
    fw: &str,
    pups: &[PupRow],
    replace_version: bool,
) -> Result<usize, ExtractionError> {
    refuse_extraction_conflict(existing, &new, replace_version)?;
    let pup_sha256 = new.kernel.pup_sha256.clone();
    let removed = if replace_version {
        remove_version_rows(existing, fw, &pup_sha256, pups)
    } else {
        0
    };
    existing.retain_pups(|pup| pup != pup_sha256);
    existing.kernels.push(new.kernel);
    existing.stubs.extend(new.stubs);
    existing.subentries.extend(new.subentries);
    existing.gates.extend(new.gates);
    refuse_digest_conflict(&existing.kernels, pups)?;
    Ok(removed)
}

/// Whether the caller writes the version's census file.
///
/// - No file: `true`.
/// - With `replace_version`: `true`.
/// - A file with the same text: `false`.
/// - A file with other text: refused.
///
/// # Errors
///
/// [`ExtractionError::CensusConflict`].
pub fn census_needs_write(
    existing: Option<&str>,
    census: &str,
    fw: &str,
    replace_version: bool,
) -> Result<bool, ExtractionError> {
    match existing {
        None => Ok(true),
        Some(_) if replace_version => Ok(true),
        Some(text) if text == census => Ok(false),
        Some(_) => Err(ExtractionError::CensusConflict { fw: fw.to_string() }),
    }
}

impl ExtractedRows {
    fn retain_pups(&mut self, keep: impl Fn(&str) -> bool) {
        self.kernels.retain(|row| keep(&row.pup_sha256));
        self.stubs.retain(|row| keep(&row.pup_sha256));
        self.subentries.retain(|row| keep(&row.pup_sha256));
        self.gates.retain(|row| keep(&row.pup_sha256));
    }
}

#[cfg(test)]
#[path = "tests/extraction_tests.rs"]
mod tests;
