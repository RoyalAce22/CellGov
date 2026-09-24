//! The handling tables: `arm.tsv`, `route.tsv` and the curated `behavior.tsv`, with their label sets.

use super::schema::{Column, ColumnKind, OwnerClass, TableSpec, GATE, REGENERATE};

/// Labels of `route.route`; [`Route::label`](crate::Route::label) pins the order.
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
