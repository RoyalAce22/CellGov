//! Coverage denominators over the extracted LV2 census.

use std::collections::{BTreeMap, BTreeSet};

use super::spec::COVERAGE;
use super::table::{self, ArchiveError, NONE};
use super::{CensusClass, CensusRow, Route, RouteRow};

/// One extracted-surface coverage denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageScope {
    /// Ordinals implemented by at least one extracted kernel.
    Implemented,
    /// Ordinals present as either an implementation or a stub.
    Total,
}

impl CoverageScope {
    const fn label(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Total => "total",
        }
    }
}

/// Reports one handling count against one extracted-surface denominator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRow {
    /// Names the denominator.
    pub scope: CoverageScope,
    /// Counts ordinals in the extracted surface.
    pub extracted: Option<usize>,
    /// Counts extracted ordinals CellGov routes outside the null backend.
    pub handled: Option<usize>,
    /// Counts the firmware versions that contributed census rows.
    pub versions: usize,
}

impl CoverageRow {
    fn cells(&self) -> Vec<String> {
        vec![
            self.scope.label().to_string(),
            self.extracted
                .map_or_else(|| NONE.to_string(), |value| value.to_string()),
            self.handled
                .map_or_else(|| NONE.to_string(), |value| value.to_string()),
            self.versions.to_string(),
        ]
    }
}

/// Computes coverage against implemented and total extracted ordinals.
#[must_use]
pub fn coverage_rows(
    routes: &[RouteRow],
    census_by_version: &BTreeMap<String, Vec<CensusRow>>,
) -> Vec<CoverageRow> {
    if census_by_version.is_empty() {
        return vec![
            CoverageRow {
                scope: CoverageScope::Implemented,
                extracted: None,
                handled: None,
                versions: 0,
            },
            CoverageRow {
                scope: CoverageScope::Total,
                extracted: None,
                handled: None,
                versions: 0,
            },
        ];
    }
    let handled: BTreeSet<u64> = routes
        .iter()
        .filter(|row| row.route != Route::NullBackend)
        .map(|row| row.ordinal)
        .collect();
    let mut implemented = BTreeSet::new();
    let mut total = BTreeSet::new();
    for rows in census_by_version.values() {
        for row in rows {
            if row.class != CensusClass::Absent {
                total.insert(row.ordinal as u64);
            }
            if row.class == CensusClass::Implemented {
                implemented.insert(row.ordinal as u64);
            }
        }
    }
    [
        (CoverageScope::Implemented, implemented),
        (CoverageScope::Total, total),
    ]
    .into_iter()
    .map(|(scope, extracted)| CoverageRow {
        scope,
        handled: Some(extracted.intersection(&handled).count()),
        extracted: Some(extracted.len()),
        versions: census_by_version.len(),
    })
    .collect()
}

/// Canonicalizes extracted-surface coverage rows by denominator scope.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn coverage_tsv(rows: &[CoverageRow]) -> Result<String, ArchiveError> {
    let mut cells: Vec<Vec<String>> = rows.iter().map(CoverageRow::cells).collect();
    cells.sort();
    table::render(&COVERAGE, &cells)
}

#[cfg(test)]
#[path = "tests/coverage_tests.rs"]
mod tests;
