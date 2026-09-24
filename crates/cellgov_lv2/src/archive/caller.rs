//! The caller tables for PPU syscall sites in firmware: `caller.tsv`,
//! `caller_unresolved.tsv` and `reach.tsv`.

use std::collections::BTreeSet;

use super::spec::{CALLER, CALLER_UNRESOLVED, REACH};
use super::table::{self, ArchiveError, Table, NONE};

/// One module's resolved syscall sites for one ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// The module's path under `dev_flash`.
    pub module: String,
    /// The syscall ordinal the sites load.
    pub ordinal: usize,
    /// The site addresses, ascending; never empty.
    pub sites: Vec<u64>,
}

/// One scanned module and the sites whose ordinal the scan could not
/// resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerUnresolvedRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// The module's path under `dev_flash`.
    pub module: String,
    /// The unresolved site addresses, ascending; empty for a module
    /// whose every site resolved.
    pub sites: Vec<u64>,
}

/// One exported function that reaches a resolved ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachRow {
    /// Identifies the source PUP by SHA-256.
    pub pup_sha256: String,
    /// The module's path under `dev_flash`.
    pub module: String,
    /// The export's NID. A NID is 32 bits wide; the field takes the
    /// column's full width, so a held row wider than a NID round-trips
    /// unchanged.
    pub export_nid: u64,
    /// The ordinal a site inside the export loads.
    pub ordinal: usize,
}

/// The three caller tables of one census.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerCensus {
    /// `caller.tsv`.
    pub caller: Vec<CallerRow>,
    /// `caller_unresolved.tsv`: one row per scanned module.
    pub unresolved: Vec<CallerUnresolvedRow>,
    /// `reach.tsv`.
    pub reach: Vec<ReachRow>,
}

impl CallerCensus {
    /// Keep the rows of `existing` that this census did not rescan.
    ///
    /// A PUP counts as rescanned when this census holds a row for it in
    /// any of the three tables. An existing row survives when its PUP is
    /// in `valid_pups` and was not rescanned.
    pub fn merge_existing(&mut self, existing: CallerCensus, valid_pups: &BTreeSet<&str>) {
        let rescanned: BTreeSet<String> = self
            .unresolved
            .iter()
            .map(|row| &row.pup_sha256)
            .chain(self.caller.iter().map(|row| &row.pup_sha256))
            .chain(self.reach.iter().map(|row| &row.pup_sha256))
            .cloned()
            .collect();
        let keep = |pup: &str| valid_pups.contains(pup) && !rescanned.contains(pup);
        self.caller.extend(
            existing
                .caller
                .into_iter()
                .filter(|row| keep(&row.pup_sha256)),
        );
        self.unresolved.extend(
            existing
                .unresolved
                .into_iter()
                .filter(|row| keep(&row.pup_sha256)),
        );
        self.reach.extend(
            existing
                .reach
                .into_iter()
                .filter(|row| keep(&row.pup_sha256)),
        );
    }
}

/// The typed rows of a parsed `caller.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`CALLER`].
pub fn caller_rows(table: &Table) -> Vec<CallerRow> {
    debug_assert_eq!(table.spec.name, CALLER.name);
    table
        .rows
        .iter()
        .map(|row| CallerRow {
            pup_sha256: row[0].clone(),
            module: row[1].clone(),
            ordinal: parse_usize(&row[2]),
            sites: parse_list(&row[3]),
        })
        .collect()
}

/// The typed rows of a parsed `caller_unresolved.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`CALLER_UNRESOLVED`].
pub fn caller_unresolved_rows(table: &Table) -> Vec<CallerUnresolvedRow> {
    debug_assert_eq!(table.spec.name, CALLER_UNRESOLVED.name);
    table
        .rows
        .iter()
        .map(|row| CallerUnresolvedRow {
            pup_sha256: row[0].clone(),
            module: row[1].clone(),
            sites: if row[2] == NONE {
                Vec::new()
            } else {
                parse_list(&row[2])
            },
        })
        .collect()
}

/// The typed rows of a parsed `reach.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`REACH`].
pub fn reach_rows(table: &Table) -> Vec<ReachRow> {
    debug_assert_eq!(table.spec.name, REACH.name);
    table
        .rows
        .iter()
        .map(|row| ReachRow {
            pup_sha256: row[0].clone(),
            module: row[1].clone(),
            export_nid: row[2]
                .parse()
                .expect("the archive parser checked the NID integer"),
            ordinal: parse_usize(&row[3]),
        })
        .collect()
}

/// Canonicalizes caller rows by PUP, module and ordinal.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn caller_tsv(rows: &[CallerRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&CallerRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.pup_sha256, &a.module, a.ordinal).cmp(&(&b.pup_sha256, &b.module, b.ordinal))
    });
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                row.module.clone(),
                row.ordinal.to_string(),
                list_cell(&row.sites),
            ]
        })
        .collect();
    table::render(&CALLER, &cells)
}

/// Canonicalizes unresolved rows by PUP and module.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn caller_unresolved_tsv(rows: &[CallerUnresolvedRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&CallerUnresolvedRow> = rows.iter().collect();
    sorted.sort_by(|a, b| (&a.pup_sha256, &a.module).cmp(&(&b.pup_sha256, &b.module)));
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                row.module.clone(),
                if row.sites.is_empty() {
                    NONE.to_string()
                } else {
                    list_cell(&row.sites)
                },
            ]
        })
        .collect();
    table::render(&CALLER_UNRESOLVED, &cells)
}

/// Canonicalizes reach rows by PUP, module, export NID and ordinal.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn reach_tsv(rows: &[ReachRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&ReachRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.pup_sha256, &a.module, a.export_nid, a.ordinal).cmp(&(
            &b.pup_sha256,
            &b.module,
            b.export_nid,
            b.ordinal,
        ))
    });
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                row.module.clone(),
                row.export_nid.to_string(),
                row.ordinal.to_string(),
            ]
        })
        .collect();
    table::render(&REACH, &cells)
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

fn parse_usize(value: &str) -> usize {
    value
        .parse()
        .expect("the archive parser checked the decimal integer")
}

#[cfg(test)]
#[path = "tests/caller_tests.rs"]
mod tests;
