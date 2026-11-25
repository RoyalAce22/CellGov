//! The LV1 hypercall tables: each scanned kernel's hypercall sites, and
//! the one row per number that joins them with the call table and
//! CellGov's hypervisor.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::lv1::call::{
    self, LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_FLUSH, LOGICAL_CONSOLE_SEND,
};

use super::extraction::ExtractionError;
use super::pup::PupRow;
use super::spec::{HYPERCALL, HYPERCALL_SITE, HYPERCALL_UNRESOLVED};
use super::table::{self, ArchiveError, Table, NONE};

/// One kernel's sites of one hypercall number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypercallSiteRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// The constant the sites load into r11.
    pub number: u64,
    /// The real addresses of the `sc` instructions, ascending; never
    /// empty. Real mode ignores EA bits 0 and 1, so a kernel address
    /// `0x8000_0000_xxxx_xxxx` is the real address `0x0000_0000_xxxx_xxxx`.
    ///
    /// [CBE-Handbook p:98 s:4.2.8] Real mode ignores the EA bits above the real address range when it forms the RA.
    pub sites: Vec<u64>,
}

/// One scanned kernel, and the sites whose number the scan could not
/// prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypercallUnresolvedRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// The real addresses of the unresolved sites, ascending; empty when
    /// every site resolved.
    pub sites: Vec<u64>,
}

/// The two hypercall site tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HypercallCensus {
    /// `hypercall_site.tsv`.
    pub sites: Vec<HypercallSiteRow>,
    /// `hypercall_unresolved.tsv`: one row per scanned kernel.
    pub unresolved: Vec<HypercallUnresolvedRow>,
}

impl HypercallCensus {
    /// Replace every row of `pup_sha256` with `scan`, the rows of one
    /// fresh scan of that PUP's kernel.
    pub fn replace_pup(&mut self, pup_sha256: &str, scan: HypercallCensus) {
        self.sites.retain(|row| row.pup_sha256 != pup_sha256);
        self.unresolved.retain(|row| row.pup_sha256 != pup_sha256);
        self.sites.extend(scan.sites);
        self.unresolved.extend(scan.unresolved);
    }
}

/// Whether CellGov's hypervisor answers a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypercallState {
    /// A handler answers it.
    Answered,
    /// The null backend refuses it and stops the run.
    NullBackend,
}

impl HypercallState {
    /// The label `hypercall.tsv` uses.
    pub fn label(self) -> &'static str {
        match self {
            HypercallState::Answered => "answered",
            HypercallState::NullBackend => "null_backend",
        }
    }
}

/// One hypercall number: its call-table row, its family and the work
/// item that owns it, the kernels that call it, and CellGov's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypercallRow {
    /// The constant a site loads into r11.
    pub number: u64,
    /// The name `lv1call.h` gives it; `None` when neither copy of that
    /// header lists it ([`call::lookup`]).
    pub name: Option<&'static str>,
    /// Input registers from r3, when the call table lists the number.
    pub inputs: Option<u8>,
    /// Output registers from r4, when the call table lists the number.
    pub outputs: Option<u8>,
    /// The family [`hypercall_family`] puts it in.
    pub family: &'static str,
    /// The work item that answers the family.
    pub leaf: &'static str,
    /// The firmware versions whose kernel calls it, ascending.
    pub fw: Vec<String>,
    /// The most sites one kernel of the newest version in `fw` holds; 0
    /// when `fw` is empty.
    pub sites: usize,
    /// Whether CellGov's hypervisor answers it.
    pub state: HypercallState,
}

/// The family and owning work item of hypercall `number`.
///
/// The family is CellGov's grouping, read from the name `lv1call.h`
/// gives the number. The headers name 105, 107 and 109 only
/// `undocumented_function_N`; `cellgov_ps3_abi` names them the logical
/// console calls, so they form the console family.
pub fn hypercall_family(number: u64) -> (&'static str, &'static str) {
    if [
        LOGICAL_CONSOLE_CONSTRUCT,
        LOGICAL_CONSOLE_SEND,
        LOGICAL_CONSOLE_FLUSH,
    ]
    .contains(&number)
    {
        return ("console", "dispatch");
    }
    let Some(row) = call::lookup(number) else {
        return ("unnamed", "boot");
    };
    let name = row.name;
    let words: Vec<&str> = name.split('_').collect();
    let has = |part: &str| name.contains(part);
    if name.starts_with("undocumented_function") {
        ("undocumented", "boot")
    } else if words.contains(&"lpm") {
        // The performance monitor's calls name events and buses of
        // their own; none of them is an interrupt or a device call.
        ("other", "boot")
    } else if has("repository") {
        ("repository", "repository")
    } else if words.contains(&"spe") || words.contains(&"spes") {
        ("logical_spe", "logical_spe")
    } else if [
        "htab",
        "virtual_address_space",
        "memory",
        "address_region",
        "iopte",
    ]
    .iter()
    .any(|p| has(p))
    {
        ("memory", "translation")
    } else if ["irq", "interrupt", "event"].iter().any(|p| has(p)) {
        ("interrupt", "interrupt_delivery")
    } else if has("logical_partition") || has("ppe_id") {
        ("partition", "dispatch")
    } else if ["device", "storage", "bus", "gpu", "dma", "io_"]
        .iter()
        .any(|p| has(p))
    {
        ("device", "boot")
    } else {
        ("other", "boot")
    }
}

/// One row per number that a scanned kernel calls or CellGov's
/// hypervisor answers, ascending by number.
///
/// # Errors
///
/// [`ExtractionError::ExistingPupReference`] when a site row names a PUP
/// that `pups` does not hold.
pub fn hypercall_rows(
    sites: &[HypercallSiteRow],
    pups: &[PupRow],
) -> Result<Vec<HypercallRow>, ExtractionError> {
    let fw_of: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    // number -> fw -> the most sites one PUP of that version holds.
    let mut called: BTreeMap<u64, BTreeMap<&str, usize>> = BTreeMap::new();
    for row in sites {
        let fw = fw_of.get(row.pup_sha256.as_str()).ok_or_else(|| {
            ExtractionError::ExistingPupReference {
                table: HYPERCALL_SITE.name,
                pup_sha256: row.pup_sha256.clone(),
            }
        })?;
        let count = called.entry(row.number).or_default().entry(fw).or_default();
        *count = (*count).max(row.sites.len());
    }
    let numbers: BTreeSet<u64> = called
        .keys()
        .copied()
        .chain(cellgov_lv2::hypervisor::ANSWERED.iter().copied())
        .collect();
    Ok(numbers
        .into_iter()
        .map(|number| {
            let entry = call::lookup(number);
            let (family, leaf) = hypercall_family(number);
            let by_fw = called.get(&number);
            HypercallRow {
                number,
                name: entry.map(|e| e.name),
                inputs: entry.map(|e| e.inputs),
                outputs: entry.map(|e| e.outputs),
                family,
                leaf,
                fw: by_fw
                    .map(|m| m.keys().map(|fw| (*fw).to_string()).collect())
                    .unwrap_or_default(),
                sites: by_fw
                    .and_then(|m| m.values().next_back().copied())
                    .unwrap_or(0),
                state: if cellgov_lv2::hypervisor::answers(number) {
                    HypercallState::Answered
                } else {
                    HypercallState::NullBackend
                },
            }
        })
        .collect())
}

/// Renders `hypercall.tsv`.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn hypercall_tsv(rows: &[HypercallRow]) -> Result<String, ArchiveError> {
    let optional = |value: Option<String>| value.unwrap_or_else(|| NONE.to_string());
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.number.to_string(),
                optional(row.name.map(str::to_string)),
                optional(row.inputs.map(|n| n.to_string())),
                optional(row.outputs.map(|n| n.to_string())),
                row.family.to_string(),
                row.leaf.to_string(),
                if row.fw.is_empty() {
                    NONE.to_string()
                } else {
                    row.fw.join(",")
                },
                row.sites.to_string(),
                row.state.label().to_string(),
            ]
        })
        .collect();
    table::render(&HYPERCALL, &cells)
}

/// The typed rows of a parsed `hypercall_site.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`HYPERCALL_SITE`].
pub fn hypercall_site_rows(table: &Table) -> Vec<HypercallSiteRow> {
    debug_assert_eq!(table.spec.name, HYPERCALL_SITE.name);
    table
        .rows
        .iter()
        .map(|row| HypercallSiteRow {
            pup_sha256: row[0].clone(),
            number: row[1]
                .parse()
                .expect("the archive parser checked the decimal integer"),
            sites: parse_list(&row[2]),
        })
        .collect()
}

/// The typed rows of a parsed `hypercall_unresolved.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`HYPERCALL_UNRESOLVED`].
pub fn hypercall_unresolved_rows(table: &Table) -> Vec<HypercallUnresolvedRow> {
    debug_assert_eq!(table.spec.name, HYPERCALL_UNRESOLVED.name);
    table
        .rows
        .iter()
        .map(|row| HypercallUnresolvedRow {
            pup_sha256: row[0].clone(),
            sites: if row[1] == NONE {
                Vec::new()
            } else {
                parse_list(&row[1])
            },
        })
        .collect()
}

/// Renders `hypercall_site.tsv`, sorted by PUP and then by number.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn hypercall_site_tsv(rows: &[HypercallSiteRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&HypercallSiteRow> = rows.iter().collect();
    sorted.sort_by(|a, b| (&a.pup_sha256, a.number).cmp(&(&b.pup_sha256, b.number)));
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                row.number.to_string(),
                list_cell(&row.sites),
            ]
        })
        .collect();
    table::render(&HYPERCALL_SITE, &cells)
}

/// Renders `hypercall_unresolved.tsv`, sorted by PUP.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn hypercall_unresolved_tsv(rows: &[HypercallUnresolvedRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&HypercallUnresolvedRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.pup_sha256.cmp(&b.pup_sha256));
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                if row.sites.is_empty() {
                    NONE.to_string()
                } else {
                    list_cell(&row.sites)
                },
            ]
        })
        .collect();
    table::render(&HYPERCALL_UNRESOLVED, &cells)
}

fn list_cell(values: &[u64]) -> String {
    values
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_list(cell: &str) -> Vec<u64> {
    cell.split(',')
        .map(|value| {
            value
                .parse()
                .expect("the archive parser checked the integer list")
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/hypercall_tests.rs"]
mod tests;
