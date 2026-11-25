use super::*;
use cellgov_lv2_archive::TableSpec;

/// The shared tables, then this archive's tables, in reference order.
fn all_specs() -> Vec<&'static TableSpec> {
    SHARED_TABLES.iter().chain(TABLES).collect()
}

/// pup.fw, and the pup_sha256 of hypercall_site, hypercall_unresolved and
/// repository_read.
const REFERENCE_COUNT: usize = 4;

/// The number of columns of `tables` that reference a table.
fn reference_count(tables: &[&TableSpec]) -> usize {
    tables
        .iter()
        .flat_map(|t| t.columns)
        .filter(|c| c.references.is_some())
        .count()
}

#[test]
fn a_referenced_table_precedes_every_table_that_references_it() {
    let tables = all_specs();
    assert_eq!(reference_count(&tables), REFERENCE_COUNT);
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
    assert_eq!(reference_count(&tables), REFERENCE_COUNT);
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
fn no_table_has_a_nullable_key() {
    let nullable: Vec<&str> = TABLES
        .iter()
        .filter(|t| t.key_is_nullable())
        .map(|t| t.name)
        .collect();
    assert_eq!(nullable, Vec::<&str>::new());
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
    for shared in SHARED_TABLES {
        assert!(
            !files.contains(&shared.file()),
            "{} is a file of the LV2 archive",
            shared.file()
        );
    }
    for fixed in ["README.md", "sql/schema.sql", "sql/build.sql"] {
        assert!(
            files.iter().any(|f| f == fixed),
            "{fixed} has no manifest row"
        );
    }
    assert_eq!(files.len(), TABLES.len() + 3);
}
