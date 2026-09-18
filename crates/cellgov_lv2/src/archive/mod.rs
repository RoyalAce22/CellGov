//! The LV2 archive: the text tables under `docs/lv2/` and the rules they obey.
//!
//! Text in, text out: nothing here opens a file. The `lv2_archive`
//! integration test does the I/O for the generated files: it writes
//! them on `--ignored regenerate` and fails when they drift.

mod behavior;
mod census;
mod coverage;
mod firmware;
mod handling;
mod name;
mod pup;
mod spec;
mod sql;
mod table;
mod transitions;

pub use behavior::{
    arm_token, foldable, parse_citation, parse_witness, provenance_ref_fits, Witness, DOC_KEYS,
    WITNESS_CRATES,
};
pub use census::{
    census_file, census_rows, census_tsv, gate_rows, gate_tsv, kernel_rows, kernel_tsv,
    presence_rows, presence_tsv, stub_rows, stub_tsv, subentry_rows, subentry_tsv, CensusClass,
    CensusRow, DispatchShape, GateRow, GateState, KernelRow, PresenceError, PresenceRow, StubRow,
    SubentryRow,
};
pub use coverage::{coverage_rows, coverage_tsv, CoverageRow, CoverageScope};
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
pub use pup::{check_pup_rows, pup_rows, PupRow, PupTableError};
pub use spec::{
    files, manifest, Column, ColumnKind, ManifestRow, OwnerClass, TableSpec, View, ARM, BEHAVIOR,
    BEHAVIOR_GATE, CALLER, CALLER_GATE, CALLER_REGENERATE, CALLER_UNRESOLVED, CAPABILITY_GATE,
    CENSUS, CENSUS_CLASSES, CENSUS_GATE, CENSUS_REGENERATE, COMPARISON_STATES, CONFLICTS, COVERAGE,
    COVERAGE_SCOPES, DISAGREEMENTS, DISCOVERY_CONFIDENCE, DISCOVERY_METHODS, DISPATCH_SHAPES,
    ENTRY_FORMATS, EXCEPTIONS, FIDELITY_LABELS, FIRMWARE, FIRMWARE_GATE, FIRMWARE_ROLES, GATE,
    GATE_STATES, KERNEL, NAME, NAME_GATE, NAME_REGENERATE, NAME_SOURCES, PRESENCE, PRIMARY_LABELS,
    PROVENANCE_KINDS, PUP, PUP_GATE, REACH, REGENERATE, ROUTE, ROUTE_LABELS, SCHEMA_VERSION,
    SELECTOR_SLOTS, STUB, SUBENTRY, SUBENTRY_ATTRIBUTION, SUBENTRY_SOURCES, TABLES, TRANSITIONS,
    TRANSITION_KINDS, VIEWS,
};
pub use sql::{build_sql, schema_sql, SQLITE_VERSION};
pub use table::{check_references, parse, render, ArchiveError, Table, NONE};
pub use transitions::{
    transitions, transitions_tsv, ComparisonState, TransitionKind, TransitionRow,
};
