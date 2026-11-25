//! The LV1 repository tables: each logged kernel boot's repository reads,
//! and the one row per node that joins the node table with those reads.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_lv2::hypervisor::RepositoryRead;
use cellgov_ps3_abi::lv1::repository::{RepositoryKey, RepositoryNode};

use super::pup::PupRow;
use super::spec::{REPOSITORY, REPOSITORY_READ};
use super::table::{self, ArchiveError, Table, NONE};

/// One logged kernel boot's reads of one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryReadRow {
    /// Identifies the PUP whose kernel the boot ran, by SHA-256.
    pub pup_sha256: String,
    /// The partition and the raw key words of the read.
    pub key: RepositoryKey,
    /// The key words as text, from [`RepositoryKey::path`].
    pub node: String,
    /// The position of the first read of the key among the boot's reads,
    /// from 0.
    pub first: usize,
    /// Whether CellGov's hypervisor answered the read or refused it.
    pub answered: bool,
}

/// One node CellGov's hypervisor serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRow {
    /// The partition and the raw key words of the node.
    pub key: RepositoryKey,
    /// The key words as text, from [`RepositoryKey::path`].
    pub node: String,
    /// The value a read returns in r4.
    pub v1: u64,
    /// The value a read returns in r5.
    pub v2: u64,
    /// The firmware versions whose logged boot reads the node, ascending.
    pub fw: Vec<String>,
    /// The class of the values' source.
    pub source: &'static str,
    /// The locator of the source.
    pub citation: &'static str,
    /// The part of the source that holds the values.
    pub section: Option<&'static str>,
}

/// Why the node table and the logged reads disagree.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryTableError {
    /// A read row names a PUP that `pup.tsv` does not hold.
    #[error("repository_read.tsv names PUP {pup_sha256}, which pup.tsv does not hold")]
    UnknownPup {
        /// The PUP the row names.
        pup_sha256: String,
    },
    /// The node table holds a node that no logged boot reads.
    #[error("the node table holds {key}, which no logged kernel boot reads")]
    UnreadNode {
        /// The node.
        key: RepositoryKey,
    },
    /// A logged read's state disagrees with the node table: the log is
    /// stale.
    #[error(
        "repository_read.tsv logs {key} as {} for PUP {pup_sha256}, and the node table {}; \
         re-run the logged boots",
        if *answered { "answered" } else { "refused" },
        if *answered { "holds no row for it" } else { "now serves it" }
    )]
    StaleRead {
        /// The PUP of the boot.
        pup_sha256: String,
        /// The node.
        key: RepositoryKey,
        /// Whether the log says the hypervisor answered the read.
        answered: bool,
    },
}

/// One row per key that `reads`, one boot's reads in order, names.
pub fn repository_read_rows_of(
    pup_sha256: &str,
    reads: &[RepositoryRead],
) -> Vec<RepositoryReadRow> {
    let mut rows: BTreeMap<RepositoryKey, RepositoryReadRow> = BTreeMap::new();
    for (at, read) in reads.iter().enumerate() {
        rows.entry(read.key).or_insert_with(|| RepositoryReadRow {
            pup_sha256: pup_sha256.to_string(),
            key: read.key,
            node: read.key.path(),
            first: at,
            answered: read.values.is_some(),
        });
    }
    rows.into_values().collect()
}

/// One row per node of `nodes`, with the firmware whose logged boot reads
/// it, ascending by key.
///
/// # Errors
///
/// Returns the first [`RepositoryTableError`]:
/// - a read row that names a PUP `pups` does not hold;
/// - a read whose state disagrees with `nodes`;
/// - a node that no read row names.
pub fn repository_rows(
    nodes: &[RepositoryNode],
    reads: &[RepositoryReadRow],
    pups: &[PupRow],
) -> Result<Vec<RepositoryRow>, RepositoryTableError> {
    let fw_of: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let served: BTreeSet<RepositoryKey> = nodes.iter().map(|node| node.key).collect();
    let mut readers: BTreeMap<RepositoryKey, BTreeSet<&str>> = BTreeMap::new();
    for read in reads {
        let fw = fw_of.get(read.pup_sha256.as_str()).ok_or_else(|| {
            RepositoryTableError::UnknownPup {
                pup_sha256: read.pup_sha256.clone(),
            }
        })?;
        if read.answered != served.contains(&read.key) {
            return Err(RepositoryTableError::StaleRead {
                pup_sha256: read.pup_sha256.clone(),
                key: read.key,
                answered: read.answered,
            });
        }
        readers.entry(read.key).or_default().insert(fw);
    }
    let mut rows = Vec::with_capacity(nodes.len());
    for node in nodes {
        let fw = readers
            .get(&node.key)
            .ok_or(RepositoryTableError::UnreadNode { key: node.key })?;
        rows.push(RepositoryRow {
            key: node.key,
            node: node.key.path(),
            v1: node.v1,
            v2: node.v2,
            fw: fw.iter().map(|fw| (*fw).to_string()).collect(),
            source: node.source.class(),
            citation: node.source.citation(),
            section: node.source.section(),
        });
    }
    rows.sort_by_key(|row| row.key);
    Ok(rows)
}

fn hex(value: u64) -> String {
    format!("{value:#018x}")
}

fn key_cells(key: &RepositoryKey) -> [String; 5] {
    [
        key.lpar.to_string(),
        hex(key.n1),
        hex(key.n2),
        hex(key.n3),
        hex(key.n4),
    ]
}

/// Renders `repository_read.tsv`, sorted by PUP and then by key.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn repository_read_tsv(rows: &[RepositoryReadRow]) -> Result<String, ArchiveError> {
    let mut sorted: Vec<&RepositoryReadRow> = rows.iter().collect();
    sorted.sort_by(|a, b| (&a.pup_sha256, a.key).cmp(&(&b.pup_sha256, b.key)));
    let cells: Vec<Vec<String>> = sorted
        .into_iter()
        .map(|row| {
            let mut cells = vec![row.pup_sha256.clone()];
            cells.extend(key_cells(&row.key));
            cells.push(row.node.clone());
            cells.push(row.first.to_string());
            cells.push(if row.answered { "answered" } else { "refused" }.to_string());
            cells
        })
        .collect();
    table::render(&REPOSITORY_READ, &cells)
}

/// Renders `repository.tsv`.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn repository_tsv(rows: &[RepositoryRow]) -> Result<String, ArchiveError> {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            let mut cells = key_cells(&row.key).to_vec();
            cells.push(row.node.clone());
            cells.push(hex(row.v1));
            cells.push(hex(row.v2));
            cells.push(row.fw.join(","));
            cells.push(row.source.to_string());
            cells.push(row.citation.to_string());
            cells.push(row.section.unwrap_or(NONE).to_string());
            cells
        })
        .collect();
    table::render(&REPOSITORY, &cells)
}

/// The typed rows of a parsed `repository_read.tsv`.
///
/// # Panics
///
/// Panics unless the loader parsed `table` with [`REPOSITORY_READ`].
pub fn repository_read_rows(table: &Table) -> Vec<RepositoryReadRow> {
    debug_assert_eq!(table.spec.name, REPOSITORY_READ.name);
    let word = |cell: &str| {
        u64::from_str_radix(&cell[2..], 16).expect("the archive parser checked the hex word")
    };
    table
        .rows
        .iter()
        .map(|row| RepositoryReadRow {
            pup_sha256: row[0].clone(),
            key: RepositoryKey {
                lpar: row[1]
                    .parse()
                    .expect("the archive parser checked the decimal integer"),
                n1: word(&row[2]),
                n2: word(&row[3]),
                n3: word(&row[4]),
                n4: word(&row[5]),
            },
            node: row[6].clone(),
            first: row[7]
                .parse()
                .expect("the archive parser checked the decimal integer"),
            answered: row[8] == "answered",
        })
        .collect()
}

/// Replaces every row of `pup_sha256` in `rows` with `fresh`, the rows of
/// one new boot of that PUP's kernel.
pub fn replace_repository_reads(
    rows: &mut Vec<RepositoryReadRow>,
    pup_sha256: &str,
    fresh: Vec<RepositoryReadRow>,
) {
    rows.retain(|row| row.pup_sha256 != pup_sha256);
    rows.extend(fresh);
}

#[cfg(test)]
#[path = "tests/repository_tests.rs"]
mod tests;
