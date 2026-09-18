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
    self, CensusClass, ConflictRow, DispatchShape, FirmwareRole, FirmwareRow, GateRow, GateState,
    HandlingCounts, KernelRow, NameRow, NameSource, OwnerClass, PupRow, Route, StubRow,
    SubentryRow, CALLER, CALLER_GATE, CALLER_UNRESOLVED, CAPABILITY_GATE, CENSUS, CENSUS_GATE,
    FIRMWARE, FIRMWARE_GATE, GATE, KERNEL, NAME, NAME_GATE, NAME_REGENERATE, PUP, PUP_GATE, REACH,
    REGENERATE, SCHEMA_VERSION, STUB, SUBENTRY, SUBENTRY_ATTRIBUTION, TABLES,
};
use cellgov_lv2::request::fidelity::ArmFidelity;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;
use sha2::Digest as _;

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

fn committed_kernels() -> Vec<KernelRow> {
    let text = read(&archive_dir().join(KERNEL.file()));
    let table = archive::parse(&KERNEL, &text).unwrap_or_else(|error| panic!("{error}"));
    let rerendered =
        archive::render(&KERNEL, &table.rows).unwrap_or_else(|error| panic!("kernel.tsv: {error}"));
    assert_eq!(rerendered, text, "kernel.tsv is not byte-canonical");
    archive::kernel_rows(&table)
}

fn committed_stubs() -> Vec<StubRow> {
    let text = read(&archive_dir().join(STUB.file()));
    let table = archive::parse(&STUB, &text).unwrap_or_else(|error| panic!("{error}"));
    let rerendered =
        archive::render(&STUB, &table.rows).unwrap_or_else(|error| panic!("stub.tsv: {error}"));
    assert_eq!(rerendered, text, "stub.tsv is not byte-canonical");
    archive::stub_rows(&table)
}

fn committed_subentries() -> Vec<SubentryRow> {
    let text = read(&archive_dir().join(SUBENTRY.file()));
    let table = archive::parse(&SUBENTRY, &text).unwrap_or_else(|error| panic!("{error}"));
    let rerendered = archive::render(&SUBENTRY, &table.rows)
        .unwrap_or_else(|error| panic!("subentry.tsv: {error}"));
    assert_eq!(rerendered, text, "subentry.tsv is not byte-canonical");
    archive::subentry_rows(&table)
}

fn committed_gates() -> Vec<GateRow> {
    let text = read(&archive_dir().join(CAPABILITY_GATE.file()));
    let table = archive::parse(&CAPABILITY_GATE, &text).unwrap_or_else(|error| panic!("{error}"));
    let rerendered = archive::render(&CAPABILITY_GATE, &table.rows)
        .unwrap_or_else(|error| panic!("gate.tsv: {error}"));
    assert_eq!(rerendered, text, "gate.tsv is not byte-canonical");
    archive::gate_rows(&table)
}

fn census_files(kernels: &[KernelRow], pups: &[PupRow]) -> Vec<String> {
    let firmware_by_pup: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    for fw in ["3.41", "3.56"] {
        assert_eq!(
            kernels
                .iter()
                .filter(|kernel| firmware_by_pup[kernel.pup_sha256.as_str()] == fw)
                .count(),
            2,
            "firmware {fw} must retain both PUP releases"
        );
    }
    assert_eq!(
        kernels
            .iter()
            .filter(|kernel| firmware_by_pup[kernel.pup_sha256.as_str()] == "3.41")
            .map(|kernel| kernel.kernel_elf_sha256.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "the two 3.41 PUPs share one kernel"
    );
    assert_eq!(
        kernels
            .iter()
            .filter(|kernel| firmware_by_pup[kernel.pup_sha256.as_str()] == "3.56")
            .map(|kernel| kernel.kernel_elf_sha256.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        2,
        "the two 3.56 PUPs carry distinct kernels"
    );
    kernels
        .iter()
        .map(|kernel| {
            let fw = firmware_by_pup
                .get(kernel.pup_sha256.as_str())
                .unwrap_or_else(|| panic!("kernel row names no PUP: {}", kernel.pup_sha256));
            archive::census_file(fw)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn generated_presence(census_files: &[String]) -> Vec<archive::PresenceRow> {
    let by_version = census_files
        .iter()
        .map(|file| {
            let fw = file
                .strip_prefix("census/fw-")
                .and_then(|value| value.strip_suffix(".tsv"))
                .expect("census file follows the archive path convention");
            let table = archive::parse(&CENSUS, &read(&archive_dir().join(file)))
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            (fw.to_string(), archive::census_rows(&table))
        })
        .collect();
    archive::presence_rows(&by_version)
        .unwrap_or_else(|error| panic!("reduce census presence: {error}"))
}

fn generated_transitions(
    firmware: &[FirmwareRow],
    pups: &[PupRow],
    kernels: &[KernelRow],
    gates: &[GateRow],
    census_files: &[String],
) -> Vec<archive::TransitionRow> {
    let census_by_version = census_files
        .iter()
        .map(|file| {
            let fw = file
                .strip_prefix("census/fw-")
                .and_then(|value| value.strip_suffix(".tsv"))
                .expect("census file follows the archive path convention");
            let table = archive::parse(&CENSUS, &read(&archive_dir().join(file)))
                .unwrap_or_else(|error| panic!("{file}: {error}"));
            (fw.to_string(), archive::census_rows(&table))
        })
        .collect();
    let firmware_by_pup: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let mut gates_by_version = BTreeMap::new();
    for fw in firmware {
        let pup_hashes: Vec<&str> = kernels
            .iter()
            .filter(|kernel| firmware_by_pup[kernel.pup_sha256.as_str()] == fw.fw)
            .map(|kernel| kernel.pup_sha256.as_str())
            .collect();
        let variants: Vec<Vec<&GateRow>> = pup_hashes
            .iter()
            .map(|pup| {
                gates
                    .iter()
                    .filter(|gate| gate.pup_sha256 == *pup)
                    .collect()
            })
            .collect();
        let Some(first) = variants.first() else {
            continue;
        };
        if variants.iter().all(|variant| {
            variant
                .iter()
                .map(|gate| gate.state)
                .eq(first.iter().map(|gate| gate.state))
        }) {
            gates_by_version.insert(
                fw.fw.clone(),
                first.iter().map(|gate| (*gate).clone()).collect(),
            );
        }
    }
    archive::transitions(firmware, &census_by_version, &gates_by_version)
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

struct ReadmeData<'a> {
    counts: &'a HandlingCounts,
    firmware: &'a [FirmwareRow],
    pups: &'a [PupRow],
    names: &'a [NameRow],
    conflicts: &'a [ConflictRow],
    kernels: &'a [KernelRow],
    stubs: &'a [StubRow],
    subentries: &'a [SubentryRow],
    gates: &'a [GateRow],
    presence_rows: usize,
    transition_rows: usize,
    census_files: &'a [String],
}

fn readme(data: ReadmeData<'_>) -> String {
    let manifest_rows: Vec<String> = archive::manifest(data.census_files)
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
                data.counts.of_route(*r),
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
            let rows = data.names.iter().filter(|n| n.source == *s).count();
            format!("| `{}` | {rows} | {} |", s.label(), s.meaning())
        })
        .collect();
    let named_slots: BTreeSet<(u64, Option<&str>)> = data
        .names
        .iter()
        .map(|n| (n.ordinal, n.packet.as_deref()))
        .collect();
    let conflict_rows = conflict_markdown(data.conflicts);
    let uncorroborated_rows: Vec<String> = archive::uncorroborated(data.names)
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
            ("schema_version", SCHEMA_VERSION.to_string()),
            ("manifest_rows", manifest_rows.join("\n")),
            ("owner_rows", owner_rows.join("\n")),
            ("slots", SYSCALL_TABLE_SLOTS.to_string()),
            ("route_rows", route_rows.join("\n")),
            ("fidelity_rows", fidelity_rows.join("\n")),
            ("sqlite_version", archive::SQLITE_VERSION.to_string()),
            ("behavior_gate", archive::BEHAVIOR_GATE.to_string()),
            ("firmware_rows", data.firmware.len().to_string()),
            (
                "firmware_dated",
                data.firmware
                    .iter()
                    .filter(|f| f.release_date.is_some())
                    .count()
                    .to_string(),
            ),
            ("firmware_gate", FIRMWARE_GATE.to_string()),
            ("firmware_role_rows", firmware_role_rows.join("\n")),
            ("pup_rows", data.pups.len().to_string()),
            (
                "pup_versions",
                data.pups
                    .iter()
                    .map(|row| row.fw.as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    .to_string(),
            ),
            ("pup_gate", PUP_GATE.to_string()),
            ("kernel_rows", data.kernels.len().to_string()),
            ("stub_rows", data.stubs.len().to_string()),
            ("subentry_rows", data.subentries.len().to_string()),
            ("gate_rows", data.gates.len().to_string()),
            ("presence_rows", data.presence_rows.to_string()),
            ("transition_rows", data.transition_rows.to_string()),
            ("census_files", data.census_files.len().to_string()),
            ("census_gate", CENSUS_GATE.to_string()),
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
    let kernels = committed_kernels();
    let stubs = committed_stubs();
    let subentries = committed_subentries();
    let gates = committed_gates();
    let census_files = census_files(&kernels, &pups);
    let presence = generated_presence(&census_files);
    let transition_rows = generated_transitions(&firmware, &pups, &kernels, &gates, &census_files);
    let names = committed_names();
    let conflicts = archive::conflict_rows(&names);
    let mut files = BTreeMap::new();
    files.insert(
        "README.md".to_string(),
        readme(ReadmeData {
            counts: &counts,
            firmware: &firmware,
            pups: &pups,
            names: &names,
            conflicts: &conflicts,
            kernels: &kernels,
            stubs: &stubs,
            subentries: &subentries,
            gates: &gates,
            presence_rows: presence.len(),
            transition_rows: transition_rows.len(),
            census_files: &census_files,
        }),
    );
    files.insert("schema.sql".to_string(), archive::schema_sql());
    files.insert("build.sql".to_string(), archive::build_sql(&census_files));
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
    files.insert(
        archive::PRESENCE.file(),
        archive::presence_tsv(&presence).unwrap_or_else(|error| panic!("presence.tsv: {error}")),
    );
    files.insert(
        archive::TRANSITIONS.file(),
        archive::transitions_tsv(&transition_rows)
            .unwrap_or_else(|error| panic!("transitions.tsv: {error}")),
    );
    let names: Vec<&String> = files.keys().collect();
    let generated: Vec<String> = archive::manifest(&census_files)
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
    let kernels = committed_kernels();
    let pups = committed_pups();
    let census_files = census_files(&kernels, &pups);
    let mut present = archive_paths(&dir, &dir);
    present.sort();
    assert_eq!(
        present,
        archive::files(&census_files),
        "docs/lv2/ and the manifest in cellgov_lv2::archive disagree"
    );
}

fn archive_paths(root: &Path, directory: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
    {
        let entry = entry.unwrap_or_else(|error| panic!("read entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            paths.extend(archive_paths(root, &path));
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("archive entry is below its root")
            .to_string_lossy()
            .replace('\\', "/");
        if !is_built_database(&relative) {
            paths.push(relative);
        }
    }
    paths
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
    let kernels = committed_kernels();
    let pups = committed_pups();
    for file in census_files(&kernels, &pups) {
        let census = archive::parse(&CENSUS, &read(&dir.join(file)))
            .unwrap_or_else(|error| panic!("{error}"));
        let mut with_census = tables.clone();
        with_census.push(census);
        archive::check_references(&with_census).unwrap_or_else(|error| panic!("{error}"));
    }
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
fn kernel_census_rows_are_well_formed() {
    assert_eq!(KERNEL.gate, CENSUS_GATE);
    assert_eq!(STUB.gate, CENSUS_GATE);
    assert_eq!(CAPABILITY_GATE.gate, CENSUS_GATE);
    let pups = committed_pups();
    let kernels = committed_kernels();
    let stubs = committed_stubs();
    let subentries = committed_subentries();
    let gates = committed_gates();
    assert_eq!(
        subentries.len(),
        23_883,
        "committed packet-row golden count"
    );
    assert_eq!(kernels.len(), 97, "one kernel row per extracted retail PUP");
    assert_eq!(
        census_files(&kernels, &pups).len(),
        95,
        "one census file per displayed firmware version"
    );

    let firmware_by_pup: BTreeMap<&str, &str> = pups
        .iter()
        .map(|row| (row.pup_sha256.as_str(), row.fw.as_str()))
        .collect();
    let mut stubs_by_pup: BTreeMap<&str, Vec<&StubRow>> = BTreeMap::new();
    for stub in &stubs {
        stubs_by_pup
            .entry(stub.pup_sha256.as_str())
            .or_default()
            .push(stub);
    }
    let mut subentries_by_pup: BTreeMap<&str, Vec<&SubentryRow>> = BTreeMap::new();
    for row in &subentries {
        subentries_by_pup
            .entry(row.pup_sha256.as_str())
            .or_default()
            .push(row);
    }
    let mut gates_by_pup: BTreeMap<&str, Vec<&GateRow>> = BTreeMap::new();
    for row in &gates {
        gates_by_pup
            .entry(row.pup_sha256.as_str())
            .or_default()
            .push(row);
    }

    let mut pups_by_kernel: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for kernel in &kernels {
        pups_by_kernel
            .entry(kernel.kernel_elf_sha256.as_str())
            .or_default()
            .push(kernel.pup_sha256.as_str());
    }
    let duplicate_kernel_groups: Vec<_> = pups_by_kernel
        .values()
        .filter(|pups| pups.len() > 1)
        .collect();
    assert!(
        !duplicate_kernel_groups.is_empty(),
        "at least one retail kernel must be shared by release variants"
    );
    for duplicate_pups in duplicate_kernel_groups {
        let expected: Vec<_> = subentries_by_pup
            .get(duplicate_pups[0])
            .unwrap_or_else(|| panic!("{} has no subentries", duplicate_pups[0]))
            .iter()
            .map(|row| {
                (
                    row.ordinal,
                    row.selector_slot.as_str(),
                    row.packet,
                    row.class,
                    row.target,
                )
            })
            .collect();
        for pup in &duplicate_pups[1..] {
            let actual: Vec<_> = subentries_by_pup
                .get(*pup)
                .unwrap_or_else(|| panic!("{pup} has no subentries"))
                .iter()
                .map(|row| {
                    (
                        row.ordinal,
                        row.selector_slot.as_str(),
                        row.packet,
                        row.class,
                        row.target,
                    )
                })
                .collect();
            assert_eq!(
                actual, expected,
                "byte-identical kernels must have byte-identical subentries: {} and {pup}",
                duplicate_pups[0]
            );
        }
    }

    for kernel in &kernels {
        assert_eq!(kernel.entry_width, 8, "dispatch entries are u64 pointers");
        assert_eq!(
            kernel.entry_count, SYSCALL_TABLE_SLOTS as usize,
            "kernel row must cover the architectural archive slot count"
        );
        let fw = firmware_by_pup
            .get(kernel.pup_sha256.as_str())
            .unwrap_or_else(|| panic!("kernel row names no PUP: {}", kernel.pup_sha256));
        let path = archive_dir().join(archive::census_file(fw));
        let text = read(&path);
        assert_eq!(
            sha256_hex(text.as_bytes()),
            kernel.census_sha256,
            "{} names the wrong census digest",
            kernel.pup_sha256
        );
        let table = archive::parse(&CENSUS, &text).unwrap_or_else(|error| panic!("{error}"));
        let rows = archive::census_rows(&table);
        let pup_subentries = subentries_by_pup
            .get(kernel.pup_sha256.as_str())
            .cloned()
            .unwrap_or_default();
        let pup_subentry_rows: Vec<_> = pup_subentries.iter().map(|row| (*row).clone()).collect();
        let pup_subentry_text =
            archive::subentry_tsv(&pup_subentry_rows).expect("render PUP subentries");
        assert_eq!(
            sha256_hex(pup_subentry_text.as_bytes()),
            kernel.subentry_sha256,
            "{fw}: subentry digest"
        );
        let pup_gates = gates_by_pup
            .get(kernel.pup_sha256.as_str())
            .unwrap_or_else(|| panic!("{} has no gate rows", kernel.pup_sha256));
        let pup_gate_rows: Vec<_> = pup_gates.iter().map(|row| (*row).clone()).collect();
        let pup_gate_text = archive::gate_tsv(&pup_gate_rows).expect("render PUP gate rows");
        assert_eq!(
            sha256_hex(pup_gate_text.as_bytes()),
            kernel.gate_sha256,
            "{fw}: gate digest"
        );
        assert_eq!(
            pup_gates.len(),
            kernel.entry_count,
            "{fw}: gate entry count"
        );
        for (ordinal, gate) in pup_gates.iter().enumerate() {
            assert_eq!(gate.ordinal, ordinal, "{fw}: gate ordinal sequence");
            match gate.state {
                GateState::Gated => {
                    assert!(
                        gate.reads
                            .as_deref()
                            .is_some_and(|reads| reads.starts_with("ctrl_flags1_0x")),
                        "{fw}: gated ordinal {ordinal} lacks a capability read"
                    );
                    let errno = gate.fail_errno.expect("gated ordinal has a failure errno");
                    assert!(
                        cellgov_ps3_abi::lv2::errno::lookup(errno).is_some(),
                        "{fw}: unknown gate errno 0x{errno:08x} at {ordinal}"
                    );
                }
                GateState::Ungated | GateState::NotAnalysed => {
                    assert!(
                        gate.reads.is_none(),
                        "{fw}: non-gated ordinal {ordinal} has a read"
                    );
                    assert!(
                        gate.fail_errno.is_none(),
                        "{fw}: non-gated ordinal {ordinal} has a failure errno"
                    );
                }
            }
        }
        let subtable_ordinals: BTreeSet<usize> =
            pup_subentries.iter().map(|row| row.ordinal).collect();
        for row in &pup_subentries {
            assert_ne!(
                row.class,
                CensusClass::Absent,
                "subentry {}:{} cannot be absent",
                row.ordinal,
                row.packet
            );
            assert!(
                matches!(
                    row.selector_slot.as_str(),
                    "r3" | "r4" | "r5" | "r6" | "r7" | "r8" | "r9" | "r10"
                ),
                "invalid selector slot {}",
                row.selector_slot
            );
        }
        assert_eq!(rows.len(), kernel.entry_count, "{fw}: entry count");
        let mut stub_references: BTreeMap<u64, usize> = BTreeMap::new();
        for (ordinal, row) in rows.iter().enumerate() {
            assert_eq!(row.fw, *fw, "{} row {ordinal}: firmware", path.display());
            assert_eq!(row.ordinal, ordinal, "{fw}: ordinal sequence");
            assert_eq!(
                row.class == CensusClass::Absent,
                row.target.is_none(),
                "{fw}: absent/target mismatch at {ordinal}"
            );
            if row.class == CensusClass::Stub {
                *stub_references
                    .entry(row.target.expect("stub rows have a target"))
                    .or_default() += 1;
            }
            assert_eq!(
                row.dispatch == DispatchShape::Subtable,
                subtable_ordinals.contains(&ordinal),
                "{fw}: subtable/subentry mismatch at {ordinal}"
            );
        }

        let pup_stubs = stubs_by_pup
            .get(kernel.pup_sha256.as_str())
            .unwrap_or_else(|| panic!("{} has no stub rows", kernel.pup_sha256));
        assert_eq!(
            pup_stubs.iter().filter(|stub| stub.primary).count(),
            1,
            "{} must have one primary stub",
            kernel.pup_sha256
        );
        let mut recorded: BTreeMap<u64, usize> = BTreeMap::new();
        for stub in pup_stubs {
            assert!(stub.references > 0, "stub targets must have a reference");
            let expected = cellgov_ps3_abi::lv2::errno::lookup(stub.errno)
                .unwrap_or_else(|| panic!("unknown Cell errno 0x{:08x}", stub.errno));
            assert_eq!(stub.errno_symbol, expected.symbol, "stub errno symbol");
            *recorded.entry(stub.target).or_default() += stub.references;
        }
        assert_eq!(recorded, stub_references, "{fw}: stub reference counts");
    }
    assert_eq!(
        stubs_by_pup.len(),
        kernels.len(),
        "stub.tsv and kernel.tsv cover different PUP sets"
    );
    assert_eq!(
        subentries_by_pup.len(),
        kernels.len(),
        "subentry.tsv and kernel.tsv cover different PUP sets"
    );
    assert_eq!(
        gates_by_pup.len(),
        kernels.len(),
        "gate.tsv and kernel.tsv cover different PUP sets"
    );
    assert!(
        gates.iter().any(|row| {
            row.ordinal == 871
                && row.state == GateState::Gated
                && row.reads.as_deref() == Some("ctrl_flags1_0x20000000")
                && row.fail_errno == Some(cellgov_ps3_abi::lv2::errno::CELL_ENOSYS.code)
        }),
        "ordinal 871 must retain an observed capability gate"
    );

    let attributed = archive::parse(
        &SUBENTRY_ATTRIBUTION,
        &read(&archive_dir().join(SUBENTRY_ATTRIBUTION.file())),
    )
    .expect("parse attributed subentries");
    let attributed_861: BTreeSet<u64> = attributed
        .rows
        .iter()
        .filter(|row| row[0] == "861")
        .map(|row| row[2].parse().expect("packet is an integer"))
        .collect();
    assert_eq!(attributed_861, (0..=19).collect());
    let extracted_861: BTreeSet<u64> = subentries
        .iter()
        .filter(|row| row.ordinal == 861)
        .map(|row| row.packet)
        .collect();
    assert!(
        attributed_861.is_superset(&extracted_861),
        "the attributed packet set must cover the extracted packet set"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
