//! The LV2 archive: the text tables under `docs/lv2/` and the rules they obey.
//!
//! Text in, text out: nothing here opens a file. The `lv2_archive`
//! integration test does the I/O: it writes the committed copies on
//! `--ignored regenerate` and fails when they drift.

mod handling;
mod spec;
mod sql;
mod table;

pub use handling::{
    arm_rows, arm_tsv, route_rows, route_tsv, ArmRow, HandlingCounts, Route, RouteRow,
};
pub use spec::{
    files, manifest, Column, ColumnKind, ManifestRow, OwnerClass, TableSpec, View, ARM,
    FIDELITY_LABELS, GATE, REGENERATE, ROUTE, ROUTE_LABELS, TABLES, VIEWS,
};
pub use sql::{build_sql, schema_sql, SQLITE_VERSION};
pub use table::{check_references, parse, render, ArchiveError, Table, NONE};
