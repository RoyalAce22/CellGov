use super::*;

/// The `CREATE TABLE` statement for `table`, so a check on one table
/// cannot match a same-named column of another.
fn create_block<'a>(schema: &'a str, table: &str) -> &'a str {
    let open = format!("CREATE TABLE {table} (\n");
    let start = schema
        .find(&open)
        .unwrap_or_else(|| panic!("no CREATE TABLE for {table}"));
    let end = schema[start..]
        .find(") STRICT;\n")
        .unwrap_or_else(|| panic!("{table} is not STRICT"));
    &schema[start..start + end + ") STRICT;\n".len()]
}

#[test]
fn the_schema_creates_every_table_strict_with_its_checks_keys_and_references() {
    let full = schema_sql();
    for table in TABLES {
        let schema = create_block(&full, table.name);
        assert!(
            schema.ends_with(&format!(
                "    PRIMARY KEY ({})\n) STRICT;\n",
                table.key.join(", ")
            )),
            "{} is not STRICT with its key",
            table.name
        );
        for column in table.columns {
            if let ColumnKind::Enum(labels) = column.kind {
                let quoted: Vec<String> = labels.iter().map(|l| format!("'{l}'")).collect();
                let check = format!("CHECK ({} IN ({}))", column.name, quoted.join(", "));
                assert!(
                    schema.contains(&check),
                    "{}.{} has no {check}",
                    table.name,
                    column.name
                );
            }
            if let Some((target_table, target_column)) = column.references {
                let reference = format!(
                    "{} TEXT REFERENCES {target_table} ({target_column})",
                    column.name
                );
                assert!(
                    schema.contains(&reference),
                    "{}.{} lacks {reference}",
                    table.name,
                    column.name
                );
            }
            let not_null = format!("    {} {} NOT NULL", column.name, column.kind.sql_type());
            assert_eq!(
                schema.contains(&not_null),
                !column.nullable,
                "{}.{} nullability",
                table.name,
                column.name
            );
        }
    }
    for view in VIEWS {
        assert!(
            full.contains(&format!("CREATE VIEW {} AS\n{};\n", view.name, view.select)),
            "no view {}",
            view.name
        );
    }
    let schema_tables = full.matches("CREATE TABLE ").count();
    assert_eq!(schema_tables, TABLES.len());
}

#[test]
fn the_build_reads_the_schema_and_imports_every_table_through_staging_in_order() {
    let build = build_sql();
    assert!(build.starts_with("-- Rendered from cellgov_lv2::archive by\n"));
    assert!(build.contains("\n.bail on\nPRAGMA foreign_keys = ON;\n.read schema.sql\n"));
    let mut cursor = 0;
    for table in TABLES {
        let import = format!(
            ".import --ascii --colsep \"\\t\" --rowsep \"\\n\" --skip 1 {} staging_{}\n",
            table.file(),
            table.name
        );
        let at = build[cursor..]
            .find(&import)
            .unwrap_or_else(|| panic!("{} is not imported after the table before it", table.name));
        cursor += at + import.len();
        assert!(build.contains(&format!("DROP TABLE staging_{};\n", table.name)));
    }
    assert!(
        build.contains("NULLIF(arm, 'none')"),
        "nullable arm maps none to NULL"
    );
    assert!(
        build.contains("CAST(ordinal AS INTEGER)"),
        "integer ordinal is cast"
    );
    assert!(
        !build.contains("NULLIF(ordinal,"),
        "the key column takes no null"
    );
}
