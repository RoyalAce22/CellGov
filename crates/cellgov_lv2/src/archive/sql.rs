//! `schema.sql` and `build.sql`, rendered from the table list.

use super::spec::{ColumnKind, TableSpec, GATE, REGENERATE, TABLES, VIEWS};
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

fn create_table(table: &TableSpec) -> String {
    let mut lines = Vec::new();
    for column in table.columns {
        let mut line = format!("    {} {}", column.name, column.kind.sql_type());
        if !column.nullable {
            line.push_str(" NOT NULL");
        }
        if let ColumnKind::Enum(labels) = column.kind {
            let list: Vec<String> = labels.iter().map(|l| format!("'{l}'")).collect();
            line.push_str(&format!(
                " CHECK ({} IN ({}))",
                column.name,
                list.join(", ")
            ));
        }
        if let Some((target_table, target_column)) = column.references {
            line.push_str(&format!(" REFERENCES {target_table} ({target_column})"));
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
    lines.push(format!("    {constraint} ({})", table.key.join(", ")));
    format!(
        "CREATE TABLE {} (\n{}\n) STRICT;\n",
        table.name,
        lines.join(",\n")
    )
}

/// The `schema.sql` text: one STRICT table per spec, then the views.
pub fn schema_sql() -> String {
    let mut out = banner();
    for table in TABLES {
        out.push('\n');
        out.push_str(&create_table(table));
    }
    for view in VIEWS {
        out.push_str(&format!(
            "\nCREATE VIEW {} AS\n{};\n",
            view.name, view.select
        ));
    }
    out
}

/// The `build.sql` text: read the schema, then import every table
/// through a staging table; the first error stops the run.
pub fn build_sql() -> String {
    let mut out = banner();
    out.push_str(
        "-- Run in docs/lv2/: sqlite3 lv2.db < build.sql\n\
         \n\
         .bail on\n\
         PRAGMA foreign_keys = ON;\n\
         .read schema.sql\n",
    );
    for table in TABLES {
        let staging = format!("staging_{}", table.name);
        let names: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
        let typed: Vec<String> = names.iter().map(|n| format!("{n} TEXT")).collect();
        let selected: Vec<String> = table
            .columns
            .iter()
            .map(|column| {
                let mut expr = column.name.to_string();
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
            table.file(),
            table.name,
            names.join(", "),
            selected.join(", "),
        ));
    }
    out
}

#[cfg(test)]
#[path = "tests/sql_tests.rs"]
mod tests;
