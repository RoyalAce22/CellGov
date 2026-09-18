//! The LV2 archive: the text tables under `docs/lv2/` and the rules they obey.
//!
//! Text in, text out: nothing here opens a file. The `lv2_archive`
//! integration test does the I/O for the generated files: it writes
//! them on `--ignored regenerate` and fails when they drift.

mod behavior;
mod firmware;
mod handling;
mod name;
mod spec;
mod sql;
mod table;

pub use behavior::{
    arm_token, foldable, parse_citation, parse_witness, provenance_ref_fits, Witness, DOC_KEYS,
    WITNESS_CRATES,
};
pub use firmware::{
    check_firmware_rows, firmware_rows, is_version_key, FirmwareRole, FirmwareRow,
    FirmwareTableError,
};
pub use handling::{
    arm_rows, arm_tsv, route_rows, route_tsv, ArmRow, HandlingCounts, Route, RouteRow,
};
pub use name::{
    conflict_rows, conflicts_tsv, macro_name_rows, name_rows, name_tsv, uncorroborated,
    with_cellgov_rows, ConflictRow, Disagreement, NameRow, NameSource, CELLGOV_CONSTANT_PATH,
};
pub use spec::{
    files, manifest, Column, ColumnKind, ManifestRow, OwnerClass, TableSpec, View, ARM, BEHAVIOR,
    BEHAVIOR_GATE, CONFLICTS, DISAGREEMENTS, EXCEPTIONS, FIDELITY_LABELS, FIRMWARE, FIRMWARE_GATE,
    FIRMWARE_ROLES, GATE, NAME, NAME_GATE, NAME_REGENERATE, NAME_SOURCES, PROVENANCE_KINDS,
    REGENERATE, ROUTE, ROUTE_LABELS, SELECTOR_SLOTS, TABLES, VIEWS,
};
pub use sql::{build_sql, schema_sql, SQLITE_VERSION};
pub use table::{check_references, parse, render, ArchiveError, Table, NONE};
