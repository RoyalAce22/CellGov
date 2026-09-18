//! Owns the `firmware.tsv` rows that index the archive's per-version tables.

use super::spec::FIRMWARE;
use super::table::{Table, NONE};

/// Defines the archive role of a firmware version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareRole {
    /// Marks the first retail release.
    Baseline,
    /// The census starts dispatch-table characterisation with this version.
    CensusReference,
    /// Marks the last release of the final line.
    Final,
    /// Marks a version without a special archive role.
    None,
}

impl FirmwareRole {
    /// Keeps the role order used by the archive document.
    pub const ALL: &[FirmwareRole] = &[
        FirmwareRole::Baseline,
        FirmwareRole::CensusReference,
        FirmwareRole::Final,
        FirmwareRole::None,
    ];

    /// Maps [`FirmwareRole::None`] to the null cell used by `firmware.tsv`.
    pub fn label(self) -> &'static str {
        match self {
            FirmwareRole::Baseline => "baseline",
            FirmwareRole::CensusReference => "census_reference",
            FirmwareRole::Final => "final",
            FirmwareRole::None => NONE,
        }
    }

    /// Finds the role with `label`, if any.
    pub fn from_label(label: &str) -> Option<FirmwareRole> {
        FirmwareRole::ALL
            .iter()
            .copied()
            .find(|r| r.label() == label)
    }

    /// Returns the archive document's one-line definition.
    pub fn meaning(self) -> &'static str {
        match self {
            FirmwareRole::Baseline => "The first retail release.",
            FirmwareRole::CensusReference => {
                "The version the dispatch table is characterised on first; the rest are read against it."
            }
            FirmwareRole::Final => "The last release of the final line.",
            FirmwareRole::None => "No role beyond its row.",
        }
    }
}

/// Represents one row of `firmware.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareRow {
    /// Stores the version key with the store's spelling, such as `4.91`.
    pub fw: String,
    /// Orders rows with an integer key that rises down the file.
    ///
    /// This key does not sort rows by release date.
    pub order: u64,
    /// Stores the release date as `YYYY-MM-DD`, or `None` if no source states one.
    pub release_date: Option<String>,
    /// Ranks how soon the program needs the version, with `1` first.
    pub priority: u64,
    /// Stores the version's archive role.
    pub role: FirmwareRole,
}

/// Decodes rows after loader validation against [`FIRMWARE`].
///
/// # Panics
///
/// Panics if `table` was not parsed with [`FIRMWARE`].
pub fn firmware_rows(table: &Table) -> Vec<FirmwareRow> {
    assert_eq!(table.spec.name, FIRMWARE.name, "not a firmware table");
    let integer = |cell: &String| {
        cell.parse()
            .unwrap_or_else(|_| panic!("the loader passed {cell:?} as an integer"))
    };
    table
        .rows
        .iter()
        .map(|cells| FirmwareRow {
            fw: cells[0].clone(),
            order: integer(&cells[1]),
            release_date: (cells[2] != NONE).then(|| cells[2].clone()),
            priority: integer(&cells[3]),
            role: FirmwareRole::from_label(&cells[4])
                .unwrap_or_else(|| panic!("the loader passed {:?} as a role", cells[4])),
        })
        .collect()
}

/// Reports errors from checks that extend beyond loader validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FirmwareTableError {
    /// The cell is not the store's version key spelling.
    #[error(
        "firmware.tsv row {fw:?}: not a version key (one or two digits with no leading zero, a dot, two digits)"
    )]
    NotAVersionKey {
        /// The `fw` cell.
        fw: String,
    },
    /// A priority violates the range or row contract in the archive document.
    #[error("firmware.tsv row {fw:?}: priority {priority} is not {expected}")]
    Priority {
        /// The `fw` cell.
        fw: String,
        /// Its priority.
        priority: u64,
        /// What the document allows the row.
        expected: &'static str,
    },
    /// A firmware row required by the priority or role contract is
    /// absent.
    #[error("firmware.tsv has no row for required firmware {fw} ({purpose})")]
    RequiredRow {
        /// The required firmware.
        fw: &'static str,
        /// The priority or role contract that names the row.
        purpose: &'static str,
    },
    /// `order` does not rise from the row before.
    #[error(
        "firmware.tsv row {fw:?}: order {order} does not rise from the row before ({previous})"
    )]
    OrderNotRising {
        /// The `fw` cell.
        fw: String,
        /// Its order.
        order: u64,
        /// The order of the row before it.
        previous: u64,
    },
    /// A release date violates the `YYYY-MM-DD` calendar rules.
    #[error("firmware.tsv row {fw:?}: release date {date:?} is not YYYY-MM-DD")]
    BadDate {
        /// The `fw` cell.
        fw: String,
        /// The cell.
        date: String,
    },
    /// A role that exactly one row must carry appears on none or on
    /// several.
    #[error("firmware.tsv: {count} rows carry the role {role}, which exactly one row carries")]
    RoleCount {
        /// The role.
        role: &'static str,
        /// How many rows carry it.
        count: usize,
    },
    /// A unique role sits on a row other than the one its definition
    /// names.
    #[error("firmware.tsv: role {role} is on row {fw:?}, expected {expected}")]
    RoleRow {
        /// The role.
        role: &'static str,
        /// The row carrying it.
        fw: String,
        /// The row the role definition names.
        expected: String,
    },
}

const PRIORITIES: std::ops::RangeInclusive<u64> = 1..=3;

/// Checks whether `fw` uses the store's version key spelling.
///
/// The store uses the spelling from `version.txt`, such as `1.50` for
/// PARAM.SFO's `01.5000`. It compares keys as strings, so `01.50` matches no
/// entry.
pub fn is_version_key(fw: &str) -> bool {
    let Some((major, minor)) = fw.split_once('.') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let unpadded = major == "0" || !major.starts_with('0');
    digits(major) && major.len() <= 2 && unpadded && digits(minor) && minor.len() == 2
}

/// Checks the firmware-row invariants that the loader cannot verify.
///
/// The check requires:
///
/// - Each `fw` uses the store's version key spelling.
/// - Each `order` is greater than the value in the prior row.
/// - Each date is a valid Gregorian date in `YYYY-MM-DD` form.
/// - Each priority is in `1..=3`.
/// - Only the `3.60` and `4.00` rows can have priority `2`.
/// - Each non-null role occurs on exactly one required row.
/// - The census-reference row has priority `1`.
///
/// # Errors
///
/// Returns the first [`FirmwareTableError`] for an invalid row set.
pub fn check_firmware_rows(rows: &[FirmwareRow]) -> Result<(), FirmwareTableError> {
    let mut previous: Option<u64> = None;
    for row in rows {
        if !is_version_key(&row.fw) {
            return Err(FirmwareTableError::NotAVersionKey { fw: row.fw.clone() });
        }
        if !PRIORITIES.contains(&row.priority) {
            return Err(FirmwareTableError::Priority {
                fw: row.fw.clone(),
                priority: row.priority,
                expected: "1, 2 or 3",
            });
        }
        if row.role == FirmwareRole::CensusReference && row.priority != 1 {
            return Err(FirmwareTableError::Priority {
                fw: row.fw.clone(),
                priority: row.priority,
                expected: "1, which the census reference carries",
            });
        }
        let opens_era = matches!(row.fw.as_str(), "3.60" | "4.00");
        if row.priority == 2 && !opens_era {
            return Err(FirmwareTableError::Priority {
                fw: row.fw.clone(),
                priority: row.priority,
                expected: "1 or 3; priority 2 belongs to 3.60 and 4.00",
            });
        }
        if opens_era && row.priority == 3 {
            return Err(FirmwareTableError::Priority {
                fw: row.fw.clone(),
                priority: row.priority,
                expected: "1 or 2; 3.60 and 4.00 have raised priority",
            });
        }
        if let Some(previous) = previous {
            if row.order <= previous {
                return Err(FirmwareTableError::OrderNotRising {
                    fw: row.fw.clone(),
                    order: row.order,
                    previous,
                });
            }
        }
        previous = Some(row.order);
        if let Some(date) = &row.release_date {
            if !is_date(date) {
                return Err(FirmwareTableError::BadDate {
                    fw: row.fw.clone(),
                    date: date.clone(),
                });
            }
        }
    }
    // The archive's priority contract names both era-opening rows.
    for fw in ["3.60", "4.00"] {
        if !rows.iter().any(|row| row.fw == fw) {
            return Err(FirmwareTableError::RequiredRow {
                fw,
                purpose: "priority 2 era opener",
            });
        }
    }
    for role in [
        FirmwareRole::Baseline,
        FirmwareRole::CensusReference,
        FirmwareRole::Final,
    ] {
        let count = rows.iter().filter(|r| r.role == role).count();
        if count != 1 {
            return Err(FirmwareTableError::RoleCount {
                role: role.label(),
                count,
            });
        }
    }
    // The archive's role contract fixes the first two versions and
    // defines final as the last retail release in the 4.9x line.
    for (role, expected) in [
        (FirmwareRole::Baseline, "1.00"),
        (FirmwareRole::CensusReference, "3.55"),
    ] {
        let row = rows
            .iter()
            .find(|row| row.role == role)
            .expect("the role count was checked above");
        if row.fw != expected {
            return Err(FirmwareTableError::RoleRow {
                role: role.label(),
                fw: row.fw.clone(),
                expected: expected.to_string(),
            });
        }
    }
    let final_row = rows
        .iter()
        .find(|row| row.role == FirmwareRole::Final)
        .expect("the role count was checked above");
    let expected_final = rows.iter().rev().find(|row| row.fw.starts_with("4.9"));
    if expected_final.is_none_or(|expected| expected.fw != final_row.fw) {
        return Err(FirmwareTableError::RoleRow {
            role: FirmwareRole::Final.label(),
            fw: final_row.fw.clone(),
            expected: expected_final
                .map_or_else(|| "the last 4.9x row".to_string(), |row| row.fw.clone()),
        });
    }
    Ok(())
}

fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts[..] else {
        return false;
    };
    let number = |s: &str, len: usize| {
        (s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let (Some(year), Some(month), Some(day)) = (number(year, 4), number(month, 2), number(day, 2))
    else {
        return false;
    };
    // docs/lv2/README.md defines release_date as a calendar day, so a
    // digit-shaped but impossible date is malformed too.
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;
