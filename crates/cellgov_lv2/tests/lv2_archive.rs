//! `docs/lv2/` drift gate.
//!
//! The gate renders every generated file of the archive from
//! `cellgov_lv2::archive` and compares it to the committed copy.
//! Regenerate with:
//!
//! ```text
//! cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate
//! ```
//!
//! The `cellgov` rows of `name.tsv` have their own gate (the test
//! [`NAME_GATE`] names) and their own regenerate:
//!
//! ```text
//! cargo test -p cellgov_lv2 --test lv2_archive -- --ignored regenerate_cellgov_names
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_lv2::archive::{
    self, ConflictRow, FirmwareRole, FirmwareRow, HandlingCounts, NameRow, NameSource, OwnerClass,
    PupRow, Route, CALLER, CALLER_GATE, CALLER_UNRESOLVED, FIRMWARE, FIRMWARE_GATE, GATE, NAME,
    NAME_GATE, NAME_REGENERATE, PUP, PUP_GATE, REACH, REGENERATE, TABLES,
};
use cellgov_lv2::request::fidelity::ArmFidelity;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

const README_TEMPLATE: &str = include_str!("templates/README.md.template");

fn archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2")
}

/// The committed `name.tsv`, loaded; the generator renders
/// `conflicts.tsv` and the document's name sections from it.
fn committed_names() -> Vec<NameRow> {
    let text = read(&archive_dir().join(NAME.file()));
    let table = archive::parse(&NAME, &text).unwrap_or_else(|e| panic!("{e}"));
    archive::name_rows(&table)
}

fn committed_firmware() -> Vec<FirmwareRow> {
    let text = read(&archive_dir().join(FIRMWARE.file()));
    let table = archive::parse(&FIRMWARE, &text).unwrap_or_else(|e| panic!("{e}"));
    let rerendered =
        archive::render(&FIRMWARE, &table.rows).unwrap_or_else(|e| panic!("firmware.tsv: {e}"));
    assert_eq!(
        rerendered, text,
        "the firmware loader does not re-render firmware.tsv byte-identically"
    );
    archive::firmware_rows(&table)
}

fn committed_pups() -> Vec<PupRow> {
    let text = read(&archive_dir().join(PUP.file()));
    let table = archive::parse(&PUP, &text).unwrap_or_else(|error| panic!("{error}"));
    let rerendered =
        archive::render(&PUP, &table.rows).unwrap_or_else(|error| panic!("pup.tsv: {error}"));
    assert_eq!(
        rerendered, text,
        "the archive loader does not re-render pup.tsv byte-identically"
    );
    archive::pup_rows(&table)
}

fn slot(ordinal: u64, packet: Option<&str>) -> String {
    match packet {
        Some(packet) => format!("{ordinal} `{packet}`"),
        None => ordinal.to_string(),
    }
}

/// The sources that give each distinct name of one conflicting slot.
type NamesBySource<'a> = BTreeMap<&'a str, Vec<&'a str>>;

/// One markdown row per conflicting slot: every distinct name with the
/// sources that give it.
fn conflict_markdown(conflicts: &[ConflictRow]) -> Vec<String> {
    // Keyed on the packet cell as the loader sorts it, so the rows
    // keep `conflicts.tsv` order once a packet slot exists.
    let mut slots: BTreeMap<(u64, &str), (&ConflictRow, NamesBySource)> = BTreeMap::new();
    for row in conflicts {
        slots
            .entry((row.ordinal, row.packet.as_deref().unwrap_or(archive::NONE)))
            .or_insert_with(|| (row, BTreeMap::new()))
            .1
            .entry(&row.name)
            .or_default()
            .push(row.source.label());
    }
    slots
        .into_iter()
        .map(|((ordinal, _), (first, names))| {
            let listed: Vec<String> = names
                .iter()
                .map(|(name, sources)| format!("`{name}` ({})", sources.join(", ")))
                .collect();
            format!(
                "| {} | {} | {} |",
                slot(ordinal, first.packet.as_deref()),
                first.disagreement.label(),
                listed.join(", ")
            )
        })
        .collect()
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

fn readme(
    counts: &HandlingCounts,
    firmware: &[FirmwareRow],
    pups: &[PupRow],
    names: &[NameRow],
    conflicts: &[ConflictRow],
) -> String {
    let manifest_rows: Vec<String> = archive::manifest()
        .iter()
        .map(|row| {
            let regenerate = match row.regenerate {
                Some(command) if command == NAME_REGENERATE => {
                    format!("`{command}` (the `cellgov` rows)")
                }
                Some(command) => format!("`{command}`"),
                None => "written by hand".to_string(),
            };
            format!(
                "| `{}` | {} | {} | `{}` |",
                row.file,
                row.owner.label(),
                regenerate,
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
    let firmware_role_rows: Vec<String> = FirmwareRole::ALL
        .iter()
        .map(|r| format!("| `{}` | {} |", r.label(), r.meaning()))
        .collect();
    let name_source_rows: Vec<String> = NameSource::ALL
        .iter()
        .map(|s| {
            let rows = names.iter().filter(|n| n.source == *s).count();
            format!("| `{}` | {rows} | {} |", s.label(), s.meaning())
        })
        .collect();
    let named_slots: BTreeSet<(u64, Option<&str>)> = names
        .iter()
        .map(|n| (n.ordinal, n.packet.as_deref()))
        .collect();
    let conflict_rows = conflict_markdown(conflicts);
    let uncorroborated_rows: Vec<String> = archive::uncorroborated(names)
        .iter()
        .map(|n| {
            let constant = n
                .reference
                .as_deref()
                .and_then(|r| r.strip_prefix(archive::CELLGOV_CONSTANT_PATH))
                .unwrap_or_else(|| panic!("{} is a cellgov row with no constant", n.ordinal));
            format!(
                "| {} | `{}` | `{constant}` |",
                slot(n.ordinal, n.packet.as_deref()),
                n.name
            )
        })
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
            ("behavior_gate", archive::BEHAVIOR_GATE.to_string()),
            ("firmware_rows", firmware.len().to_string()),
            (
                "firmware_dated",
                firmware
                    .iter()
                    .filter(|f| f.release_date.is_some())
                    .count()
                    .to_string(),
            ),
            ("firmware_gate", FIRMWARE_GATE.to_string()),
            ("firmware_role_rows", firmware_role_rows.join("\n")),
            ("pup_rows", pups.len().to_string()),
            (
                "pup_versions",
                pups.iter()
                    .map(|row| row.fw.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    .to_string(),
            ),
            ("pup_gate", PUP_GATE.to_string()),
            ("name_source_rows", name_source_rows.join("\n")),
            ("named_slots", named_slots.len().to_string()),
            ("name_gate", NAME_GATE.to_string()),
            ("name_regenerate", NAME_REGENERATE.to_string()),
            ("conflict_count", conflict_rows.len().to_string()),
            ("conflict_rows", conflict_rows.join("\n")),
            (
                "uncorroborated_count",
                uncorroborated_rows.len().to_string(),
            ),
            ("uncorroborated_rows", uncorroborated_rows.join("\n")),
        ],
    )
}

/// Every generated file of the archive, rendered, keyed by file name.
fn rendered() -> BTreeMap<String, String> {
    let routes = archive::route_rows();
    let arms = archive::arm_rows(&routes);
    let counts = HandlingCounts::of(&routes);
    let firmware = committed_firmware();
    let pups = committed_pups();
    let names = committed_names();
    let conflicts = archive::conflict_rows(&names);
    let mut files = BTreeMap::new();
    files.insert(
        "README.md".to_string(),
        readme(&counts, &firmware, &pups, &names, &conflicts),
    );
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
    files.insert(
        "conflicts.tsv".to_string(),
        archive::conflicts_tsv(&conflicts).unwrap_or_else(|e| panic!("conflicts.tsv: {e}")),
    );
    let names: Vec<&String> = files.keys().collect();
    let generated: Vec<String> = archive::manifest()
        .into_iter()
        .filter(|row| row.regenerate == Some(REGENERATE))
        .map(|row| row.file)
        .collect();
    assert_eq!(
        names,
        generated.iter().collect::<Vec<_>>(),
        "the generator and the manifest name different generated files"
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
fn firmware_rows_are_well_formed() {
    let rows = committed_firmware();
    assert!(
        rows.len() >= 90,
        "the retail line has more than {} versions",
        rows.len()
    );
    archive::check_firmware_rows(&rows).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn pup_rows_are_well_formed() {
    archive::check_pup_rows(&committed_pups()).unwrap_or_else(|error| panic!("{error}"));
    let dir = archive_dir();
    let tables: Vec<archive::Table> = [FIRMWARE, PUP]
        .iter()
        .map(|spec| {
            archive::parse(spec, &read(&dir.join(spec.file())))
                .unwrap_or_else(|error| panic!("{error}"))
        })
        .collect();
    archive::check_references(&tables).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn caller_rows_are_well_formed() {
    assert_eq!(CALLER.gate, CALLER_GATE);
    assert_eq!(CALLER_UNRESOLVED.gate, CALLER_GATE);
    assert_eq!(REACH.gate, CALLER_GATE);
    let dir = archive_dir();
    let caller = archive::parse(&CALLER, &read(&dir.join(CALLER.file())))
        .unwrap_or_else(|error| panic!("{error}"));
    let unresolved = archive::parse(
        &CALLER_UNRESOLVED,
        &read(&dir.join(CALLER_UNRESOLVED.file())),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let reach = archive::parse(&REACH, &read(&dir.join(REACH.file())))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(!caller.rows.is_empty(), "caller.tsv has no resolved site");
    assert!(
        !unresolved.rows.is_empty(),
        "caller_unresolved.tsv names no scanned module"
    );
    assert!(!reach.rows.is_empty(), "reach.tsv names no exported reach");

    let expected_pups: BTreeSet<String> = committed_pups()
        .into_iter()
        .map(|row| row.pup_sha256)
        .collect();
    let scanned_pups: BTreeSet<String> = unresolved.rows.iter().map(|row| row[0].clone()).collect();
    assert_eq!(
        scanned_pups, expected_pups,
        "caller_unresolved.tsv must cover every PUP image"
    );

    let scanned: BTreeSet<(&str, &str)> = unresolved
        .rows
        .iter()
        .map(|row| (row[0].as_str(), row[1].as_str()))
        .collect();
    for row in &caller.rows {
        assert!(
            scanned.contains(&(row[0].as_str(), row[1].as_str())),
            "caller row names a module absent from caller_unresolved: {row:?}"
        );
    }
    let resolved: BTreeSet<(&str, &str, &str)> = caller
        .rows
        .iter()
        .map(|row| (row[0].as_str(), row[1].as_str(), row[2].as_str()))
        .collect();
    for row in &reach.rows {
        assert!(
            resolved.contains(&(row[0].as_str(), row[1].as_str(), row[3].as_str())),
            "reach row names no resolved caller ordinal: {row:?}"
        );
    }
}

#[test]
fn cellgov_name_rows_match_the_macro() {
    let committed: Vec<NameRow> = committed_names()
        .into_iter()
        .filter(|row| row.source == NameSource::Cellgov)
        .collect();
    let rendered = archive::macro_name_rows();
    let missing: Vec<&NameRow> = rendered.iter().filter(|r| !committed.contains(r)).collect();
    let extra: Vec<&NameRow> = committed.iter().filter(|r| !rendered.contains(r)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "name.tsv's cellgov rows are not the macro's: missing {missing:?}, extra {extra:?}; \
         regenerate with:\n  {NAME_REGENERATE}"
    );
}

#[test]
fn every_name_row_fits_its_source() {
    let unfit: Vec<NameRow> = committed_names()
        .into_iter()
        .filter(|row| !row.fits_source())
        .collect();
    assert!(
        unfit.is_empty(),
        "name.tsv rows whose reference or name is not what the source's meaning promises: {unfit:?}"
    );
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

#[test]
#[ignore = "writes docs/lv2/name.tsv; run when the macro's names change"]
fn regenerate_cellgov_names() {
    let rows = archive::with_cellgov_rows(&committed_names());
    let text = archive::name_tsv(&rows).unwrap_or_else(|e| panic!("name.tsv: {e}"));
    let path = archive_dir().join(NAME.file());
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
