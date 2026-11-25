//! The table list and the manifest of every archive file.

use cellgov_lv2_archive::{ManifestRow, OwnerClass, TableSpec, FIRMWARE, PUP};

use super::hypercall::{HYPERCALL, HYPERCALL_SITE, HYPERCALL_UNRESOLVED};
use super::repository::{REPOSITORY, REPOSITORY_READ};
use super::schema::{GATE, REGENERATE};

/// The LV2 archive's tables that the lv1 tables reference, a referenced
/// table first. They stay under `docs/lv2/`.
pub const SHARED_TABLES: &[TableSpec] = &[FIRMWARE, PUP];

/// Every table of this archive, a referenced table before the table that
/// references it.
pub const TABLES: &[TableSpec] = &[
    HYPERCALL_SITE,
    HYPERCALL_UNRESOLVED,
    HYPERCALL,
    REPOSITORY_READ,
    REPOSITORY,
];

/// The files under `docs/lv1/` that are not tables; the one regenerate
/// command writes all of them.
const FIXED_FILES: &[&str] = &["README.md", "sql/schema.sql", "sql/build.sql"];

/// Lists every archive file in file-name order.
pub fn manifest() -> Vec<ManifestRow> {
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
    rows.sort_by(|a, b| a.file.cmp(&b.file));
    rows
}

/// The file names of [`manifest`], in its order.
pub fn files() -> Vec<String> {
    manifest().into_iter().map(|row| row.file).collect()
}
