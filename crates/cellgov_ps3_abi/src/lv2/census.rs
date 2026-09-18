//! Generated LV2 census classifications keyed by source PUP digest.

#[path = "census_table.rs"]
mod census_table;

use census_table::PUP_CENSUS;

/// Classifies one syscall ordinal in one source PUP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PupCensusClass {
    /// The dispatch table names an implementation.
    Implemented,
    /// The dispatch table names a constant-error stub.
    Stub,
    /// The dispatch table has no target for the ordinal.
    Absent,
    /// The PUP has no compiled-in census row.
    NotExtracted,
    /// The ordinal is outside the compiled dispatch-table width.
    OutOfRange,
}

/// One PUP's generated ordinal classifications.
pub(super) struct PupCensus {
    pub(super) pup_sha256: [u8; 32],
    pub(super) classes: &'static [u8],
}

fn class_from_byte(value: u8) -> PupCensusClass {
    match value {
        0 => PupCensusClass::Implemented,
        1 => PupCensusClass::Stub,
        2 => PupCensusClass::Absent,
        _ => panic!("invalid generated census class {value}"),
    }
}

/// Looks up one ordinal without consulting a version string or file.
#[must_use]
pub fn lookup(pup_sha256: &[u8; 32], ordinal: usize) -> PupCensusClass {
    let Ok(index) = PUP_CENSUS.binary_search_by_key(pup_sha256, |row| row.pup_sha256) else {
        return PupCensusClass::NotExtracted;
    };
    PUP_CENSUS[index]
        .classes
        .get(ordinal)
        .copied()
        .map_or(PupCensusClass::OutOfRange, class_from_byte)
}

#[cfg(test)]
#[path = "tests/census_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/census_gen_tests.rs"]
mod census_gen_tests;
