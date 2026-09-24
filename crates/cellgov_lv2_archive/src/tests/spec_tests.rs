use super::*;
use crate::firmware::FirmwareRole;
use crate::handling::Route;
use crate::name::{Disagreement, NameSource};
use cellgov_lv2::request::fidelity::ArmFidelity;

fn all_specs() -> Vec<&'static TableSpec> {
    TABLES.iter().chain(std::iter::once(&CENSUS)).collect()
}

#[test]
fn a_referenced_table_precedes_every_table_that_references_it() {
    let tables = all_specs();
    for (index, table) in tables.iter().enumerate() {
        for column in table.columns {
            let Some((target, _)) = column.references else {
                continue;
            };
            let position = tables.iter().position(|t| t.name == target);
            assert!(
                position.is_some_and(|p| p < index),
                "{}.{} references {target}, which is not listed before it",
                table.name,
                column.name
            );
        }
    }
}

#[test]
fn every_key_and_reference_names_a_column() {
    let tables = all_specs();
    for table in &tables {
        assert_eq!(
            table.key_indexes().len(),
            table.key.len(),
            "{}: a key column is not a column",
            table.name
        );
        for column in table.columns {
            if let Some((target_table, target_column)) = column.references {
                let target = tables
                    .iter()
                    .find(|t| t.name == target_table)
                    .unwrap_or_else(|| panic!("{target_table} is not a table"));
                assert!(
                    target.columns.iter().any(|c| c.name == target_column),
                    "{target_table} has no column {target_column}"
                );
            }
        }
    }
}

#[test]
fn the_enum_labels_are_the_code_labels_in_order() {
    let routes: Vec<&str> = Route::ALL.iter().map(|r| r.label()).collect();
    assert_eq!(routes, ROUTE_LABELS);
    let fidelities: Vec<&str> = ArmFidelity::ALL.iter().map(|f| f.label()).collect();
    assert_eq!(fidelities, FIDELITY_LABELS);
    let sources: Vec<&str> = NameSource::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(sources, NAME_SOURCES);
    let disagreements: Vec<&str> = Disagreement::ALL.iter().map(|d| d.label()).collect();
    assert_eq!(disagreements, DISAGREEMENTS);
    // The null cell means no role, so it is not an enum label.
    let roles: Vec<&str> = FirmwareRole::ALL
        .iter()
        .filter(|r| **r != FirmwareRole::None)
        .map(|r| r.label())
        .collect();
    assert_eq!(roles, FIRMWARE_ROLES);
    assert_eq!(FirmwareRole::None.label(), crate::NONE);
}

#[test]
fn only_the_name_tables_have_a_nullable_key() {
    let nullable: Vec<&str> = TABLES
        .iter()
        .filter(|t| t.key_is_nullable())
        .map(|t| t.name)
        .collect();
    assert_eq!(
        nullable,
        ["name", "conflicts"],
        "the firmware key is not nullable"
    );
}

#[test]
fn the_manifest_lists_every_table_and_the_fixed_files_once() {
    let census = vec!["census/fw-3.55.tsv".to_string()];
    let files = files(&census);
    let mut sorted = files.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(files, sorted, "manifest is sorted and free of repeats");
    for table in TABLES {
        assert!(
            files.contains(&table.file()),
            "{} has no manifest row",
            table.file()
        );
    }
    assert!(files.contains(&census[0]));
    for fixed in ["README.md", "sql/schema.sql", "sql/build.sql"] {
        assert!(
            files.iter().any(|f| f == fixed),
            "{fixed} has no manifest row"
        );
    }
    assert_eq!(files.len(), TABLES.len() + 4);
}

#[test]
fn every_class_has_a_distinct_label() {
    let labels: Vec<&str> = OwnerClass::ALL.iter().map(|c| c.label()).collect();
    assert_eq!(labels, ["extracted", "generated", "curated", "attributed"]);
}
