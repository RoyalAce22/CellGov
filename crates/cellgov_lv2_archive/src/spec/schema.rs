//! The schema vocabulary: owner classes, column kinds, columns, tables and views, plus the shared regenerate command and gate.

/// The command that rewrites every generated file of the archive.
pub const REGENERATE: &str =
    "cargo test -p cellgov_lv2_archive --test lv2_archive -- --ignored regenerate";

/// The test that fails when a committed generated file is stale.
pub const GATE: &str = "committed_archive_matches_generator";

/// Pins SQLite's `user_version` to the archive's frozen schema.
pub const SCHEMA_VERSION: u32 = 7;

/// Who writes a table, and under what discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerClass {
    /// Only the census emitter writes it, from firmware bytes.
    Extracted,
    /// A regenerate test renders it from code, and a drift gate holds the committed copy.
    Generated,
    /// CellGov's own claims, loader-validated, with provenance on every row.
    Curated,
    /// Every row names the source of its fact.
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
                "Community or non-public facts, with a source on every row, never merged into extracted rows."
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
    /// Firmware version keys joined by `,`, strictly ascending.
    VersionList,
    /// Accepts exactly 64 lowercase hexadecimal digits.
    Sha256,
    /// Accepts `0x` followed by exactly 8 lowercase hexadecimal digits.
    Hex32,
    /// Accepts `0x` followed by exactly 16 lowercase hexadecimal digits.
    Hex64,
    /// One label from a fixed set.
    Enum(&'static [&'static str]),
    /// One token of ASCII letters, digits and `_ . / : @ + -`, wide
    /// enough for each locator the archive stores:
    /// - a file path;
    /// - a `path:function` witness;
    /// - a `DOC-KEY:p:N` citation.
    Locator,
}

impl ColumnKind {
    /// The SQL type the column takes in `schema.sql`.
    pub fn sql_type(self) -> &'static str {
        match self {
            ColumnKind::Integer => "INTEGER",
            ColumnKind::Ident
            | ColumnKind::IntegerList
            | ColumnKind::VersionList
            | ColumnKind::Sha256
            | ColumnKind::Hex32
            | ColumnKind::Hex64
            | ColumnKind::Enum(_)
            | ColumnKind::Locator => "TEXT",
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
            ColumnKind::VersionList => {
                "an ascending comma-joined list of firmware version keys".to_string()
            }
            ColumnKind::Sha256 => "64 lowercase hexadecimal digits".to_string(),
            ColumnKind::Hex32 => "0x plus 8 lowercase hexadecimal digits".to_string(),
            ColumnKind::Hex64 => "0x plus 16 lowercase hexadecimal digits".to_string(),
            ColumnKind::Enum(labels) => format!("one of {}", labels.join(", ")),
            ColumnKind::Locator => "a locator of letters, digits and _ . / : @ + -".to_string(),
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
    /// Whether a cell may read [`NONE`](crate::table::NONE), the SQL NULL.
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
    /// The command that rewrites the file, or the rows of it that are
    /// rendered from code; `None` for a table written by hand.
    pub regenerate: Option<&'static str>,
    /// The test that fails when the committed file is stale or wrong.
    pub gate: &'static str,
}

impl TableSpec {
    /// The file name under `docs/lv2/`.
    pub fn file(&self) -> String {
        format!("tables/{}.tsv", self.name)
    }

    /// Index of each key column in [`TableSpec::columns`].
    pub fn key_indexes(&self) -> Vec<usize> {
        self.key
            .iter()
            .filter_map(|k| self.columns.iter().position(|c| c.name == *k))
            .collect()
    }

    /// Whether a key column takes [`NONE`](crate::table::NONE).
    pub fn key_is_nullable(&self) -> bool {
        self.key_indexes()
            .into_iter()
            .any(|i| self.columns[i].nullable)
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
