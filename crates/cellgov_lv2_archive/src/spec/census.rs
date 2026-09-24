//! The extracted kernel census tables and the generated reductions over them.

use super::handling::SELECTOR_SLOTS;
use super::schema::{Column, ColumnKind, OwnerClass, TableSpec, GATE, REGENERATE};

/// Provides the command that refreshes extracted rows for one kernel.
pub const CENSUS_REGENERATE: &str =
    "cargo run --release -p cellgov_cli -- dev lv2-census <ELF> --fw <VERSION> --pup-sha256 <SHA256> --output-dir docs/lv2";

/// Names the self-contained gate for extracted kernel census rows.
pub const CENSUS_GATE: &str = "kernel_census_rows_are_well_formed";

/// Lists the accepted labels for `kernel.entry_format`.
pub const ENTRY_FORMATS: &[&str] = &["ppc64_descriptor_pointer"];

/// Lists the accepted labels for `kernel.discovery_method`.
pub const DISCOVERY_METHODS: &[&str] = &["sc_vector_descriptor_array"];

/// Lists the accepted labels for `kernel.confidence`.
pub const DISCOVERY_CONFIDENCE: &[&str] = &["high"];

/// Lists the accepted labels for extracted dispatch classes.
pub const CENSUS_CLASSES: &[&str] = &["implemented", "stub", "absent"];

/// Lists the accepted labels for `census.dispatch`.
pub const DISPATCH_SHAPES: &[&str] = &["flat", "subtable", "chain_incomplete"];

/// Lists the accepted labels for `stub.primary`.
pub const PRIMARY_LABELS: &[&str] = &["yes", "no"];

/// Restricts packet attribution to sources recorded in the archive.
pub const SUBENTRY_SOURCES: &[&str] = &["psdevwiki"];

/// Lists the capability-gate analysis states.
pub const GATE_STATES: &[&str] = &["gated", "ungated", "not_analysed"];

/// Lists adjacent-pair comparison states.
pub const COMPARISON_STATES: &[&str] = &["compared", "not_compared"];

/// Lists cross-version census transition kinds.
pub const TRANSITION_KINDS: &[&str] = &[
    "added",
    "removed",
    "class_changed",
    "retargeted",
    "gate_added",
    "gate_removed",
];

/// Lists extracted-surface coverage denominators.
pub const COVERAGE_SCOPES: &[&str] = &["implemented", "total"];

/// Defines `kernel.tsv` with provenance and discovery evidence per PUP.
pub const KERNEL: TableSpec = TableSpec {
    name: "kernel",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "kernel_elf_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: None,
        },
        Column {
            name: "table_base",
            kind: ColumnKind::Hex64,
            nullable: false,
            references: None,
        },
        Column {
            name: "entry_width",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "entry_format",
            kind: ColumnKind::Enum(ENTRY_FORMATS),
            nullable: false,
            references: None,
        },
        Column {
            name: "entry_count",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "discovery_method",
            kind: ColumnKind::Enum(DISCOVERY_METHODS),
            nullable: false,
            references: None,
        },
        Column {
            name: "confidence",
            kind: ColumnKind::Enum(DISCOVERY_CONFIDENCE),
            nullable: false,
            references: None,
        },
        Column {
            name: "census_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: None,
        },
        Column {
            name: "subentry_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: None,
        },
        Column {
            name: "gate_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256"],
    regenerate: Some(CENSUS_REGENERATE),
    gate: CENSUS_GATE,
};

/// Defines `stub.tsv` with every decoded constant-error target per PUP.
pub const STUB: TableSpec = TableSpec {
    name: "stub",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("kernel", "pup_sha256")),
        },
        Column {
            name: "descriptor",
            kind: ColumnKind::Hex64,
            nullable: false,
            references: None,
        },
        Column {
            name: "target",
            kind: ColumnKind::Hex64,
            nullable: false,
            references: None,
        },
        Column {
            name: "errno",
            kind: ColumnKind::Hex32,
            nullable: false,
            references: None,
        },
        Column {
            name: "errno_symbol",
            kind: ColumnKind::Ident,
            nullable: false,
            references: None,
        },
        Column {
            name: "references",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "primary",
            kind: ColumnKind::Enum(PRIMARY_LABELS),
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256", "descriptor"],
    regenerate: Some(CENSUS_REGENERATE),
    gate: CENSUS_GATE,
};

/// Keeps extracted packet dispatch separate for each source PUP.
pub const SUBENTRY: TableSpec = TableSpec {
    name: "subentry",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("kernel", "pup_sha256")),
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "selector_slot",
            kind: ColumnKind::Enum(SELECTOR_SLOTS),
            nullable: false,
            references: None,
        },
        Column {
            name: "packet",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "class",
            kind: ColumnKind::Enum(CENSUS_CLASSES),
            nullable: false,
            references: None,
        },
        Column {
            name: "target",
            kind: ColumnKind::Hex64,
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256", "ordinal", "packet"],
    regenerate: Some(CENSUS_REGENERATE),
    gate: CENSUS_GATE,
};

/// Keeps attributed packet identifiers separate from firmware extraction.
pub const SUBENTRY_ATTRIBUTION: TableSpec = TableSpec {
    name: "subentry_attribution",
    owner: OwnerClass::Attributed,
    columns: &[
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "selector_slot",
            kind: ColumnKind::Enum(SELECTOR_SLOTS),
            nullable: false,
            references: None,
        },
        Column {
            name: "packet",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "source",
            kind: ColumnKind::Enum(SUBENTRY_SOURCES),
            nullable: false,
            references: None,
        },
        Column {
            name: "ref",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
    ],
    key: &["ordinal", "selector_slot", "packet", "source"],
    regenerate: None,
    gate: CENSUS_GATE,
};

/// Defines `gate.tsv` with capability checks extracted from each PUP.
pub const CAPABILITY_GATE: TableSpec = TableSpec {
    name: "gate",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("kernel", "pup_sha256")),
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "state",
            kind: ColumnKind::Enum(GATE_STATES),
            nullable: false,
            references: None,
        },
        Column {
            name: "reads",
            kind: ColumnKind::Ident,
            nullable: true,
            references: None,
        },
        Column {
            name: "fail_errno",
            kind: ColumnKind::Hex32,
            nullable: true,
            references: None,
        },
    ],
    key: &["pup_sha256", "ordinal"],
    regenerate: Some(CENSUS_REGENERATE),
    gate: CENSUS_GATE,
};

/// Defines `presence.tsv`, the generated per-ordinal census reduction.
pub const PRESENCE: TableSpec = TableSpec {
    name: "presence",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "implemented_versions",
            kind: ColumnKind::VersionList,
            nullable: true,
            references: None,
        },
        Column {
            name: "stub_versions",
            kind: ColumnKind::VersionList,
            nullable: true,
            references: None,
        },
        Column {
            name: "absent_versions",
            kind: ColumnKind::VersionList,
            nullable: true,
            references: None,
        },
    ],
    key: &["ordinal"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};

/// Defines `transitions.tsv`, the generated adjacent-version changelog.
pub const TRANSITIONS: TableSpec = TableSpec {
    name: "transitions",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "record",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "from_fw",
            kind: ColumnKind::Locator,
            nullable: false,
            references: Some(("firmware", "fw")),
        },
        Column {
            name: "to_fw",
            kind: ColumnKind::Locator,
            nullable: false,
            references: Some(("firmware", "fw")),
        },
        Column {
            name: "comparison",
            kind: ColumnKind::Enum(COMPARISON_STATES),
            nullable: false,
            references: None,
        },
        Column {
            name: "kind",
            kind: ColumnKind::Enum(TRANSITION_KINDS),
            nullable: true,
            references: None,
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: true,
            references: Some(("route", "ordinal")),
        },
    ],
    key: &["record"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};

/// Defines `coverage.tsv`, the generated extracted-surface coverage report.
pub const COVERAGE: TableSpec = TableSpec {
    name: "coverage",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "scope",
            kind: ColumnKind::Enum(COVERAGE_SCOPES),
            nullable: false,
            references: None,
        },
        Column {
            name: "extracted",
            kind: ColumnKind::Integer,
            nullable: true,
            references: None,
        },
        Column {
            name: "handled",
            kind: ColumnKind::Integer,
            nullable: true,
            references: None,
        },
        Column {
            name: "versions",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
    ],
    key: &["scope"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};

/// Defines each `census/fw-<version>.tsv` file.
pub const CENSUS: TableSpec = TableSpec {
    name: "census",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "fw",
            kind: ColumnKind::Locator,
            nullable: false,
            references: Some(("firmware", "fw")),
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "class",
            kind: ColumnKind::Enum(CENSUS_CLASSES),
            nullable: false,
            references: None,
        },
        Column {
            name: "target",
            kind: ColumnKind::Hex64,
            nullable: true,
            references: None,
        },
        Column {
            name: "dispatch",
            kind: ColumnKind::Enum(DISPATCH_SHAPES),
            nullable: false,
            references: None,
        },
    ],
    key: &["fw", "ordinal"],
    regenerate: Some(CENSUS_REGENERATE),
    gate: CENSUS_GATE,
};
