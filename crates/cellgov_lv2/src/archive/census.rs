//! This module defines the archive rows extracted from LV2 kernels.

use super::spec::{CENSUS, KERNEL, STUB, SUBENTRY};
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

    fn from_label(label: &str) -> Self {
        match label {
            "implemented" => Self::Implemented,
            "stub" => Self::Stub,
            "absent" => Self::Absent,
            _ => unreachable!("the archive parser checked the census class"),
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

    fn from_label(label: &str) -> Self {
        match label {
            "flat" => Self::Flat,
            "subtable" => Self::Subtable,
            "chain_incomplete" => Self::ChainIncomplete,
            _ => unreachable!("the archive parser checked the dispatch shape"),
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

/// Requires `table` to satisfy the [`KERNEL`] spec.
pub fn kernel_rows(table: &Table) -> Vec<KernelRow> {
    debug_assert_eq!(table.spec.name, KERNEL.name);
    table
        .rows
        .iter()
        .map(|row| KernelRow {
            pup_sha256: row[0].clone(),
            kernel_elf_sha256: row[1].clone(),
            table_base: parse_hex64(&row[2]),
            entry_width: parse_usize(&row[3]),
            entry_format: row[4].clone(),
            entry_count: parse_usize(&row[5]),
            discovery_method: row[6].clone(),
            confidence: row[7].clone(),
            census_sha256: row[8].clone(),
            subentry_sha256: row[9].clone(),
        })
        .collect()
}

/// Requires `table` to satisfy the [`STUB`] spec.
pub fn stub_rows(table: &Table) -> Vec<StubRow> {
    debug_assert_eq!(table.spec.name, STUB.name);
    table
        .rows
        .iter()
        .map(|row| StubRow {
            pup_sha256: row[0].clone(),
            descriptor: parse_hex64(&row[1]),
            target: parse_hex64(&row[2]),
            errno: parse_hex32(&row[3]),
            errno_symbol: row[4].clone(),
            references: parse_usize(&row[5]),
            primary: row[6] == "yes",
        })
        .collect()
}

/// Requires `table` to satisfy the [`CENSUS`] spec.
pub fn census_rows(table: &Table) -> Vec<CensusRow> {
    debug_assert_eq!(table.spec.name, CENSUS.name);
    table
        .rows
        .iter()
        .map(|row| CensusRow {
            fw: row[0].clone(),
            ordinal: parse_usize(&row[1]),
            class: CensusClass::from_label(&row[2]),
            target: (row[3] != NONE).then(|| parse_hex64(&row[3])),
            dispatch: DispatchShape::from_label(&row[4]),
        })
        .collect()
}

/// Requires `table` to satisfy the [`SUBENTRY`] spec.
pub fn subentry_rows(table: &Table) -> Vec<SubentryRow> {
    debug_assert_eq!(table.spec.name, SUBENTRY.name);
    table
        .rows
        .iter()
        .map(|row| SubentryRow {
            pup_sha256: row[0].clone(),
            ordinal: parse_usize(&row[1]),
            selector_slot: row[2].clone(),
            packet: row[3]
                .parse()
                .expect("the archive parser checked the packet integer"),
            class: CensusClass::from_label(&row[4]),
            target: parse_hex64(&row[5]),
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

fn hex64(value: u64) -> String {
    format!("0x{value:016x}")
}

fn parse_usize(value: &str) -> usize {
    value
        .parse()
        .expect("the archive parser checked the decimal integer")
}

fn parse_hex64(value: &str) -> u64 {
    u64::from_str_radix(&value[2..], 16).expect("the archive parser checked the u64 hex value")
}

fn parse_hex32(value: &str) -> u32 {
    u32::from_str_radix(&value[2..], 16).expect("the archive parser checked the u32 hex value")
}

#[cfg(test)]
#[path = "tests/census_tests.rs"]
mod tests;
