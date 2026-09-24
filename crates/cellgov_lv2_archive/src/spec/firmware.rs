//! The firmware tables: `firmware.tsv` and `pup.tsv`.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec};

/// Lists the non-null `firmware.role` labels in archive order.
///
/// A version without a role uses `none`, the null cell.
pub const FIRMWARE_ROLES: &[&str] = &["baseline", "census_reference", "final"];

/// Names the firmware-table validation test.
///
/// Each firmware version in a title manifest must have a row in this table.
pub const FIRMWARE_GATE: &str = "firmware_rows_are_well_formed";

/// Defines `firmware.tsv` with one curated row per retail firmware version.
pub const FIRMWARE: TableSpec = TableSpec {
    name: "firmware",
    owner: OwnerClass::Curated,
    columns: &[
        Column {
            name: "fw",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "order",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "release_date",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
        Column {
            name: "priority",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "role",
            kind: ColumnKind::Enum(FIRMWARE_ROLES),
            nullable: true,
            references: None,
        },
    ],
    key: &["fw"],
    regenerate: None,
    gate: FIRMWARE_GATE,
};

/// The test that validates `pup.tsv` and its archive relations.
pub const PUP_GATE: &str = "pup_rows_are_well_formed";

/// Defines `pup.tsv` with one curated row per acquired PUP image.
pub const PUP: TableSpec = TableSpec {
    name: "pup",
    owner: OwnerClass::Curated,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: None,
        },
        Column {
            name: "fw",
            kind: ColumnKind::Locator,
            nullable: false,
            references: Some(("firmware", "fw")),
        },
        Column {
            name: "size_bytes",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "image_version",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "source_note",
            kind: ColumnKind::Ident,
            nullable: false,
            references: None,
        },
        Column {
            name: "acquired",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
    ],
    key: &["pup_sha256"],
    regenerate: None,
    gate: PUP_GATE,
};
