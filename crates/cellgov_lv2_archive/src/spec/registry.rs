//! The table and view lists, and the manifest of every archive file.

use super::caller::{CALLER, CALLER_UNRESOLVED, REACH};
use super::census::{
    CAPABILITY_GATE, CENSUS_GATE, CENSUS_REGENERATE, COVERAGE, KERNEL, PRESENCE, STUB, SUBENTRY,
    SUBENTRY_ATTRIBUTION, TRANSITIONS,
};
use super::firmware::{FIRMWARE, PUP};
use super::handling::{ARM, BEHAVIOR, ROUTE};
use super::naming::{CONFLICTS, NAME, PRIORITY};
use super::schema::{OwnerClass, TableSpec, View, GATE, REGENERATE};

/// Every table, a referenced table before the table that references it.
pub const TABLES: &[TableSpec] = &[
    FIRMWARE,
    PUP,
    ARM,
    ROUTE,
    KERNEL,
    STUB,
    SUBENTRY,
    CAPABILITY_GATE,
    PRESENCE,
    TRANSITIONS,
    COVERAGE,
    SUBENTRY_ATTRIBUTION,
    CALLER,
    CALLER_UNRESOLVED,
    REACH,
    BEHAVIOR,
    NAME,
    CONFLICTS,
    PRIORITY,
];

/// The files under `docs/lv2/` that are not tables; the one regenerate
/// command writes all of them.
const FIXED_FILES: &[&str] = &["README.md", "sql/schema.sql", "sql/build.sql"];

/// One row of the archive document's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRow {
    /// The file name under `docs/lv2/`.
    pub file: String,
    /// Who writes it.
    pub owner: OwnerClass,
    /// The command that rewrites it; `None` for a file written by hand.
    pub regenerate: Option<&'static str>,
    /// The test that fails when the committed copy is stale or wrong.
    pub gate: &'static str,
}

/// Lists every archive file in file-name order.
///
/// The caller supplies the discovered per-version census paths.
pub fn manifest(census_files: &[String]) -> Vec<ManifestRow> {
    let mut rows: Vec<ManifestRow> = FIXED_FILES
        .iter()
        .map(|file| ManifestRow {
            file: (*file).to_string(),
            owner: OwnerClass::Generated,
            regenerate: Some(REGENERATE),
            gate: GATE,
        })
        .collect();
    rows.extend(TABLES.iter().map(|table| ManifestRow {
        file: table.file(),
        owner: table.owner,
        regenerate: table.regenerate,
        gate: table.gate,
    }));
    rows.extend(census_files.iter().map(|file| ManifestRow {
        file: file.clone(),
        owner: OwnerClass::Extracted,
        regenerate: Some(CENSUS_REGENERATE),
        gate: CENSUS_GATE,
    }));
    rows.sort_by(|a, b| a.file.cmp(&b.file));
    rows
}

/// Lists every archive file in file-name order.
///
/// The caller supplies the discovered per-version census paths.
pub fn files(census_files: &[String]) -> Vec<String> {
    manifest(census_files)
        .into_iter()
        .map(|row| row.file)
        .collect()
}

/// Every view.
pub const VIEWS: &[View] = &[
    View {
        name: "handling",
        select: "SELECT route.ordinal, route.route, route.arm, arm.fidelity\n\
                 FROM route\n\
                 LEFT JOIN arm ON arm.arm = route.arm",
    },
    View {
        name: "authority",
        select: "SELECT behavior.ordinal, route.arm, arm.fidelity,\n\
                 \x20      behavior.provenance_kind, behavior.provenance_ref,\n\
                 \x20      behavior.witness, behavior.exception, behavior.arm_source\n\
                 FROM behavior\n\
                 JOIN route ON route.ordinal = behavior.ordinal\n\
                 LEFT JOIN arm ON arm.arm = route.arm",
    },
];
