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
    assert!(full.contains(&format!("PRAGMA user_version = {SCHEMA_VERSION};")));
    for table in TABLES.iter().chain(std::iter::once(&CENSUS)) {
        let schema = create_block(&full, table.name);
        let constraint = if table.key_is_nullable() {
            "UNIQUE"
        } else {
            "PRIMARY KEY"
        };
        assert!(
            schema.ends_with(&format!(
                "    {constraint} ({})\n) STRICT;\n",
                quoted_list(table.key)
            )),
            "{} is not STRICT with its key",
            table.name
        );
        for column in table.columns {
            if let ColumnKind::Enum(labels) = column.kind {
                let quoted: Vec<String> = labels.iter().map(|l| format!("'{l}'")).collect();
                let check = format!(
                    "CHECK ({} IN ({}))",
                    super::quoted(column.name),
                    quoted.join(", ")
                );
                assert!(
                    schema.contains(&check),
                    "{}.{} has no {check}",
                    table.name,
                    column.name
                );
            }
            if let Some((target_table, target_column)) = column.references {
                let reference = format!(
                    "{} {}{} REFERENCES {target_table} ({})",
                    super::quoted(column.name),
                    column.kind.sql_type(),
                    if column.nullable { "" } else { " NOT NULL" },
                    super::quoted(target_column)
                );
                assert!(
                    schema.contains(&reference),
                    "{}.{} lacks {reference}",
                    table.name,
                    column.name
                );
            }
            let not_null = format!(
                "    {} {} NOT NULL",
                super::quoted(column.name),
                column.kind.sql_type()
            );
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
    assert_eq!(schema_tables, TABLES.len() + 1);
    assert!(
        create_block(&full, "name")
            .contains("    UNIQUE (\"ordinal\", \"packet\", \"source\", \"name\")\n"),
        "a key with a nullable column is UNIQUE, not PRIMARY KEY"
    );
    assert!(create_block(&full, "route").contains("    PRIMARY KEY (\"ordinal\")\n"));
    assert!(
        create_block(&full, "firmware").contains("    \"order\" INTEGER NOT NULL,\n"),
        "a reserved word survives as a column name only quoted"
    );
}

#[test]
fn the_build_reads_the_schema_and_imports_every_table_through_staging_in_order() {
    let census_files = vec![
        "census/fw-3.55.tsv".to_string(),
        "census/fw-3.56.tsv".to_string(),
    ];
    let build = build_sql(&census_files);
    assert!(build.starts_with("-- Rendered from cellgov_lv2::archive by\n"));
    assert!(build.contains("\n.bail on\nPRAGMA foreign_keys = ON;\n.read sql/schema.sql\n"));
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
    for file in &census_files {
        assert!(build.contains(&format!(
            ".import --ascii --colsep \"\\t\" --rowsep \"\\n\" --skip 1 {file} staging_census\n"
        )));
    }
    assert!(
        build.contains("NULLIF(\"arm\", 'none')"),
        "nullable arm maps none to NULL"
    );
    assert!(
        build.contains("CAST(\"ordinal\" AS INTEGER)"),
        "integer ordinal is cast"
    );
    assert!(
        build.contains("NULLIF(\"ordinal\", 'none')"),
        "the transition ordinal maps none to NULL"
    );
}
