//! One table's parser and renderer, and the rules every cell obeys.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::spec::{Column, ColumnKind, TableSpec};

/// The cell that stands for the SQL NULL in a nullable column.
pub const NONE: &str = "none";

/// Why the loader refuses a table text.
///
/// Where a variant carries them:
///
/// - `table` is the table's name
/// - `line` is the 1-based line of the text
/// - `column` is the name of the cell's column
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    /// The text does not end in a line feed.
    #[error("{table}.tsv: no final line feed")]
    MissingFinalNewline {
        /// The table.
        table: &'static str,
    },
    /// The line holds a carriage return.
    #[error("{table}.tsv line {line}: carriage return")]
    CarriageReturn {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
    },
    /// The line holds a byte outside ASCII.
    #[error("{table}.tsv line {line}: non-ASCII byte")]
    NonAscii {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
    },
    /// The header row does not name the columns.
    #[error("{table}.tsv line 1: header is {found:?}, expected {expected:?}")]
    Header {
        /// The table.
        table: &'static str,
        /// The header the spec names.
        expected: String,
        /// The header the text carries.
        found: String,
    },
    /// The row has the wrong number of cells.
    #[error("{table}.tsv line {line}: {found} cells, expected {expected}")]
    CellCount {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column count the spec names.
        expected: usize,
        /// The cell count the row carries.
        found: usize,
    },
    /// The cell is empty.
    #[error("{table}.tsv line {line} column {column}: empty cell")]
    EmptyCell {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column.
        column: &'static str,
    },
    /// The cell starts with a double quote.
    #[error("{table}.tsv line {line} column {column}: cell starts with a double quote")]
    LeadingQuote {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column.
        column: &'static str,
    },
    /// The cell reads [`NONE`] in a column that takes no null.
    #[error("{table}.tsv line {line} column {column}: none in a column that takes no null")]
    NoneRefused {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column.
        column: &'static str,
    },
    /// The cell does not have the column's kind.
    #[error("{table}.tsv line {line} column {column}: {cell:?} is not {expected}")]
    BadCell {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column.
        column: &'static str,
        /// The cell.
        cell: String,
        /// The kind the column takes.
        expected: String,
    },
    /// The row's key sorts below the previous row's.
    #[error("{table}.tsv line {line}: key sorts below the previous row's")]
    Unsorted {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
    },
    /// The row's key repeats the previous row's.
    #[error("{table}.tsv line {line}: key repeats the previous row's")]
    DuplicateKey {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
    },
    /// The cell names no row of the table the column references.
    #[error("{table}.tsv line {line} column {column}: {cell:?} names no row of {target_table}.{target_column}")]
    DanglingReference {
        /// The table.
        table: &'static str,
        /// The line.
        line: usize,
        /// The column.
        column: &'static str,
        /// The cell.
        cell: String,
        /// The table the column references.
        target_table: &'static str,
        /// The column it references.
        target_column: &'static str,
    },
    /// The table a column references is not among the loaded tables.
    #[error("{table}.tsv column {column} references {target_table}.{target_column}, which is not among the loaded tables")]
    ReferenceTargetMissing {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
        /// The table the column references.
        target_table: &'static str,
        /// The column it references.
        target_column: &'static str,
    },
    /// The referenced column is not the target table's primary key.
    #[error("{table}.tsv column {column} references {target_table}.{target_column}, which is not that table's primary key")]
    ReferenceTargetNotKey {
        /// The referencing table.
        table: &'static str,
        /// The referencing column.
        column: &'static str,
        /// The referenced table.
        target_table: &'static str,
        /// The referenced column.
        target_column: &'static str,
    },
}

/// One parsed table: the rows in file order, header excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The spec [`parse`] checked the text against.
    pub spec: &'static TableSpec,
    /// Each row's cells, in column order.
    pub rows: Vec<Vec<String>>,
}

/// A key cell: numeric for an integer column, bytewise text for every other column.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum KeyPart {
    Int(u64),
    Text(String),
}

/// The bound is SQLite's 64-bit INTEGER: `build.sql` casts the cell, and
/// a `CAST` past `i64::MAX` saturates with no refusal.
fn is_integer(cell: &str) -> bool {
    let digits = !cell.is_empty() && cell.bytes().all(|b| b.is_ascii_digit());
    digits && (cell == "0" || !cell.starts_with('0')) && cell.parse::<i64>().is_ok()
}

fn is_ident(cell: &str) -> bool {
    !cell.is_empty() && cell.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_lower_hex(cell: &str, digits: usize) -> bool {
    cell.len() == digits
        && cell
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_locator(cell: &str) -> bool {
    !cell.is_empty()
        && cell
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@+-".contains(&b))
}

fn is_ascending_integer_list(cell: &str) -> bool {
    let mut previous: Option<u64> = None;
    for item in cell.split(',') {
        let Ok(value) = item.parse::<u64>() else {
            return false;
        };
        if !is_integer(item) || previous.is_some_and(|p| p >= value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

fn is_ascending_version_list(cell: &str) -> bool {
    let mut previous: Option<&str> = None;
    for version in cell.split(',') {
        if !super::firmware::is_version_key(version)
            || previous.is_some_and(|prior| prior >= version)
        {
            return false;
        }
        previous = Some(version);
    }
    true
}

fn check_cell(
    table: &'static str,
    line: usize,
    column: &Column,
    cell: &str,
) -> Result<(), ArchiveError> {
    let name = column.name;
    if cell.is_empty() {
        return Err(ArchiveError::EmptyCell {
            table,
            line,
            column: name,
        });
    }
    if cell.starts_with('"') {
        return Err(ArchiveError::LeadingQuote {
            table,
            line,
            column: name,
        });
    }
    if cell == NONE {
        return if column.nullable {
            Ok(())
        } else {
            Err(ArchiveError::NoneRefused {
                table,
                line,
                column: name,
            })
        };
    }
    let well_formed = match column.kind {
        ColumnKind::Integer => is_integer(cell),
        ColumnKind::Ident => is_ident(cell),
        ColumnKind::IntegerList => is_ascending_integer_list(cell),
        ColumnKind::VersionList => is_ascending_version_list(cell),
        ColumnKind::Sha256 => is_lower_hex(cell, 64),
        ColumnKind::Hex32 => cell
            .strip_prefix("0x")
            .is_some_and(|digits| is_lower_hex(digits, 8)),
        ColumnKind::Hex64 => cell
            .strip_prefix("0x")
            .is_some_and(|digits| is_lower_hex(digits, 16)),
        ColumnKind::Enum(labels) => labels.contains(&cell),
        ColumnKind::Locator => is_locator(cell),
    };
    if well_formed {
        Ok(())
    } else {
        Err(ArchiveError::BadCell {
            table,
            line,
            column: name,
            cell: cell.to_string(),
            expected: column.kind.describe(),
        })
    }
}

fn key_of(spec: &TableSpec, cells: &[&str]) -> Vec<KeyPart> {
    spec.key_indexes()
        .into_iter()
        .filter_map(|i| Some((spec.columns.get(i)?, *cells.get(i)?)))
        .map(|(column, cell)| match column.kind {
            ColumnKind::Integer => cell
                .parse()
                .map_or_else(|_| KeyPart::Text(cell.to_string()), KeyPart::Int),
            _ => KeyPart::Text(cell.to_string()),
        })
        .collect()
}

/// Parse one table's text against `spec` and refuse at the first rule the text breaks.
///
/// # Errors
///
/// Every [`ArchiveError`] variant except the two reference variants,
/// which [`check_references`] reports.
pub fn parse(spec: &'static TableSpec, text: &str) -> Result<Table, ArchiveError> {
    let table = spec.name;
    if !text.ends_with('\n') {
        return Err(ArchiveError::MissingFinalNewline { table });
    }
    let header: Vec<&str> = spec.columns.iter().map(|c| c.name).collect();
    let mut rows = Vec::new();
    let mut previous_key: Option<Vec<KeyPart>> = None;
    for (index, raw) in text.split_terminator('\n').enumerate() {
        let line = index + 1;
        if raw.contains('\r') {
            return Err(ArchiveError::CarriageReturn { table, line });
        }
        if !raw.is_ascii() {
            return Err(ArchiveError::NonAscii { table, line });
        }
        let cells: Vec<&str> = raw.split('\t').collect();
        if line == 1 {
            if cells != header {
                return Err(ArchiveError::Header {
                    table,
                    expected: header.join("\t"),
                    found: raw.to_string(),
                });
            }
            continue;
        }
        if cells.len() != spec.columns.len() {
            return Err(ArchiveError::CellCount {
                table,
                line,
                expected: spec.columns.len(),
                found: cells.len(),
            });
        }
        for (column, cell) in spec.columns.iter().zip(&cells) {
            check_cell(table, line, column, cell)?;
        }
        let key = key_of(spec, &cells);
        if let Some(previous) = &previous_key {
            match key.cmp(previous) {
                Ordering::Less => return Err(ArchiveError::Unsorted { table, line }),
                Ordering::Equal => return Err(ArchiveError::DuplicateKey { table, line }),
                Ordering::Greater => {}
            }
        }
        previous_key = Some(key);
        rows.push(cells.iter().map(|c| (*c).to_string()).collect());
    }
    Ok(Table { spec, rows })
}

/// Render `rows` under `spec`'s header, then parse the result so a
/// generator cannot write a table the loader would refuse.
///
/// # Errors
///
/// Whatever [`parse`] refuses in the rendered text.
pub fn render(spec: &'static TableSpec, rows: &[Vec<String>]) -> Result<String, ArchiveError> {
    let header: Vec<&str> = spec.columns.iter().map(|c| c.name).collect();
    let mut out = header.join("\t");
    out.push('\n');
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    parse(spec, &out)?;
    Ok(out)
}

/// Check every referencing column of every table in `tables` against
/// the table it names.
///
/// # Errors
///
/// - [`ArchiveError::DanglingReference`] if the target table has no matching row.
/// - [`ArchiveError::ReferenceTargetMissing`] if `tables` does not hold the target.
/// - [`ArchiveError::ReferenceTargetNotKey`] if SQLite would reject the foreign key.
pub fn check_references(tables: &[Table]) -> Result<(), ArchiveError> {
    for table in tables {
        for (column_index, column) in table.spec.columns.iter().enumerate() {
            let Some((target_table, target_column)) = column.references else {
                continue;
            };
            let missing = || ArchiveError::ReferenceTargetMissing {
                table: table.spec.name,
                column: column.name,
                target_table,
                target_column,
            };
            let target = tables
                .iter()
                .find(|t| t.spec.name == target_table)
                .ok_or_else(missing)?;
            let target_index = target
                .spec
                .columns
                .iter()
                .position(|c| c.name == target_column)
                .ok_or_else(missing)?;
            if target.spec.key != [target_column] {
                return Err(ArchiveError::ReferenceTargetNotKey {
                    table: table.spec.name,
                    column: column.name,
                    target_table,
                    target_column,
                });
            }
            let values: BTreeSet<&str> = target
                .rows
                .iter()
                .filter_map(|row| row.get(target_index).map(String::as_str))
                .collect();
            for (row_index, row) in table.rows.iter().enumerate() {
                let Some(cell) = row.get(column_index) else {
                    continue;
                };
                if cell != NONE && !values.contains(cell.as_str()) {
                    return Err(ArchiveError::DanglingReference {
                        table: table.spec.name,
                        line: row_index + 2,
                        column: column.name,
                        cell: cell.clone(),
                        target_table,
                        target_column,
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/table_tests.rs"]
mod tests;
