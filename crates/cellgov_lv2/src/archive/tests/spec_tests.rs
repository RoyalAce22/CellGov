use super::*;
use crate::archive::handling::Route;
use crate::archive::name::{Disagreement, NameSource};
use crate::request::fidelity::ArmFidelity;

#[test]
fn a_referenced_table_precedes_every_table_that_references_it() {
    for (index, table) in TABLES.iter().enumerate() {
        for column in table.columns {
            let Some((target, _)) = column.references else {
                continue;
            };
            let position = TABLES.iter().position(|t| t.name == target);
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
    for table in TABLES {
        assert_eq!(
            table.key_indexes().len(),
            table.key.len(),
            "{}: a key column is not a column",
            table.name
        );
        for column in table.columns {
            if let Some((target_table, target_column)) = column.references {
                let target = TABLES
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
}

#[test]
fn only_the_name_tables_have_a_nullable_key() {
    let nullable: Vec<&str> = TABLES
        .iter()
        .filter(|t| t.key_is_nullable())
        .map(|t| t.name)
        .collect();
    assert_eq!(nullable, ["name", "conflicts"]);
}

#[test]
fn the_manifest_lists_every_table_and_the_fixed_files_once() {
    let files = files();
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
    for fixed in ["README.md", "schema.sql", "build.sql"] {
        assert!(
            files.iter().any(|f| f == fixed),
            "{fixed} has no manifest row"
        );
    }
    assert_eq!(files.len(), TABLES.len() + 3);
}

#[test]
fn every_class_has_a_distinct_label() {
    let labels: Vec<&str> = OwnerClass::ALL.iter().map(|c| c.label()).collect();
    assert_eq!(labels, ["extracted", "generated", "curated", "attributed"]);
}
