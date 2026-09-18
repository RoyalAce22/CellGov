//! `schema.sql` and `build.sql`, rendered from the table list.

use super::spec::{ColumnKind, TableSpec, CENSUS, GATE, REGENERATE, SCHEMA_VERSION, TABLES, VIEWS};
use super::table::NONE;

/// The sqlite3 shell version `build.sql` targets.
pub const SQLITE_VERSION: &str = "3.53.0";

fn banner() -> String {
    format!(
        "-- Rendered from cellgov_lv2::archive by\n\
         --   {REGENERATE}\n\
         -- Do not edit by hand: {GATE} fails on drift.\n"
    )
}

/// Quotes every column name so the reserved `order` name needs no special case.
fn quoted(column: &str) -> String {
    format!("\"{column}\"")
}

fn quoted_list(columns: &[&str]) -> String {
    let quoted: Vec<String> = columns.iter().map(|c| quoted(c)).collect();
    quoted.join(", ")
}

fn create_table(table: &TableSpec) -> String {
    let mut lines = Vec::new();
    for column in table.columns {
        let name = quoted(column.name);
        let mut line = format!("    {name} {}", column.kind.sql_type());
        if !column.nullable {
            line.push_str(" NOT NULL");
        }
        if let ColumnKind::Enum(labels) = column.kind {
            let list: Vec<String> = labels.iter().map(|l| format!("'{l}'")).collect();
            line.push_str(&format!(" CHECK ({name} IN ({}))", list.join(", ")));
        }
        if let Some((target_table, target_column)) = column.references {
            line.push_str(&format!(
                " REFERENCES {target_table} ({})",
                quoted(target_column)
            ));
        }
        lines.push(line);
    }
    // A STRICT table refuses a null in a PRIMARY KEY column whatever
    // the column says, so a key with a nullable column is UNIQUE
    // instead. SQLite treats two nulls as distinct there; the loader
    // alone catches a repeated key with a `none` in it.
    let constraint = if table.key_is_nullable() {
        "UNIQUE"
    } else {
        "PRIMARY KEY"
    };
    lines.push(format!("    {constraint} ({})", quoted_list(table.key)));
    format!(
        "CREATE TABLE {} (\n{}\n) STRICT;\n",
        table.name,
        lines.join(",\n")
    )
}

/// The `schema.sql` text: one STRICT table per spec, then the views.
pub fn schema_sql() -> String {
    let mut out = banner();
    out.push_str(&format!("\nPRAGMA user_version = {SCHEMA_VERSION};\n"));
    for table in TABLES {
        out.push('\n');
        out.push_str(&create_table(table));
    }
    out.push('\n');
    out.push_str(&create_table(&CENSUS));
    for view in VIEWS {
        out.push_str(&format!(
            "\nCREATE VIEW {} AS\n{};\n",
            view.name, view.select
        ));
    }
    out
}

/// Renders `build.sql` for the fixed tables and supplied census files.
///
/// The script imports through staging tables and stops at the first error.
pub fn build_sql(census_files: &[String]) -> String {
    let mut out = banner();
    out.push_str(
        "-- Run in docs/lv2/: sqlite3 lv2.db < build.sql\n\
         \n\
         .bail on\n\
         PRAGMA foreign_keys = ON;\n\
         .read schema.sql\n",
    );
    for table in TABLES {
        append_import(&mut out, table, &table.file());
    }
    // Sorting preserves byte-deterministic output when the caller discovers
    // the per-version files in filesystem order.
    let mut census_files = census_files.to_vec();
    census_files.sort();
    for file in &census_files {
        append_import(&mut out, &CENSUS, file);
    }
    out
}

fn append_import(out: &mut String, table: &TableSpec, file: &str) {
    let staging = format!("staging_{}", table.name);
    let names: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
    let typed: Vec<String> = names
        .iter()
        .map(|n| format!("{} TEXT", quoted(n)))
        .collect();
    let selected: Vec<String> = table
        .columns
        .iter()
        .map(|column| {
            let mut expr = quoted(column.name);
            if column.nullable {
                expr = format!("NULLIF({expr}, '{NONE}')");
            }
            if column.kind == ColumnKind::Integer {
                expr = format!("CAST({expr} AS INTEGER)");
            }
            expr
        })
        .collect();
    out.push_str(&format!(
        "\n\
             CREATE TEMP TABLE {staging} ({});\n\
             .import --ascii --colsep \"\\t\" --rowsep \"\\n\" --skip 1 {} {staging}\n\
             INSERT INTO {} ({})\n\
             SELECT {}\n\
             FROM {staging};\n\
             DROP TABLE {staging};\n",
        typed.join(", "),
        file,
        table.name,
        quoted_list(&names),
        selected.join(", "),
    ));
}

#[cfg(test)]
#[path = "tests/sql_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/census_sql_tests.rs"]
mod census_tests;
