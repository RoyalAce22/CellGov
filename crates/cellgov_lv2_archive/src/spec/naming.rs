//! The naming tables: `name.tsv`, `conflicts.tsv` and `priority.tsv`.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec, GATE, REGENERATE};

/// Labels of `name.source`; `NameSource::label` pins the order.
pub const NAME_SOURCES: &[&str] = &["psdevwiki", "psl1ght", "cellgov", "non_public"];

/// Labels of `conflicts.disagreement`: how the names of one ordinal differ.
pub const DISAGREEMENTS: &[&str] = &["spelling", "name"];

/// The test that fails when the `cellgov` rows of `name.tsv` are not
/// the macro's.
pub const NAME_GATE: &str = "cellgov_name_rows_match_the_macro";

/// The command that rewrites the `cellgov` rows of `name.tsv` and
/// leaves every other row as it is.
pub const NAME_REGENERATE: &str =
    "cargo test -p cellgov_lv2_archive --test lv2_archive -- --ignored regenerate_cellgov_names";

/// `name.tsv`: every name a committed source gives an ordinal, one
/// row per (ordinal, packet, source, name).
pub const NAME: TableSpec = TableSpec {
    name: "name",
    owner: OwnerClass::Attributed,
    columns: &[
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "packet",
            kind: ColumnKind::Ident,
            nullable: true,
            references: None,
        },
        Column {
            name: "name",
            kind: ColumnKind::Ident,
            nullable: false,
            references: None,
        },
        Column {
            name: "source",
            kind: ColumnKind::Enum(NAME_SOURCES),
            nullable: false,
            references: None,
        },
        Column {
            name: "ref",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
        Column {
            name: "fw_from",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
        Column {
            name: "fw_to",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
    ],
    key: &["ordinal", "packet", "source", "name"],
    regenerate: Some(NAME_REGENERATE),
    gate: NAME_GATE,
};

/// `conflicts.tsv`: the rows of `name.tsv` whose ordinal and packet
/// carry more than one distinct name, each tagged with how the names
/// differ.
pub const CONFLICTS: TableSpec = TableSpec {
    name: "conflicts",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "packet",
            kind: ColumnKind::Ident,
            nullable: true,
            references: None,
        },
        Column {
            name: "name",
            kind: ColumnKind::Ident,
            nullable: false,
            references: None,
        },
        Column {
            name: "source",
            kind: ColumnKind::Enum(NAME_SOURCES),
            nullable: false,
            references: None,
        },
        Column {
            name: "disagreement",
            kind: ColumnKind::Enum(DISAGREEMENTS),
            nullable: false,
            references: None,
        },
    ],
    key: &["ordinal", "packet", "source", "name"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};

/// `priority.tsv`: evidence-ranked unmodelled syscall work from title anchors.
pub const PRIORITY: TableSpec = TableSpec {
    name: "priority",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "ranking",
            kind: ColumnKind::Enum(&["title_count", "earliness", "caller_modules", "census_gap"]),
            nullable: false,
            references: None,
        },
        Column {
            name: "rank",
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
        Column {
            name: "title_count",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "first_tick",
            kind: ColumnKind::Integer,
            nullable: true,
            references: None,
        },
        Column {
            name: "caller_modules",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
    ],
    key: &["ranking", "rank"],
    regenerate: Some(REGENERATE),
    gate: GATE,
};
