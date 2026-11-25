//! `docs/lv1/` drift gate.
//!
//! The gate renders every generated file of the archive from
//! `cellgov_lv1_archive` and compares it to the committed copy. Under
//! `CELLGOV_REGENERATE=1` it writes them instead:
//!
//! ```text
//! CELLGOV_REGENERATE=1 cargo test -p cellgov_lv1_archive --test lv1_archive committed_archive_matches_generator
//! ```
//!
//! The firmware, PUP and kernel tables it reads are the LV2 archive's,
//! under `docs/lv2/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[path = "support/regenerate.rs"]
mod regenerate;

use cellgov_lv1_archive::{
    self as archive, GATE, HYPERCALL, HYPERCALL_GATE, HYPERCALL_REGENERATE, HYPERCALL_SITE,
    HYPERCALL_UNRESOLVED, REGENERATE, REPOSITORY, REPOSITORY_GATE, REPOSITORY_LOGGED_FIRMWARE,
    REPOSITORY_READ, REPOSITORY_REGENERATE, REPOSITORY_UNOBSERVED, SCHEMA_VERSION, SHARED_TABLES,
    TABLES,
};
use cellgov_lv2_archive::{self as lv2_archive, KernelRow, OwnerClass, PupRow, KERNEL, PUP};
use cellgov_ps3_abi::lv1::repository::NODES;

const README_TEMPLATE: &str = include_str!("templates/README.md.template");

fn archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv1")
}

/// The LV2 archive, which holds the firmware, PUP and kernel tables.
fn lv2_archive_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/lv2")
}

fn committed_pups() -> Vec<PupRow> {
    let text = read(&lv2_archive_dir().join(PUP.file()));
    let table = lv2_archive::parse(&PUP, &text).unwrap_or_else(|error| panic!("{error}"));
    lv2_archive::pup_rows(&table)
}

fn committed_kernels() -> Vec<KernelRow> {
    let text = read(&lv2_archive_dir().join(KERNEL.file()));
    let table = lv2_archive::parse(&KERNEL, &text).unwrap_or_else(|error| panic!("{error}"));
    lv2_archive::kernel_rows(&table)
}

fn committed_hypercall_sites() -> Vec<archive::HypercallSiteRow> {
    let text = read(&archive_dir().join(HYPERCALL_SITE.file()));
    let table = lv2_archive::parse(&HYPERCALL_SITE, &text).unwrap_or_else(|e| panic!("{e}"));
    archive::hypercall_site_rows(&table)
}

fn committed_repository_reads() -> Vec<archive::RepositoryReadRow> {
    let text = read(&archive_dir().join(REPOSITORY_READ.file()));
    let table = lv2_archive::parse(&REPOSITORY_READ, &text).unwrap_or_else(|e| panic!("{e}"));
    let rerendered = lv2_archive::render(&REPOSITORY_READ, &table.rows)
        .unwrap_or_else(|e| panic!("repository_read.tsv: {e}"));
    assert_eq!(
        rerendered, text,
        "repository_read.tsv is not byte-canonical"
    );
    archive::repository_read_rows(&table)
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
    hypercalls: &[archive::HypercallRow],
    repository: &[archive::RepositoryRow],
    repository_reads: &[archive::RepositoryReadRow],
) -> String {
    let manifest_rows: Vec<String> = archive::manifest()
        .iter()
        .map(|row| {
            let regenerate = match row.regenerate {
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
    fill(
        README_TEMPLATE,
        &[
            ("regenerate", REGENERATE.to_string()),
            ("gate", GATE.to_string()),
            ("schema_version", SCHEMA_VERSION.to_string()),
            ("shared_table_dir", archive::SHARED_TABLE_DIR.to_string()),
            ("manifest_rows", manifest_rows.join("\n")),
            ("owner_rows", owner_rows.join("\n")),
            ("sqlite_version", lv2_archive::SQLITE_VERSION.to_string()),
            ("hypercall_numbers", hypercalls.len().to_string()),
            (
                "hypercall_answered",
                hypercalls
                    .iter()
                    .filter(|row| row.state == archive::HypercallState::Answered)
                    .count()
                    .to_string(),
            ),
            (
                "hypercall_invalid",
                hypercalls
                    .iter()
                    .filter(|row| row.lv1_493 == archive::Lv1Entry::Invalid)
                    .count()
                    .to_string(),
            ),
            ("hypercall_gate", HYPERCALL_GATE.to_string()),
            ("hypercall_regenerate", HYPERCALL_REGENERATE.to_string()),
            ("repository_nodes", repository.len().to_string()),
            (
                "repository_logged_firmware",
                REPOSITORY_LOGGED_FIRMWARE.join(", "),
            ),
            (
                "repository_refused",
                repository_reads
                    .iter()
                    .filter(|row| !row.answered)
                    .map(|row| format!("`{}`", row.node))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            ("repository_gate", REPOSITORY_GATE.to_string()),
            ("repository_regenerate", REPOSITORY_REGENERATE.to_string()),
            ("repository_unobserved", REPOSITORY_UNOBSERVED.to_string()),
        ],
    )
}

/// Every generated file of the archive, rendered, keyed by file name.
fn rendered() -> BTreeMap<String, String> {
    let pups = committed_pups();
    let hypercalls = archive::hypercall_rows(&committed_hypercall_sites(), &pups)
        .expect("every committed hypercall site row names a committed PUP");
    let repository_reads = committed_repository_reads();
    let repository = archive::repository_rows(NODES, &repository_reads, &pups)
        .unwrap_or_else(|e| panic!("{e}; regenerate with:\n  {REPOSITORY_REGENERATE}"));
    let mut files = BTreeMap::new();
    files.insert(
        "README.md".to_string(),
        readme(&hypercalls, &repository, &repository_reads),
    );
    files.insert(
        HYPERCALL.file(),
        archive::hypercall_tsv(&hypercalls).unwrap_or_else(|e| panic!("hypercall.tsv: {e}")),
    );
    files.insert(
        REPOSITORY.file(),
        archive::repository_tsv(&repository).unwrap_or_else(|e| panic!("repository.tsv: {e}")),
    );
    files.insert("sql/schema.sql".to_string(), archive::schema_sql());
    files.insert("sql/build.sql".to_string(), archive::build_sql());
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
    if regenerate::asked() {
        for (name, text) in rendered() {
            let path = dir.join(name);
            let parent = path
                .parent()
                .expect("archive output has a parent directory");
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
            regenerate::write_if_asked(&path, text);
        }
        return;
    }
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
        "docs/lv1/ is stale in {stale:?}; regenerate with:\n  {REGENERATE}"
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
    let mut present = archive_paths(&dir, &dir);
    present.sort();
    assert_eq!(
        present,
        archive::files(),
        "docs/lv1/ and the manifest in cellgov_lv1_archive disagree"
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
    let shared = SHARED_TABLES.iter().map(|spec| (spec, lv2_archive_dir()));
    let own = TABLES.iter().map(|spec| (spec, archive_dir()));
    let tables: Vec<lv2_archive::Table> = shared
        .chain(own)
        .map(|(spec, dir)| {
            lv2_archive::parse(spec, &read(&dir.join(spec.file())))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    assert_eq!(tables.len(), SHARED_TABLES.len() + TABLES.len());
    lv2_archive::check_references(&tables).unwrap_or_else(|e| panic!("{e}"));
}

/// Each number that CellGov's hypervisor answers and whose kernel sites
/// the census scanner misses. The scanner walks the words linearly and
/// stops at a `blr`, so it drops the `li r11` of an `sc` block that sits
/// after the function's `blr`. Each entry is the number, a firmware
/// whose kernel holds the site, and the site's real address, which
/// `hypercall_unresolved.tsv` must list for that kernel:
/// - 12, `connect_irq_plug_ext`
///   [OFW-4.93 lv2_kernel a:0x80000000000087a0-0x80000000000087f8].
const ANSWERED_WITH_MISSED_SITES: &[(u64, &str, u64)] = &[(12, "4.93", 0x87F8)];

#[test]
fn hypercall_site_rows_are_well_formed() {
    assert_eq!(HYPERCALL_SITE.gate, HYPERCALL_GATE);
    assert_eq!(HYPERCALL_UNRESOLVED.gate, HYPERCALL_GATE);
    let dir = archive_dir();
    let sites = committed_hypercall_sites();
    let unresolved = archive::hypercall_unresolved_rows(
        &lv2_archive::parse(
            &HYPERCALL_UNRESOLVED,
            &read(&dir.join(HYPERCALL_UNRESOLVED.file())),
        )
        .unwrap_or_else(|error| panic!("{error}")),
    );
    let kernel_pups: BTreeSet<String> = committed_kernels()
        .into_iter()
        .map(|row| row.pup_sha256)
        .collect();
    let scanned: BTreeSet<String> = unresolved
        .iter()
        .map(|row| row.pup_sha256.clone())
        .collect();
    assert_eq!(
        scanned, kernel_pups,
        "hypercall_unresolved.tsv must name every kernel.tsv kernel"
    );
    assert!(!scanned.is_empty(), "kernel.tsv names no kernel");
    for row in &sites {
        assert!(
            scanned.contains(&row.pup_sha256),
            "hypercall site row names an unscanned kernel: {row:?}"
        );
    }
    let ascending = |list: &[u64]| list.windows(2).all(|pair| pair[0] < pair[1]);
    for row in &sites {
        assert!(
            !row.sites.is_empty() && ascending(&row.sites),
            "hypercall site row sites are empty or not ascending: {row:?}"
        );
    }
    for row in &unresolved {
        assert!(
            ascending(&row.sites),
            "hypercall unresolved row sites are not ascending: {row:?}"
        );
    }
    // A missed site stays listed as unresolved, so the census cannot
    // give it to another number, and its number stays answered.
    let pups = committed_pups();
    for &(number, fw, site) in ANSWERED_WITH_MISSED_SITES {
        assert!(
            cellgov_lv1::ANSWERED.contains(&number),
            "hypercall {number} is no longer answered; \
             remove it from ANSWERED_WITH_MISSED_SITES"
        );
        let listed = pups.iter().filter(|pup| pup.fw == fw).any(|pup| {
            unresolved
                .iter()
                .any(|row| row.pup_sha256 == pup.pup_sha256 && row.sites.contains(&site))
        });
        assert!(
            listed,
            "the {fw} kernel's unresolved sites do not list {site:#x}, \
             the missed site of hypercall {number}"
        );
    }
    // Every number CellGov's hypervisor answers is one each kernel calls.
    for &number in cellgov_lv1::ANSWERED {
        let callers: BTreeSet<&str> = sites
            .iter()
            .filter(|row| row.number == number)
            .map(|row| row.pup_sha256.as_str())
            .collect();
        if ANSWERED_WITH_MISSED_SITES
            .iter()
            .any(|&(missed, _, _)| missed == number)
        {
            assert!(
                callers.is_empty(),
                "the census now finds a site of answered hypercall {number}; \
                 remove it from ANSWERED_WITH_MISSED_SITES"
            );
            continue;
        }
        assert_eq!(
            callers.len(),
            scanned.len(),
            "answered hypercall {number} has no site in some scanned kernel"
        );
    }
}
