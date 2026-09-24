//! The firmware caller tables: `caller.tsv`, `caller_unresolved.tsv` and `reach.tsv`.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec};

/// Refreshes the firmware caller tables from installed modules.
pub const CALLER_REGENERATE: &str =
    "cargo run --release -p cellgov_cli --features decrypt -- dev caller-census --all --output-dir docs/lv2";

/// Checks the three firmware caller tables.
pub const CALLER_GATE: &str = "caller_rows_are_well_formed";

/// Defines `caller.tsv` with resolved syscall sites grouped by PUP, module, and ordinal.
pub const CALLER: TableSpec = TableSpec {
    name: "caller",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "module",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "sites",
            kind: ColumnKind::IntegerList,
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256", "module", "ordinal"],
    regenerate: Some(CALLER_REGENERATE),
    gate: CALLER_GATE,
};

/// Defines `caller_unresolved.tsv` with every scanned module and its unresolved sites.
pub const CALLER_UNRESOLVED: TableSpec = TableSpec {
    name: "caller_unresolved",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "module",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "sites",
            kind: ColumnKind::IntegerList,
            nullable: true,
            references: None,
        },
    ],
    key: &["pup_sha256", "module"],
    regenerate: Some(CALLER_REGENERATE),
    gate: CALLER_GATE,
};

/// Defines `reach.tsv` with exported functions and the resolved ordinals they reach.
pub const REACH: TableSpec = TableSpec {
    name: "reach",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        Column {
            name: "module",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "export_nid",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
    ],
    key: &["pup_sha256", "module", "export_nid", "ordinal"],
    regenerate: Some(CALLER_REGENERATE),
    gate: CALLER_GATE,
};
