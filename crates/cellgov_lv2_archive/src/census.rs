//! This module defines the archive rows extracted from LV2 kernels.

use std::collections::BTreeMap;

use super::spec::{CAPABILITY_GATE, CENSUS, KERNEL, PRESENCE, STUB, SUBENTRY};
use super::table::{self, ArchiveError, Table, NONE};

/// Records the extraction class of an ordinal or packet dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CensusClass {
    /// The dispatch names a non-stub implementation.
    Implemented,
    /// The dispatch names a decoded constant-error stub.
    Stub,
    /// The dispatch has no target.
    Absent,
}

impl CensusClass {
    const fn label(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Stub => "stub",
            Self::Absent => "absent",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "implemented" => Some(Self::Implemented),
            "stub" => Some(Self::Stub),
            "absent" => Some(Self::Absent),
            _ => None,
        }
    }
}

/// Records the dispatch structure below one ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchShape {
    /// The ordinal dispatches directly to its target.
    Flat,
    /// The target dispatches through a recognized packet table.
    Subtable,
    /// The target uses a comparison chain that was not fully extracted.
    ChainIncomplete,
}

impl DispatchShape {
    const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Subtable => "subtable",
            Self::ChainIncomplete => "chain_incomplete",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "flat" => Some(Self::Flat),
            "subtable" => Some(Self::Subtable),
            "chain_incomplete" => Some(Self::ChainIncomplete),
            _ => None,
        }
    }
}

/// Links a source PUP to its kernel dispatch table and archive-row digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// Identifies the decrypted kernel ELF by SHA-256.
    pub kernel_elf_sha256: String,
    /// Gives the virtual address of the dispatch table.
    pub table_base: u64,
    /// Gives the width of one table entry in bytes.
    pub entry_width: usize,
    /// Names the stable format of a table entry.
    pub entry_format: String,
    /// Counts the table entries.
    pub entry_count: usize,
    /// Names the stable discovery method.
    pub discovery_method: String,
    /// Names the stable discovery confidence.
    pub confidence: String,
    /// Identifies the matching per-version census file by SHA-256.
    pub census_sha256: String,
    /// Identifies this PUP's canonical subentry rows by SHA-256.
    pub subentry_sha256: String,
    /// Identifies this PUP's canonical capability-gate rows by SHA-256.
    pub gate_sha256: String,
}

impl KernelRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.pup_sha256.clone(),
            self.kernel_elf_sha256.clone(),
            hex64(self.table_base),
            self.entry_width.to_string(),
            self.entry_format.clone(),
            self.entry_count.to_string(),
            self.discovery_method.clone(),
            self.confidence.clone(),
            self.census_sha256.clone(),
            self.subentry_sha256.clone(),
            self.gate_sha256.clone(),
        ]
    }
}

/// Records one constant-error stub found in a kernel dispatch table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// Gives the function-descriptor address in the dispatch table.
    pub descriptor: u64,
    /// Gives the code address from the descriptor.
    pub target: u64,
    /// Records the constant Cell error that the target returns.
    pub errno: u32,
    /// Names the Cell error symbol.
    pub errno_symbol: String,
    /// Counts the table entries that name the descriptor.
    pub references: usize,
    /// Marks the descriptor-histogram mode.
    pub primary: bool,
}

impl StubRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.pup_sha256.clone(),
            hex64(self.descriptor),
            hex64(self.target),
            format!("0x{:08x}", self.errno),
            self.errno_symbol.clone(),
            self.references.to_string(),
            if self.primary { "yes" } else { "no" }.to_string(),
        ]
    }
}

/// Records one firmware version's classification of a syscall ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CensusRow {
    /// Names the firmware version key.
    pub fw: String,
    /// Gives the zero-based syscall-table ordinal.
    pub ordinal: usize,
    /// Classifies the slot.
    pub class: CensusClass,
    /// Gives the code address, or `None` for an absent slot.
    pub target: Option<u64>,
    /// Describes the dispatch structure below the ordinal.
    pub dispatch: DispatchShape,
}

/// Summarizes every extracted version's classification of one ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceRow {
    /// Gives the zero-based syscall-table ordinal.
    pub ordinal: usize,
    /// Lists extracted versions where the ordinal is implemented.
    pub implemented_versions: Vec<String>,
    /// Lists extracted versions where the ordinal is a constant-error stub.
    pub stub_versions: Vec<String>,
    /// Lists extracted versions where the ordinal is absent.
    pub absent_versions: Vec<String>,
}

/// Why per-version census rows cannot be reduced to ordinal presence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresenceError {
    /// One version does not cover the same ordinal range as the others.
    #[error("presence reduction: firmware {fw} has {found} rows, expected {expected}")]
    EntryCount {
        /// The firmware version key.
        fw: String,
        /// The shared row count required by the reduction.
        expected: usize,
        /// The row count supplied for this version.
        found: usize,
    },
    /// A row is not at the ordinal its position requires.
    #[error("presence reduction: firmware {fw} row {index} names ordinal {found}")]
    Ordinal {
        /// The firmware version key.
        fw: String,
        /// The zero-based row position.
        index: usize,
        /// The ordinal recorded by the row.
        found: usize,
    },
    /// A row names a different firmware version than its input group.
    #[error("presence reduction: firmware {fw} ordinal {ordinal} names firmware {found}")]
    Firmware {
        /// The firmware version key used to group the rows.
        fw: String,
        /// The row's ordinal.
        ordinal: usize,
        /// The firmware version key recorded by the row.
        found: String,
    },
}

impl PresenceRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.ordinal.to_string(),
            version_list_cell(&self.implemented_versions),
            version_list_cell(&self.stub_versions),
            version_list_cell(&self.absent_versions),
        ]
    }
}

fn version_list_cell(versions: &[String]) -> String {
    if versions.is_empty() {
        NONE.to_string()
    } else {
        versions.join(",")
    }
}

/// Records one packet target extracted below an LV2 ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubentryRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// Gives the top-level LV2 ordinal.
    pub ordinal: usize,
    /// Names the selector argument (`r3` through `r10`).
    pub selector_slot: String,
    /// Gives the packet identifier.
    pub packet: u64,
    /// Classifies the packet target.
    pub class: CensusClass,
    /// Gives the packet target's code address.
    pub target: u64,
}

impl SubentryRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.pup_sha256.clone(),
            self.ordinal.to_string(),
            self.selector_slot.clone(),
            self.packet.to_string(),
            self.class.label().to_string(),
            hex64(self.target),
        ]
    }
}

/// Records the capability-gate analysis state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    /// A recognized check reads a capability value.
    Gated,
    /// A recognized permission record has no capability requirement.
    Ungated,
    /// The bounded recognizer did not classify the implementation.
    NotAnalysed,
}

impl GateState {
    /// The state's `gate.tsv` label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gated => "gated",
            Self::Ungated => "ungated",
            Self::NotAnalysed => "not_analysed",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "gated" => Some(Self::Gated),
            "ungated" => Some(Self::Ungated),
            "not_analysed" => Some(Self::NotAnalysed),
            _ => None,
        }
    }
}

/// Records one PUP's capability-gate state for an ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// Gives the top-level LV2 ordinal.
    pub ordinal: usize,
    /// Gives the bounded analysis result.
    pub state: GateState,
    /// Names the capability value read by a recognized gate.
    pub reads: Option<String>,
    /// Gives the Cell error returned when the gate fails.
    pub fail_errno: Option<u32>,
}

impl GateRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.pup_sha256.clone(),
            self.ordinal.to_string(),
            self.state.label().to_string(),
            self.reads.clone().unwrap_or_else(|| NONE.to_string()),
            self.fail_errno
                .map(hex32)
                .unwrap_or_else(|| NONE.to_string()),
        ]
    }
}

impl CensusRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.fw.clone(),
            self.ordinal.to_string(),
            self.class.label().to_string(),
            self.target.map_or_else(|| NONE.to_string(), hex64),
            self.dispatch.label().to_string(),
        ]
    }
}

/// Uses the archive path convention for one firmware version.
pub fn census_file(fw: &str) -> String {
    format!("census/fw-{fw}.tsv")
}

/// Reduces extracted per-version census rows to one presence row per ordinal.
///
/// # Errors
///
/// Returns [`PresenceError`] when the versions do not cover the same ordinal
/// range or a row disagrees with its version or ordinal position.
pub fn presence_rows(
    census_by_version: &BTreeMap<String, Vec<CensusRow>>,
) -> Result<Vec<PresenceRow>, PresenceError> {
    let entry_count = census_by_version.values().map(Vec::len).max().unwrap_or(0);
    for (fw, rows) in census_by_version {
        if rows.len() != entry_count {
            return Err(PresenceError::EntryCount {
                fw: fw.clone(),
                expected: entry_count,
                found: rows.len(),
            });
        }
        for (index, row) in rows.iter().enumerate() {
            if row.ordinal != index {
                return Err(PresenceError::Ordinal {
                    fw: fw.clone(),
                    index,
                    found: row.ordinal,
                });
            }
            if row.fw != *fw {
                return Err(PresenceError::Firmware {
                    fw: fw.clone(),
                    ordinal: row.ordinal,
                    found: row.fw.clone(),
                });
            }
        }
    }
    Ok((0..entry_count)
        .map(|ordinal| {
            let mut implemented_versions = Vec::new();
            let mut stub_versions = Vec::new();
            let mut absent_versions = Vec::new();
            for (fw, rows) in census_by_version {
                let row = &rows[ordinal];
                match row.class {
                    CensusClass::Implemented => implemented_versions.push(fw.clone()),
                    CensusClass::Stub => stub_versions.push(fw.clone()),
                    CensusClass::Absent => absent_versions.push(fw.clone()),
                }
            }
            PresenceRow {
                ordinal,
                implemented_versions,
                stub_versions,
                absent_versions,
            }
        })
        .collect())
}

/// The typed rows of a table parsed with [`KERNEL`].
///
/// # Errors
///
/// Returns the [`table::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`KERNEL`].
pub fn kernel_rows(table: &Table) -> Result<Vec<KernelRow>, ArchiveError> {
    table::checked_rows(&KERNEL, table)?
        .iter()
        .map(|row| {
            Ok(KernelRow {
                pup_sha256: row.text(0)?.to_string(),
                kernel_elf_sha256: row.text(1)?.to_string(),
                table_base: row.hex64(2)?,
                entry_width: row.integer(3)?,
                entry_format: row.text(4)?.to_string(),
                entry_count: row.integer(5)?,
                discovery_method: row.text(6)?.to_string(),
                confidence: row.text(7)?.to_string(),
                census_sha256: row.text(8)?.to_string(),
                subentry_sha256: row.text(9)?.to_string(),
                gate_sha256: row.text(10)?.to_string(),
            })
        })
        .collect()
}

/// The typed rows of a table parsed with [`STUB`].
///
/// # Errors
///
/// Returns the [`table::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`STUB`].
pub fn stub_rows(table: &Table) -> Result<Vec<StubRow>, ArchiveError> {
    table::checked_rows(&STUB, table)?
        .iter()
        .map(|row| {
            Ok(StubRow {
                pup_sha256: row.text(0)?.to_string(),
                descriptor: row.hex64(1)?,
                target: row.hex64(2)?,
                errno: row.hex32(3)?,
                errno_symbol: row.text(4)?.to_string(),
                references: row.integer(5)?,
                primary: row.text(6)? == "yes",
            })
        })
        .collect()
}

/// The typed rows of a table parsed with [`CENSUS`].
///
/// # Errors
///
/// Returns the [`table::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`CENSUS`].
pub fn census_rows(table: &Table) -> Result<Vec<CensusRow>, ArchiveError> {
    table::checked_rows(&CENSUS, table)?
        .iter()
        .map(|row| {
            Ok(CensusRow {
                fw: row.text(0)?.to_string(),
                ordinal: row.integer(1)?,
                class: row.label(2, CensusClass::from_label)?,
                target: row.optional(3)?.map(|_| row.hex64(3)).transpose()?,
                dispatch: row.label(4, DispatchShape::from_label)?,
            })
        })
        .collect()
}

/// The typed rows of a table parsed with [`SUBENTRY`].
///
/// # Errors
///
/// Returns the [`table::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`SUBENTRY`].
pub fn subentry_rows(table: &Table) -> Result<Vec<SubentryRow>, ArchiveError> {
    table::checked_rows(&SUBENTRY, table)?
        .iter()
        .map(|row| {
            Ok(SubentryRow {
                pup_sha256: row.text(0)?.to_string(),
                ordinal: row.integer(1)?,
                selector_slot: row.text(2)?.to_string(),
                packet: row.integer(3)?,
                class: row.label(4, CensusClass::from_label)?,
                target: row.hex64(5)?,
            })
        })
        .collect()
}

/// The typed rows of a table parsed with [`CAPABILITY_GATE`].
///
/// # Errors
///
/// Returns the [`table::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`CAPABILITY_GATE`].
pub fn gate_rows(table: &Table) -> Result<Vec<GateRow>, ArchiveError> {
    table::checked_rows(&CAPABILITY_GATE, table)?
        .iter()
        .map(|row| {
            Ok(GateRow {
                pup_sha256: row.text(0)?.to_string(),
                ordinal: row.integer(1)?,
                state: row.label(2, GateState::from_label)?,
                reads: row.optional(3)?.map(str::to_string),
                fail_errno: row.optional(4)?.map(|_| row.hex32(4)).transpose()?,
            })
        })
        .collect()
}

/// Canonicalizes kernel rows by source PUP.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn kernel_tsv(rows: &[KernelRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(KernelRow::cells).collect();
    cells.sort_by(|a, b| a[0].cmp(&b[0]));
    table::render(&KERNEL, &cells)
}

/// Canonicalizes stub rows by source PUP and descriptor.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn stub_tsv(rows: &[StubRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(StubRow::cells).collect();
    cells.sort_by(|a, b| a[0].cmp(&b[0]).then_with(|| a[1].cmp(&b[1])));
    table::render(&STUB, &cells)
}

/// Canonicalizes census rows by firmware version and ordinal.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn census_tsv(rows: &[CensusRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(CensusRow::cells).collect();
    cells.sort_by(|a, b| {
        a[0].cmp(&b[0])
            .then_with(|| parse_usize(&a[1]).cmp(&parse_usize(&b[1])))
    });
    table::render(&CENSUS, &cells)
}

/// Canonicalizes subentry rows by PUP, ordinal, and packet.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn subentry_tsv(rows: &[SubentryRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(SubentryRow::cells).collect();
    cells.sort_by(|a, b| {
        a[0].cmp(&b[0])
            .then_with(|| parse_usize(&a[1]).cmp(&parse_usize(&b[1])))
            .then_with(|| {
                a[3].parse::<u64>()
                    .expect("packet cell came from u64")
                    .cmp(&b[3].parse().expect("packet cell came from u64"))
            })
    });
    table::render(&SUBENTRY, &cells)
}

/// Canonicalizes capability-gate rows by PUP and ordinal.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn gate_tsv(rows: &[GateRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(GateRow::cells).collect();
    cells.sort_by(|a, b| {
        a[0].cmp(&b[0])
            .then_with(|| parse_usize(&a[1]).cmp(&parse_usize(&b[1])))
    });
    table::render(&CAPABILITY_GATE, &cells)
}

/// Canonicalizes ordinal presence rows by ordinal.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn presence_tsv(rows: &[PresenceRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(PresenceRow::cells).collect();
    cells.sort_by(|a, b| parse_usize(&a[0]).cmp(&parse_usize(&b[0])));
    table::render(&PRESENCE, &cells)
}

fn hex32(value: u32) -> String {
    format!("0x{value:08x}")
}

fn hex64(value: u64) -> String {
    format!("0x{value:016x}")
}

fn parse_usize(value: &str) -> usize {
    value
        .parse()
        .expect("the archive parser checked the decimal integer")
}

#[cfg(test)]
#[path = "tests/census_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/census_console_tests.rs"]
mod console_tests;
