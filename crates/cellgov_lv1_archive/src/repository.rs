//! The LV1 repository tables: each logged kernel boot's repository reads,
//! and the one row per node that joins the node table with those reads.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_lv1::RepositoryRead;
use cellgov_ps3_abi::lv1::repository::{AbsentNode, RepositoryKey, RepositoryNode, SecondValue};

use cellgov_lv2_archive::{self as archive, ArchiveError, PupRow, Table, NONE};

use super::spec::{REPOSITORY, REPOSITORY_READ, REPOSITORY_UNOBSERVED};

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
    /// The value a read returns in r5, or `None` when the source does not
    /// observe it.
    pub v2: Option<u64>,
    /// The firmware versions CellGov serves the node's values to,
    /// ascending. The logged boot of each one reads the node. The source
    /// can be of another firmware: the psdevwiki dump is of 3.41.
    pub fw: Vec<String>,
    /// The class of the values' source.
    pub source: &'static str,
    /// The locator of the source.
    pub citation: &'static str,
    /// The part of the source that holds the values.
    pub section: Option<String>,
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
    /// The node table holds a node for a firmware whose logged boot does
    /// not read it.
    #[error(
        "the node table holds {key} for firmware {fw}, whose logged kernel boot does not read it"
    )]
    UnreadNode {
        /// The node.
        key: RepositoryKey,
        /// The firmware the node names.
        fw: String,
    },
    /// The absent-key table holds a key for a firmware whose logged boot
    /// does not read it.
    #[error(
        "the absent-key table holds {key} for firmware {fw}, whose logged kernel boot does not \
         read it"
    )]
    UnreadAbsentKey {
        /// The key.
        key: RepositoryKey,
        /// The firmware the entry names.
        fw: String,
    },
    /// A logged read's state disagrees with the node table: the log is
    /// stale.
    #[error(
        "repository_read.tsv logs {key} as {} for PUP {pup_sha256}, and the node table {} for \
         that PUP's firmware; re-run the logged boots",
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
    /// A logged refusal of a node whose second value no source observes.
    /// The hypervisor answers that node only at the read sites it lists,
    /// so the refusal is not by itself a stale log.
    #[error(
        "repository_read.tsv logs {key} as refused for PUP {pup_sha256}; the node table serves \
         it at that PUP's firmware only at the read sites it lists, so the read's site is not \
         one of them, or the log is stale"
    )]
    RefusedAtUnlistedSite {
        /// The PUP of the boot.
        pup_sha256: String,
        /// The node.
        key: RepositoryKey,
    },
}

/// A key that one boot's reads both answered and refused, which one
/// `repository_read.tsv` row cannot hold.
///
/// The hypervisor answers an unobserved second value at some read sites
/// and refuses it at others, so two reads of one key can disagree.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the boot answered read {answered_at} of {key} and refused read {refused_at}; \
     repository_read.tsv holds one state per key"
)]
pub struct MixedRepositoryRead {
    /// The key.
    pub key: RepositoryKey,
    /// The position of a read of the key that the hypervisor answered.
    pub answered_at: usize,
    /// The position of a read of the key that the hypervisor refused.
    pub refused_at: usize,
}

/// One row per key that `reads`, one boot's reads in order, names.
///
/// # Errors
///
/// Returns [`MixedRepositoryRead`] for a key that the hypervisor answers
/// at one read and refuses at another.
pub fn repository_read_rows_of(
    pup_sha256: &str,
    reads: &[RepositoryRead],
) -> Result<Vec<RepositoryReadRow>, MixedRepositoryRead> {
    let mut rows: BTreeMap<RepositoryKey, RepositoryReadRow> = BTreeMap::new();
    for (at, read) in reads.iter().enumerate() {
        let answered = read.values.is_some();
        let row = rows.entry(read.key).or_insert_with(|| RepositoryReadRow {
            pup_sha256: pup_sha256.to_string(),
            key: read.key,
            node: read.key.path(),
            first: at,
            answered,
        });
        if row.answered != answered {
            let (answered_at, refused_at) = if answered {
                (at, row.first)
            } else {
                (row.first, at)
            };
            return Err(MixedRepositoryRead {
                key: read.key,
                answered_at,
                refused_at,
            });
        }
    }
    Ok(rows.into_values().collect())
}

/// One row per node of `nodes`, with the firmware it names, ascending by
/// key and then by firmware.
///
/// The hypervisor answers a key of `absent` with lv1's no-node status, so
/// an answered read of that key needs no row of `nodes`.
///
/// # Errors
///
/// Returns the first [`RepositoryTableError`]:
/// - a read row that names a PUP `pups` does not hold;
/// - a read whose state disagrees with `nodes` and `absent` at the read's
///   firmware;
/// - a refused read of a node whose second value no source observes;
/// - an absent key that names a firmware whose read rows do not name it;
/// - a node that names a firmware whose read rows do not name it.
pub fn repository_rows(
    nodes: &[RepositoryNode],
    absent: &[AbsentNode],
    reads: &[RepositoryReadRow],
    pups: &[PupRow],
) -> Result<Vec<RepositoryRow>, RepositoryTableError> {
    let fw_of: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let mut read_at: BTreeSet<(RepositoryKey, &str)> = BTreeSet::new();
    for read in reads {
        let fw = fw_of.get(read.pup_sha256.as_str()).ok_or_else(|| {
            RepositoryTableError::UnknownPup {
                pup_sha256: read.pup_sha256.clone(),
            }
        })?;
        let served = nodes
            .iter()
            .find(|node| node.key == read.key && node.firmware.contains(fw));
        // crates/cellgov_lv1 `read_node` refuses an unobserved
        // second value at every read site the node does not list.
        if let (
            false,
            Some(RepositoryNode {
                v2: SecondValue::Unobserved { .. },
                ..
            }),
        ) = (read.answered, served)
        {
            return Err(RepositoryTableError::RefusedAtUnlistedSite {
                pup_sha256: read.pup_sha256.clone(),
                key: read.key,
            });
        }
        let shown_absent = absent
            .iter()
            .any(|absent| absent.key == read.key && absent.firmware.contains(fw));
        if read.answered != (served.is_some() || shown_absent) {
            return Err(RepositoryTableError::StaleRead {
                pup_sha256: read.pup_sha256.clone(),
                key: read.key,
                answered: read.answered,
            });
        }
        read_at.insert((read.key, fw));
    }
    let unread_absent = absent.iter().find_map(|absent| {
        absent
            .firmware
            .iter()
            .find(|fw| !read_at.contains(&(absent.key, **fw)))
            .map(|fw| (absent.key, fw))
    });
    if let Some((key, fw)) = unread_absent {
        return Err(RepositoryTableError::UnreadAbsentKey {
            key,
            fw: (*fw).to_string(),
        });
    }
    let mut rows = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(fw) = node
            .firmware
            .iter()
            .find(|fw| !read_at.contains(&(node.key, **fw)))
        {
            return Err(RepositoryTableError::UnreadNode {
                key: node.key,
                fw: (*fw).to_string(),
            });
        }
        rows.push(RepositoryRow {
            key: node.key,
            node: node.key.path(),
            v1: node.v1,
            v2: match node.v2 {
                SecondValue::Given(v2) => Some(v2),
                SecondValue::Unobserved { .. } => None,
            },
            fw: node.firmware.iter().map(|fw| (*fw).to_string()).collect(),
            source: node.source.class(),
            citation: node.source.citation(),
            section: node.source.section(),
        });
    }
    rows.sort_by(|a, b| (a.key, &a.fw).cmp(&(b.key, &b.fw)));
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
    archive::render(&REPOSITORY_READ, &cells)
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
            cells.push(
                row.v2
                    .map_or_else(|| REPOSITORY_UNOBSERVED.to_string(), hex),
            );
            cells.push(row.fw.join(","));
            cells.push(row.source.to_string());
            cells.push(row.citation.to_string());
            cells.push(row.section.as_deref().unwrap_or(NONE).to_string());
            cells
        })
        .collect();
    archive::render(&REPOSITORY, &cells)
}

/// The typed rows of a parsed `repository_read.tsv`.
///
/// # Errors
///
/// Returns the [`archive::checked_rows`] refusal when `table` does not
/// hold the columns and the cell kinds of [`REPOSITORY_READ`].
pub fn repository_read_rows(table: &Table) -> Result<Vec<RepositoryReadRow>, ArchiveError> {
    archive::checked_rows(&REPOSITORY_READ, table)?
        .iter()
        .map(|row| {
            Ok(RepositoryReadRow {
                pup_sha256: row.text(0)?.to_string(),
                key: RepositoryKey {
                    lpar: row.integer(1)?,
                    n1: row.hex64(2)?,
                    n2: row.hex64(3)?,
                    n3: row.hex64(4)?,
                    n4: row.hex64(5)?,
                },
                node: row.text(6)?.to_string(),
                first: row.integer(7)?,
                answered: row.text(8)? == "answered",
            })
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

#[cfg(test)]
#[path = "tests/repository_unobserved_tests.rs"]
mod unobserved_tests;

#[cfg(test)]
#[path = "tests/repository_absent_tests.rs"]
mod absent_tests;
