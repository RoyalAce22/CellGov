//! The LV2 archive: the text tables under `docs/lv2/` and the rules they obey.
//!
//! Text in, text out: nothing here opens a file. The `lv2_archive`
//! integration test does the I/O for the generated files: it writes
//! them on `--ignored regenerate` and fails when they drift.

mod behavior;
mod handling;
mod spec;
mod sql;
mod table;

pub use behavior::{
    arm_token, foldable, parse_citation, parse_witness, provenance_ref_fits, Witness, DOC_KEYS,
    WITNESS_CRATES,
};
pub use handling::{
    arm_rows, arm_tsv, route_rows, route_tsv, ArmRow, HandlingCounts, Route, RouteRow,
};
pub use spec::{
    files, manifest, Column, ColumnKind, ManifestRow, OwnerClass, TableSpec, View, ARM, BEHAVIOR,
    BEHAVIOR_GATE, EXCEPTIONS, FIDELITY_LABELS, GATE, PROVENANCE_KINDS, REGENERATE, ROUTE,
    ROUTE_LABELS, SELECTOR_SLOTS, TABLES, VIEWS,
};
pub use sql::{build_sql, schema_sql, SQLITE_VERSION};
pub use table::{check_references, parse, render, ArchiveError, Table, NONE};
