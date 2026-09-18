//! `docs/lv2/` drift gate.
//!
//! The gate renders every file of the archive from
//! `cellgov_lv2::archive` and compares it to the committed copy.
//! Regenerate with:
//!
//! ```text
//! cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cellgov_lv2::archive::{self, HandlingCounts, OwnerClass, Route, GATE, REGENERATE, TABLES};
use cellgov_lv2::request::fidelity::ArmFidelity;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

const README_TEMPLATE: &str = include_str!("templates/README.md.template");

fn archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2")
}

fn fill(template: &str, subs: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in subs {
        let placeholder = format!("{{{{{key}}}}}");
        assert!(
            out.contains(&placeholder),
            "the template has no {placeholder}"
        );
        out = out.replace(&placeholder, value);
    }
    assert!(
        !out.contains("{{"),
        "the template has a placeholder nothing fills: {out}"
    );
    out
}

fn readme(counts: &HandlingCounts) -> String {
    let manifest_rows: Vec<String> = archive::manifest()
        .iter()
        .map(|row| {
            format!(
                "| `{}` | {} | `{}` | `{}` |",
                row.file,
                row.owner.label(),
                row.regenerate,
                row.gate
            )
        })
        .collect();
    let owner_rows: Vec<String> = OwnerClass::ALL
        .iter()
        .map(|c| format!("| {} | {} |", c.label(), c.meaning()))
        .collect();
    let route_rows: Vec<String> = Route::ALL
        .iter()
        .map(|r| {
            format!(
                "| `{}` | {} | {} |",
                r.label(),
                counts.of_route(*r),
                r.meaning()
            )
        })
        .collect();
    let fidelity_rows: Vec<String> = ArmFidelity::ALL
        .iter()
        .map(|f| format!("| `{}` | {} |", f.label(), f.meaning()))
        .collect();
    fill(
        README_TEMPLATE,
        &[
            ("regenerate", REGENERATE.to_string()),
            ("gate", GATE.to_string()),
            ("manifest_rows", manifest_rows.join("\n")),
            ("owner_rows", owner_rows.join("\n")),
            ("slots", SYSCALL_TABLE_SLOTS.to_string()),
            ("route_rows", route_rows.join("\n")),
            ("fidelity_rows", fidelity_rows.join("\n")),
            ("sqlite_version", archive::SQLITE_VERSION.to_string()),
        ],
    )
}

/// Every file of the archive, rendered, keyed by file name.
fn rendered() -> BTreeMap<String, String> {
    let routes = archive::route_rows();
    let arms = archive::arm_rows(&routes);
    let counts = HandlingCounts::of(&routes);
    let mut files = BTreeMap::new();
    files.insert("README.md".to_string(), readme(&counts));
    files.insert("schema.sql".to_string(), archive::schema_sql());
    files.insert("build.sql".to_string(), archive::build_sql());
    files.insert(
        "route.tsv".to_string(),
        archive::route_tsv(&routes).unwrap_or_else(|e| panic!("route.tsv: {e}")),
    );
    files.insert(
        "arm.tsv".to_string(),
        archive::arm_tsv(&arms).unwrap_or_else(|e| panic!("arm.tsv: {e}")),
    );
    let names: Vec<&String> = files.keys().collect();
    let manifest = archive::files();
    assert_eq!(
        names,
        manifest.iter().collect::<Vec<_>>(),
        "the generator and the manifest name different files"
    );
    files
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}; regenerate with:\n  {REGENERATE}",
            path.display()
        )
    })
}

/// Collapse markdown table-cell padding so a formatter's column
/// alignment does not read as drift; content changes still do.
fn normalize_markdown(text: &str) -> String {
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").lines() {
        let line = line.trim_end();
        if line.starts_with('|') && line.ends_with('|') {
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| {
                    let c = c.trim();
                    if c.len() >= 3 && c.chars().all(|ch| ch == '-' || ch == ':') {
                        format!(
                            "{}---{}",
                            if c.starts_with(':') { ":" } else { "" },
                            if c.ends_with(':') { ":" } else { "" }
                        )
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn committed_archive_matches_generator() {
    let dir = archive_dir();
    let mut stale = Vec::new();
    for (name, text) in rendered() {
        let committed = read(&dir.join(&name));
        let same = if name.ends_with(".md") {
            normalize_markdown(&committed) == normalize_markdown(&text)
        } else {
            committed == text
        };
        if !same {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "docs/lv2/ is stale in {stale:?}; regenerate with:\n  {REGENERATE}"
    );
}

/// A database `build.sql` wrote in place, with the files SQLite keeps
/// beside it. It is gitignored, so it is never part of the archive.
fn is_built_database(name: &str) -> bool {
    [".db", ".db-journal", ".db-wal", ".db-shm"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

#[test]
fn the_archive_directory_holds_exactly_the_manifest() {
    let dir = archive_dir();
    let mut present: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("read entry under {}: {e}", dir.display()))
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .filter(|name| !is_built_database(name))
        .collect();
    present.sort();
    assert_eq!(
        present,
        archive::files(),
        "docs/lv2/ and the manifest in cellgov_lv2::archive disagree"
    );
}

#[test]
fn committed_tables_load_and_reference_each_other() {
    let dir = archive_dir();
    let tables: Vec<archive::Table> = TABLES
        .iter()
        .map(|spec| {
            archive::parse(spec, &read(&dir.join(spec.file()))).unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    archive::check_references(&tables).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
#[ignore = "writes docs/lv2/; run on dispatch-surface changes"]
fn regenerate() {
    let dir = archive_dir();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
    for (name, text) in rendered() {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
}
