//! The LV1 hypercall tables: `hypercall_site.tsv`,
//! `hypercall_unresolved.tsv` and `hypercall.tsv`.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec, GATE, REGENERATE};

/// Rewrites the two hypercall site tables: `cellgov dev hypercall-census`
/// over every `kernel.tsv` kernel in the installed firmware.
pub const HYPERCALL_REGENERATE: &str = "CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test lv2_census_firmware firmware_reextracts_byte_identical_hypercall_rows";

/// Checks the two hypercall site tables.
pub const HYPERCALL_GATE: &str = "hypercall_site_rows_are_well_formed";

/// The families `hypercall.tsv` groups the numbers into.
pub const HYPERCALL_FAMILIES: &[&str] = &[
    "console",
    "device",
    "interrupt",
    "logical_spe",
    "memory",
    "other",
    "partition",
    "repository",
    "undocumented",
    "unnamed",
];

/// The work items that answer a family's calls.
pub const HYPERCALL_LEAVES: &[&str] = &[
    "boot",
    "dispatch",
    "interrupt_delivery",
    "logical_spe",
    "repository",
    "translation",
];

/// Labels of `hypercall.state`; `HypercallState::label` gives each one.
pub const HYPERCALL_STATES: &[&str] = &["answered", "null_backend"];

/// Defines `hypercall_site.tsv`: each kernel's hypercall sites, by
/// number.
pub const HYPERCALL_SITE: TableSpec = TableSpec {
    name: "hypercall_site",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "number",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "sites",
            kind: ColumnKind::IntegerList,
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256", "number"],
    regenerate: Some(HYPERCALL_REGENERATE),
    gate: HYPERCALL_GATE,
};

/// Defines `hypercall_unresolved.tsv`: every scanned kernel, and its
/// sites whose number the scan could not prove.
pub const HYPERCALL_UNRESOLVED: TableSpec = TableSpec {
    name: "hypercall_unresolved",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "sites",
            kind: ColumnKind::IntegerList,
            nullable: true,
            references: None,
        },
    ],
    key: &["pup_sha256"],
    regenerate: Some(HYPERCALL_REGENERATE),
    gate: HYPERCALL_GATE,
};

/// Defines `hypercall.tsv`: one row per number a scanned kernel calls or
/// CellGov's hypervisor answers.
pub const HYPERCALL: TableSpec = TableSpec {
    name: "hypercall",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "number",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "name",
            kind: ColumnKind::Ident,
            nullable: true,
            references: None,
        },
        Column {
            name: "inputs",
            kind: ColumnKind::Integer,
            nullable: true,
            references: None,
        },
        Column {
            name: "outputs",
            kind: ColumnKind::Integer,
            nullable: true,
            references: None,
        },
        Column {
            name: "family",
            kind: ColumnKind::Enum(HYPERCALL_FAMILIES),
            nullable: false,
            references: None,
        },
        Column {
            name: "leaf",
            kind: ColumnKind::Enum(HYPERCALL_LEAVES),
            nullable: false,
            references: None,
        },
        Column {
            name: "fw",
            kind: ColumnKind::VersionList,
            nullable: true,
            references: None,
        },
        Column {
            name: "sites",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "state",
            kind: ColumnKind::Enum(HYPERCALL_STATES),
            nullable: false,
            references: None,
        },
    ],
    key: &["number"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};
