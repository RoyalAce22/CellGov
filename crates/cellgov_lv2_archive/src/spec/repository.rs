//! The LV1 repository tables: `repository_read.tsv` and
//! `repository.tsv`.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec, GATE, REGENERATE};

/// Rewrites `repository_read.tsv`: `cellgov dev repository-census` over
/// each logged firmware's stored kernel.
pub const REPOSITORY_REGENERATE: &str = "CELLGOV_REGENERATE=1 cargo test -p cellgov_cli --features installed-firmware-tests --test repository_census_firmware";

/// Checks `repository_read.tsv` against the node table.
pub const REPOSITORY_GATE: &str = "repository_rows_are_well_formed";

/// The firmware versions whose kernel boots `repository_read.tsv` logs.
pub const REPOSITORY_LOGGED_FIRMWARE: &[&str] = &["1.50", "1.94", "3.70", "4.93"];

/// Labels of `repository_read.state`.
pub const REPOSITORY_READ_STATES: &[&str] = &["answered", "refused"];

/// Labels of `repository.source`: the classes of source a node value may
/// cite.
pub const REPOSITORY_SOURCES: &[&str] = &["cellgov", "console", "linux", "psdevwiki"];

const fn key_word(name: &'static str) -> Column {
    Column {
        name,
        kind: ColumnKind::Hex64,
        nullable: false,
        references: None,
    }
}

const LPAR: Column = Column {
    name: "lpar",
    kind: ColumnKind::Integer,
    nullable: false,
    references: None,
};

const NODE: Column = Column {
    name: "node",
    kind: ColumnKind::Locator,
    nullable: false,
    references: None,
};

/// Defines `repository_read.tsv`: each logged kernel boot's repository
/// reads, one row per key.
pub const REPOSITORY_READ: TableSpec = TableSpec {
    name: "repository_read",
    owner: OwnerClass::Extracted,
    columns: &[
        Column {
            name: "pup_sha256",
            kind: ColumnKind::Sha256,
            nullable: false,
            references: Some(("pup", "pup_sha256")),
        },
        LPAR,
        key_word("n1"),
        key_word("n2"),
        key_word("n3"),
        key_word("n4"),
        NODE,
        Column {
            name: "first",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "state",
            kind: ColumnKind::Enum(REPOSITORY_READ_STATES),
            nullable: false,
            references: None,
        },
    ],
    key: &["pup_sha256", "lpar", "n1", "n2", "n3", "n4"],
    regenerate: Some(REPOSITORY_REGENERATE),
    gate: REPOSITORY_GATE,
};

/// Defines `repository.tsv`: one row per node CellGov's hypervisor
/// serves, with its values, the firmware whose boot reads it and its
/// source.
pub const REPOSITORY: TableSpec = TableSpec {
    name: "repository",
    owner: OwnerClass::Generated,
    columns: &[
        LPAR,
        key_word("n1"),
        key_word("n2"),
        key_word("n3"),
        key_word("n4"),
        NODE,
        key_word("v1"),
        key_word("v2"),
        Column {
            name: "fw",
            kind: ColumnKind::VersionList,
            nullable: false,
            references: None,
        },
        Column {
            name: "source",
            kind: ColumnKind::Enum(REPOSITORY_SOURCES),
            nullable: false,
            references: None,
        },
        Column {
            name: "citation",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
        Column {
            name: "section",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
    ],
    key: &["lpar", "n1", "n2", "n3", "n4"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};
