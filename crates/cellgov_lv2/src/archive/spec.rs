//! The table list: each table's name, owner class, columns and key.

/// The command that rewrites every generated file of the archive.
pub const REGENERATE: &str = "cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate";

/// The test that fails when a committed generated file is stale.
pub const GATE: &str = "committed_archive_matches_generator";

/// Who writes a table, and under what discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerClass {
    /// Only the census emitter writes it, from firmware bytes.
    Extracted,
    /// A regenerate test renders it from code, and a drift gate holds the committed copy.
    Generated,
    /// CellGov's own claims, loader-validated, with provenance on every row.
    Curated,
    /// Community or non-public names, with a source on every row.
    Attributed,
}

impl OwnerClass {
    /// Every class, in the order the archive document lists them.
    pub const ALL: &[OwnerClass] = &[
        OwnerClass::Extracted,
        OwnerClass::Generated,
        OwnerClass::Curated,
        OwnerClass::Attributed,
    ];

    /// The lowercase label the archive document uses.
    pub fn label(self) -> &'static str {
        match self {
            OwnerClass::Extracted => "extracted",
            OwnerClass::Generated => "generated",
            OwnerClass::Curated => "curated",
            OwnerClass::Attributed => "attributed",
        }
    }

    /// One-line meaning of the class, as the archive document states it.
    pub fn meaning(self) -> &'static str {
        match self {
            OwnerClass::Extracted => {
                "Written only by the census emitter from firmware bytes, under anchor discipline."
            }
            OwnerClass::Generated => {
                "Rendered from code by a regenerate test; a drift gate fails when the committed copy is stale."
            }
            OwnerClass::Curated => {
                "CellGov's own claims, loader-validated, with provenance on every row."
            }
            OwnerClass::Attributed => {
                "Community or non-public names, with a source on every row, never merged into extracted rows."
            }
        }
    }
}

/// The value shape of one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// A decimal unsigned integer: digits only, no leading zero past a
    /// lone `0`, at most `i64::MAX` (SQLite's INTEGER).
    Integer,
    /// One token of ASCII letters, digits and `_`.
    Ident,
    /// [`Integer`](ColumnKind::Integer) tokens joined by `,`, strictly ascending.
    IntegerList,
    /// One label from a fixed set.
    Enum(&'static [&'static str]),
}

impl ColumnKind {
    /// The SQL type the column takes in `schema.sql`.
    pub fn sql_type(self) -> &'static str {
        match self {
            ColumnKind::Integer => "INTEGER",
            ColumnKind::Ident | ColumnKind::IntegerList | ColumnKind::Enum(_) => "TEXT",
        }
    }

    /// What a cell of this kind has to be, for the loader's refusal.
    pub fn describe(self) -> String {
        match self {
            ColumnKind::Integer => "a decimal integer".to_string(),
            ColumnKind::Ident => "an identifier of letters, digits and _".to_string(),
            ColumnKind::IntegerList => {
                "an ascending comma-joined list of decimal integers".to_string()
            }
            ColumnKind::Enum(labels) => format!("one of {}", labels.join(", ")),
        }
    }
}

/// One column of a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    /// The header cell and the SQL column name.
    pub name: &'static str,
    /// The shape every non-null cell has.
    pub kind: ColumnKind,
    /// Whether a cell may read [`NONE`](super::table::NONE), the SQL NULL.
    pub nullable: bool,
    /// `(table, column)` every non-null value has to exist in.
    pub references: Option<(&'static str, &'static str)>,
}

/// One table of the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    /// The file stem and the SQL table name.
    pub name: &'static str,
    /// Who writes the file.
    pub owner: OwnerClass,
    /// The columns, in file order.
    pub columns: &'static [Column],
    /// Primary key: rows sort by it and no two rows share it.
    pub key: &'static [&'static str],
    /// The command that rewrites the file.
    pub regenerate: &'static str,
    /// The test that fails when the committed file is stale.
    pub gate: &'static str,
}

impl TableSpec {
    /// The file name under `docs/lv2/`.
    pub fn file(&self) -> String {
        format!("{}.tsv", self.name)
    }

    /// Index of each key column in [`TableSpec::columns`].
    pub fn key_indexes(&self) -> Vec<usize> {
        self.key
            .iter()
            .filter_map(|k| self.columns.iter().position(|c| c.name == *k))
            .collect()
    }
}

/// A join-only view rendered into `schema.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// The SQL view name.
    pub name: &'static str,
    /// The `SELECT` body, a join over the tables and nothing else.
    pub select: &'static str,
}

/// Labels of `route.route`; [`Route::label`](super::Route::label) pins the order.
pub const ROUTE_LABELS: &[&str] = &["typed", "routed", "null_backend", "runtime_fast_path"];

/// Labels of `arm.fidelity`; `ArmFidelity::label` pins the order.
pub const FIDELITY_LABELS: &[&str] = &["modeled", "partial-state", "abi-only", "null-backend"];

/// `arm.tsv`: one row per dispatch arm.
pub const ARM: TableSpec = TableSpec {
    name: "arm",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "arm",
            kind: ColumnKind::Ident,
            nullable: false,
            references: None,
        },
        Column {
            name: "fidelity",
            kind: ColumnKind::Enum(FIDELITY_LABELS),
            nullable: false,
            references: None,
        },
        Column {
            name: "ordinals",
            kind: ColumnKind::IntegerList,
            nullable: true,
            references: None,
        },
    ],
    key: &["arm"],
    regenerate: REGENERATE,
    gate: GATE,
};

/// `route.tsv`: one row per LV2 syscall slot.
pub const ROUTE: TableSpec = TableSpec {
    name: "route",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "ordinal",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "route",
            kind: ColumnKind::Enum(ROUTE_LABELS),
            nullable: false,
            references: None,
        },
        Column {
            name: "arm",
            kind: ColumnKind::Ident,
            nullable: true,
            references: Some(("arm", "arm")),
        },
    ],
    key: &["ordinal"],
    regenerate: REGENERATE,
    gate: GATE,
};

/// Every table, a referenced table before the table that references it.
pub const TABLES: &[TableSpec] = &[ARM, ROUTE];

/// The files under `docs/lv2/` that are not tables; the one regenerate
/// command writes all of them.
const FIXED_FILES: &[&str] = &["README.md", "schema.sql", "build.sql"];

/// One row of the archive document's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRow {
    /// The file name under `docs/lv2/`.
    pub file: String,
    /// Who writes it.
    pub owner: OwnerClass,
    /// The command that rewrites it.
    pub regenerate: &'static str,
    /// The test that fails when the committed copy is stale.
    pub gate: &'static str,
}

/// Every file the archive holds, one row each, sorted by file name.
pub fn manifest() -> Vec<ManifestRow> {
    let mut rows: Vec<ManifestRow> = FIXED_FILES
        .iter()
        .map(|file| ManifestRow {
            file: (*file).to_string(),
            owner: OwnerClass::Generated,
            regenerate: REGENERATE,
            gate: GATE,
        })
        .collect();
    rows.extend(TABLES.iter().map(|table| ManifestRow {
        file: table.file(),
        owner: table.owner,
        regenerate: table.regenerate,
        gate: table.gate,
    }));
    rows.sort_by(|a, b| a.file.cmp(&b.file));
    rows
}

/// Every file the archive holds, sorted.
pub fn files() -> Vec<String> {
    manifest().into_iter().map(|row| row.file).collect()
}

/// Every view.
pub const VIEWS: &[View] = &[View {
    name: "handling",
    select: "SELECT route.ordinal, route.route, route.arm, arm.fidelity\n\
             FROM route\n\
             LEFT JOIN arm ON arm.arm = route.arm",
}];

#[cfg(test)]
#[path = "tests/spec_tests.rs"]
mod tests;
