//! `schema.sql` and `build.sql`, rendered from the table list.
//!
//! The database holds the shared firmware and PUP tables too, so each
//! reference resolves inside it.

use cellgov_lv2_archive::{create_table_sql, import_sql, sql_banner};

use super::spec::{GATE, REGENERATE, SCHEMA_VERSION, SHARED_TABLES, TABLES};

/// Where `build.sql` reads the shared tables from, relative to
/// `docs/lv1/`.
pub const SHARED_TABLE_DIR: &str = "../lv2";

fn banner() -> String {
    sql_banner("cellgov_lv1_archive", REGENERATE, GATE)
}

/// The `schema.sql` text: one STRICT table per shared table, then one per
/// lv1 table.
pub fn schema_sql() -> String {
    let mut out = banner();
    out.push_str(&format!("\nPRAGMA user_version = {SCHEMA_VERSION};\n"));
    for table in SHARED_TABLES.iter().chain(TABLES) {
        out.push('\n');
        out.push_str(&create_table_sql(table));
    }
    out
}

/// Renders `build.sql`: the shared tables from [`SHARED_TABLE_DIR`], then
/// the lv1 tables.
///
/// The script imports through staging tables and stops at the first error.
pub fn build_sql() -> String {
    let mut out = banner();
    out.push_str(
        "-- Run in docs/lv1/: sqlite3 lv1.db < sql/build.sql\n\
         \n\
         .bail on\n\
         PRAGMA foreign_keys = ON;\n\
         .read sql/schema.sql\n",
    );
    for table in SHARED_TABLES {
        out.push_str(&import_sql(
            table,
            &format!("{SHARED_TABLE_DIR}/{}", table.file()),
        ));
    }
    for table in TABLES {
        out.push_str(&import_sql(table, &table.file()));
    }
    out
}

#[cfg(test)]
#[path = "tests/sql_tests.rs"]
mod tests;
