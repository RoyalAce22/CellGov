//! The table list: each table's name, owner class, columns and key.

/// The command that rewrites every generated file of the archive.
pub const REGENERATE: &str = "cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate";

/// The test that fails when a committed generated file is stale.
pub const GATE: &str = "committed_archive_matches_generator";

/// Pins SQLite's `user_version` to the archive's frozen schema.
pub const SCHEMA_VERSION: u32 = 4;

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
    /// The command that rewrites the file, or the rows of it that are
    /// rendered from code; `None` for a table written by hand.
    pub regenerate: Option<&'static str>,
    /// The test that fails when the committed file is stale or wrong.
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

    /// Whether a key column takes [`NONE`](super::table::NONE).
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

/// Labels of `route.route`; [`Route::label`](super::Route::label) pins the order.
pub const ROUTE_LABELS: &[&str] = &["typed", "routed", "null_backend", "runtime_fast_path"];

/// Labels of `arm.fidelity`; `ArmFidelity::label` pins the order.
pub const FIDELITY_LABELS: &[&str] = &["modeled", "partial-state", "abi-only", "null-backend"];

/// Labels of `behavior.provenance_kind`: what a modelled behaviour rests on.
pub const PROVENANCE_KINDS: &[&str] = &[
    "citation",
    "firmware_reading",
    "console_capture",
    "non_public",
    "unestablished",
];

/// Labels for selector argument slots in dispatch tables.
pub const SELECTOR_SLOTS: &[&str] = &["r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10"];

/// Labels of `behavior.exception`: the standing departures from the null backend.
pub const EXCEPTIONS: &[&str] = &["fabricated_success"];

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
    regenerate: Some(REGENERATE),
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
    regenerate: Some(REGENERATE),
    gate: GATE,
};

/// The test that fails when `behavior.tsv` is wrong.
pub const BEHAVIOR_GATE: &str = "behavior_rows_cover_the_handled_surface";

/// `behavior.tsv`: one curated row per typed or routed ordinal.
pub const BEHAVIOR: TableSpec = TableSpec {
    name: "behavior",
    owner: OwnerClass::Curated,
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
            name: "same_as",
            kind: ColumnKind::Integer,
            nullable: true,
            references: Some(("route", "ordinal")),
        },
        Column {
            name: "selector_slot",
            kind: ColumnKind::Enum(SELECTOR_SLOTS),
            nullable: true,
            references: None,
        },
        Column {
            name: "provenance_kind",
            kind: ColumnKind::Enum(PROVENANCE_KINDS),
            nullable: false,
            references: None,
        },
        Column {
            name: "provenance_ref",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
        Column {
            name: "witness",
            kind: ColumnKind::Locator,
            nullable: true,
            references: None,
        },
        Column {
            name: "exception",
            kind: ColumnKind::Enum(EXCEPTIONS),
            nullable: true,
            references: None,
        },
        Column {
            name: "arm_source",
            kind: ColumnKind::Locator,
            nullable: false,
            references: None,
        },
    ],
    key: &["ordinal"],
    regenerate: None,
    gate: BEHAVIOR_GATE,
};

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

/// Provides the command that refreshes extracted rows for one kernel.
pub const CENSUS_REGENERATE: &str =
    "cargo run --release -p cellgov_cli -- dev lv2-census <ELF> --fw <VERSION> --pup-sha256 <SHA256> --output-dir docs/lv2";

/// Names the corpus-free gate for extracted kernel census rows.
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
    "cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate_cellgov_names";

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

/// Every table, a referenced table before the table that references it.
pub const TABLES: &[TableSpec] = &[
    FIRMWARE,
    PUP,
    ARM,
    ROUTE,
    KERNEL,
    STUB,
    SUBENTRY,
    CAPABILITY_GATE,
    SUBENTRY_ATTRIBUTION,
    CALLER,
    CALLER_UNRESOLVED,
    REACH,
    BEHAVIOR,
    NAME,
    CONFLICTS,
];

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
    /// The command that rewrites it; `None` for a file written by hand.
    pub regenerate: Option<&'static str>,
    /// The test that fails when the committed copy is stale or wrong.
    pub gate: &'static str,
}

/// Lists every archive file in file-name order.
///
/// The caller supplies the discovered per-version census paths.
pub fn manifest(census_files: &[String]) -> Vec<ManifestRow> {
    let mut rows: Vec<ManifestRow> = FIXED_FILES
        .iter()
        .map(|file| ManifestRow {
            file: (*file).to_string(),
            owner: OwnerClass::Generated,
            regenerate: Some(REGENERATE),
            gate: GATE,
        })
        .collect();
    rows.extend(TABLES.iter().map(|table| ManifestRow {
        file: table.file(),
        owner: table.owner,
        regenerate: table.regenerate,
        gate: table.gate,
    }));
    rows.extend(census_files.iter().map(|file| ManifestRow {
        file: file.clone(),
        owner: OwnerClass::Extracted,
        regenerate: Some(CENSUS_REGENERATE),
        gate: CENSUS_GATE,
    }));
    rows.sort_by(|a, b| a.file.cmp(&b.file));
    rows
}

/// Lists every archive file in file-name order.
///
/// The caller supplies the discovered per-version census paths.
pub fn files(census_files: &[String]) -> Vec<String> {
    manifest(census_files)
        .into_iter()
        .map(|row| row.file)
        .collect()
}

/// Every view.
pub const VIEWS: &[View] = &[
    View {
        name: "handling",
        select: "SELECT route.ordinal, route.route, route.arm, arm.fidelity\n\
                 FROM route\n\
                 LEFT JOIN arm ON arm.arm = route.arm",
    },
    View {
        name: "authority",
        select: "SELECT behavior.ordinal, route.arm, arm.fidelity,\n\
                 \x20      behavior.provenance_kind, behavior.provenance_ref,\n\
                 \x20      behavior.witness, behavior.exception, behavior.arm_source\n\
                 FROM behavior\n\
                 JOIN route ON route.ordinal = behavior.ordinal\n\
                 LEFT JOIN arm ON arm.arm = route.arm",
    },
];

#[cfg(test)]
#[path = "tests/spec_tests.rs"]
mod tests;
