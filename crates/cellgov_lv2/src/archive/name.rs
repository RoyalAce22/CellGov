//! The rows of `name.tsv`, the `cellgov` rows rendered from the
//! `lv2_syscalls!` macro, and the conflicts among the sources.
//!
//! Every row names its source, and an ordinal no source names has no
//! row. Where sources disagree, every candidate stays: [`conflict_rows`]
//! reports the disagreement and resolves nothing.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::lv2::syscall::{
    Lv2Syscall, ALL_LV2_SYSCALLS, ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS,
};

use super::spec::{CONFLICTS, NAME};
use super::table::{render, ArchiveError, Table, NONE};

/// The psdevwiki page every `psdevwiki` row refers to.
pub const PSDEVWIKI_PAGE: &str = "https://www.psdevwiki.com/ps3/LV2_Functions_and_Syscalls";

/// The PSL1GHT header every `psl1ght` row refers to, by the path
/// under the toolchain's PPU include root.
pub const PSL1GHT_HEADER: &str = "ppu/include/lv2/syscalls.h";

/// The prefix of the token a `psl1ght` reference names in the header.
pub const PSL1GHT_TOKEN_PREFIX: &str = "SYSCALL_";

/// The path every `cellgov` reference names a constant under.
pub const CELLGOV_CONSTANT_PATH: &str = "cellgov_ps3_abi::lv2::syscall::";

/// Where a name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameSource {
    /// psdevwiki's LV2 Functions and Syscalls page.
    Psdevwiki,
    /// The PSL1GHT `SYSCALL_` tokens, lowercased under a `sys_` prefix.
    Psl1ght,
    /// The name field of the `lv2_syscalls!` macro.
    Cellgov,
    /// Material with no public citation.
    NonPublic,
}

impl NameSource {
    /// Every source, in the order the archive document lists them.
    pub const ALL: &[NameSource] = &[
        NameSource::Psdevwiki,
        NameSource::Psl1ght,
        NameSource::Cellgov,
        NameSource::NonPublic,
    ];

    /// The stable label `name.tsv` carries.
    pub fn label(self) -> &'static str {
        match self {
            NameSource::Psdevwiki => "psdevwiki",
            NameSource::Psl1ght => "psl1ght",
            NameSource::Cellgov => "cellgov",
            NameSource::NonPublic => "non_public",
        }
    }

    /// The source with `label`, if any.
    pub fn from_label(label: &str) -> Option<NameSource> {
        NameSource::ALL.iter().copied().find(|s| s.label() == label)
    }

    /// One-line meaning of the source, as the archive document states it.
    pub fn meaning(self) -> &'static str {
        match self {
            NameSource::Psdevwiki => {
                "The name cell of psdevwiki's LV2 Functions and Syscalls table, taken only where it is one plain identifier and not a stub ending in `_`; `ref` is the page."
            }
            NameSource::Psl1ght => {
                "`sys_` and the lowercased rest of a `SYSCALL_` token in PSL1GHT's `lv2/syscalls.h`; `ref` is the header and the token."
            }
            NameSource::Cellgov => {
                "The name field of the `lv2_syscalls!` macro; `ref` is the constant. CellGov's own vocabulary, rendered, never hand-copied."
            }
            NameSource::NonPublic => {
                "A name known only from material that cannot be cited; `ref` is `none`."
            }
        }
    }
}

/// One row of `name.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRow {
    /// The syscall number.
    pub ordinal: u64,
    /// The packet the name applies to; `None` for the whole ordinal.
    pub packet: Option<String>,
    /// The name.
    pub name: String,
    /// Who gives it.
    pub source: NameSource,
    /// Where in the source, as the source's meaning states.
    pub reference: Option<String>,
    /// The first firmware the source gives the name for.
    pub fw_from: Option<String>,
    /// The last firmware the source gives the name for.
    pub fw_to: Option<String>,
}

impl NameRow {
    /// The row's key, ordered as the loader orders it.
    fn key(&self) -> (u64, &str, &'static str, &str) {
        (
            self.ordinal,
            self.packet.as_deref().unwrap_or(NONE),
            self.source.label(),
            &self.name,
        )
    }

    /// Whether `reference` and `name` have the shape the source's
    /// meaning promises.
    pub fn fits_source(&self) -> bool {
        match (self.source, self.reference.as_deref()) {
            // A cell the wiki left as a stub (`sys_`, `sys_trace_`) is
            // a plain identifier and still no name.
            (NameSource::Psdevwiki, Some(reference)) => {
                reference == PSDEVWIKI_PAGE && !self.name.ends_with('_')
            }
            (NameSource::Psl1ght, Some(reference)) => {
                let Some((header, token)) = reference.rsplit_once(':') else {
                    return false;
                };
                header == PSL1GHT_HEADER && psl1ght_name(token).as_deref() == Some(&self.name)
            }
            (NameSource::Cellgov, Some(reference)) => reference
                .strip_prefix(CELLGOV_CONSTANT_PATH)
                .is_some_and(|constant| {
                    !constant.is_empty()
                        && constant
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                }),
            (NameSource::NonPublic, None) => true,
            _ => false,
        }
    }

    fn cells(&self) -> Vec<String> {
        let cell = |value: &Option<String>| value.clone().unwrap_or_else(|| NONE.to_string());
        vec![
            self.ordinal.to_string(),
            cell(&self.packet),
            self.name.clone(),
            self.source.label().to_string(),
            cell(&self.reference),
            cell(&self.fw_from),
            cell(&self.fw_to),
        ]
    }
}

/// The name a PSL1GHT `SYSCALL_` token stands for: `sys_` and the
/// lowercased rest.
///
/// `None` for a token the header could not spell:
/// - without the `SYSCALL_` prefix;
/// - with a character outside uppercase letters, digits and `_`.
pub fn psl1ght_name(token: &str) -> Option<String> {
    let rest = token.strip_prefix(PSL1GHT_TOKEN_PREFIX)?;
    let in_alphabet = |b: u8| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_';
    (!rest.is_empty() && rest.bytes().all(in_alphabet))
        .then(|| format!("sys_{}", rest.to_ascii_lowercase()))
}

/// The `cellgov` rows: one per named entry of the `lv2_syscalls!`
/// lists, sorted by key.
pub fn macro_name_rows() -> Vec<NameRow> {
    let mut rows: Vec<NameRow> = ALL_LV2_SYSCALLS
        .iter()
        .chain(ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS)
        .filter_map(|entry: &Lv2Syscall| {
            entry.name.map(|name| NameRow {
                ordinal: entry.number,
                packet: None,
                name: name.to_string(),
                source: NameSource::Cellgov,
                reference: Some(format!("{CELLGOV_CONSTANT_PATH}{}", entry.constant)),
                fw_from: None,
                fw_to: None,
            })
        })
        .collect();
    sort(&mut rows);
    rows
}

/// The rows of a parsed `name.tsv`.
///
/// # Panics
///
/// When `table` was not parsed under [`NAME`]: the loader then
/// checked nothing this reading relies on.
pub fn name_rows(table: &Table) -> Vec<NameRow> {
    assert_eq!(table.spec.name, NAME.name, "not a name table");
    let optional = |cell: &String| (cell != NONE).then(|| cell.clone());
    table
        .rows
        .iter()
        .map(|cells| NameRow {
            ordinal: cells[0]
                .parse()
                .unwrap_or_else(|_| panic!("the loader passed {:?} as an integer", cells[0])),
            packet: optional(&cells[1]),
            name: cells[2].clone(),
            source: NameSource::from_label(&cells[3])
                .unwrap_or_else(|| panic!("the loader passed {:?} as a source", cells[3])),
            reference: optional(&cells[4]),
            fw_from: optional(&cells[5]),
            fw_to: optional(&cells[6]),
        })
        .collect()
}

/// `rows` with its `cellgov` rows replaced by [`macro_name_rows`],
/// sorted by key.
pub fn with_cellgov_rows(rows: &[NameRow]) -> Vec<NameRow> {
    let mut out: Vec<NameRow> = rows
        .iter()
        .filter(|row| row.source != NameSource::Cellgov)
        .cloned()
        .collect();
    out.extend(macro_name_rows());
    sort(&mut out);
    out
}

fn sort(rows: &mut [NameRow]) {
    rows.sort_by(|a, b| a.key().cmp(&b.key()));
}

/// The `name.tsv` text for `rows`.
///
/// # Errors
///
/// Whatever [`render`] refuses in the rendered rows.
pub fn name_tsv(rows: &[NameRow]) -> Result<String, ArchiveError> {
    let cells: Vec<Vec<String>> = rows.iter().map(NameRow::cells).collect();
    render(&NAME, &cells)
}

/// How the names of one ordinal differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disagreement {
    /// One identifier under different leading underscores.
    Spelling,
    /// Different identifiers.
    Name,
}

impl Disagreement {
    /// Every disagreement, in the order the archive document lists them.
    pub const ALL: &[Disagreement] = &[Disagreement::Spelling, Disagreement::Name];

    /// The stable label `conflicts.tsv` carries.
    pub fn label(self) -> &'static str {
        match self {
            Disagreement::Spelling => "spelling",
            Disagreement::Name => "name",
        }
    }
}

/// `name` without its leading underscores: what a spelling disagreement leaves.
pub fn spelling(name: &str) -> &str {
    name.trim_start_matches('_')
}

/// One row of `conflicts.tsv`: a [`NameRow`] whose ordinal and packet
/// carry more than one distinct name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRow {
    /// The syscall number.
    pub ordinal: u64,
    /// The packet, or `None` for the whole ordinal.
    pub packet: Option<String>,
    /// The name.
    pub name: String,
    /// Who gives it.
    pub source: NameSource,
    /// How the names of this ordinal and packet differ.
    pub disagreement: Disagreement,
}

/// The rows of `names` whose ordinal and packet carry more than one
/// distinct name, in the order of `names`.
pub fn conflict_rows(names: &[NameRow]) -> Vec<ConflictRow> {
    let mut by_slot: BTreeMap<(u64, Option<&str>), BTreeSet<&str>> = BTreeMap::new();
    for row in names {
        by_slot
            .entry((row.ordinal, row.packet.as_deref()))
            .or_default()
            .insert(&row.name);
    }
    names
        .iter()
        .filter_map(|row| {
            let distinct = &by_slot[&(row.ordinal, row.packet.as_deref())];
            if distinct.len() < 2 {
                return None;
            }
            let spellings: BTreeSet<&str> = distinct.iter().map(|n| spelling(n)).collect();
            Some(ConflictRow {
                ordinal: row.ordinal,
                packet: row.packet.clone(),
                name: row.name.clone(),
                source: row.source,
                disagreement: if spellings.len() == 1 {
                    Disagreement::Spelling
                } else {
                    Disagreement::Name
                },
            })
        })
        .collect()
}

/// The rows of `names` whose ordinal and packet only the `cellgov` source names.
pub fn uncorroborated(names: &[NameRow]) -> Vec<&NameRow> {
    let mut sources: BTreeMap<(u64, Option<&str>), BTreeSet<NameSource>> = BTreeMap::new();
    for row in names {
        sources
            .entry((row.ordinal, row.packet.as_deref()))
            .or_default()
            .insert(row.source);
    }
    names
        .iter()
        .filter(|row| {
            sources[&(row.ordinal, row.packet.as_deref())]
                .iter()
                .all(|s| *s == NameSource::Cellgov)
        })
        .collect()
}

/// The `conflicts.tsv` text for `rows`.
///
/// # Errors
///
/// Whatever [`render`] refuses in the rendered rows.
pub fn conflicts_tsv(rows: &[ConflictRow]) -> Result<String, ArchiveError> {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.ordinal.to_string(),
                row.packet.clone().unwrap_or_else(|| NONE.to_string()),
                row.name.clone(),
                row.source.label().to_string(),
                row.disagreement.label().to_string(),
            ]
        })
        .collect();
    render(&CONFLICTS, &cells)
}

#[cfg(test)]
#[path = "tests/name_tests.rs"]
mod tests;
