use super::*;

#[test]
fn the_schema_creates_the_shared_tables_and_then_every_lv1_table() {
    let schema = schema_sql();
    assert!(schema.starts_with("-- Rendered from cellgov_lv1_archive by\n"));
    assert!(schema.contains(&format!("PRAGMA user_version = {SCHEMA_VERSION};")));
    let mut cursor = 0;
    for table in SHARED_TABLES.iter().chain(TABLES) {
        let statement = create_table_sql(table);
        let at = schema[cursor..]
            .find(&statement)
            .unwrap_or_else(|| panic!("{} is not created after the table before it", table.name));
        cursor += at + statement.len();
    }
    assert_eq!(
        schema.matches("CREATE TABLE ").count(),
        SHARED_TABLES.len() + TABLES.len()
    );
}

#[test]
fn the_build_imports_the_shared_tables_from_the_lv2_archive_and_then_every_lv1_table() {
    let build = build_sql();
    assert!(build.contains("\n.bail on\nPRAGMA foreign_keys = ON;\n.read sql/schema.sql\n"));
    let mut cursor = 0;
    let shared = SHARED_TABLES
        .iter()
        .map(|table| (table, format!("../lv2/{}", table.file())));
    let own = TABLES.iter().map(|table| (table, table.file()));
    for (table, file) in shared.chain(own) {
        let import = import_sql(table, &file);
        let at = build[cursor..]
            .find(&import)
            .unwrap_or_else(|| panic!("{} is not imported after the table before it", table.name));
        cursor += at + import.len();
    }
    assert_eq!(
        build.matches(".import ").count(),
        SHARED_TABLES.len() + TABLES.len()
    );
}
