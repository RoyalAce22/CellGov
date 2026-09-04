use std::collections::BTreeSet;

/// Every `"BENCH_<NAME>:` string literal in `source`: the prefixes of
/// the stderr lines the boot path emits. A literal inside an
/// `#[error(...)]` attribute is an error's Display text, not a line.
fn emitted_bench_prefixes(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (start, _) in source.match_indices("\"BENCH_") {
        if source[..start].trim_end().ends_with("#[error(") {
            continue;
        }
        let body = &source[start + 1..];
        let name_len = body
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        if body[name_len..].starts_with(':') {
            out.insert(body[..=name_len].to_string());
        }
    }
    out
}

/// The boot stage modules the `BENCH_` scan reads.
///
/// `include_str!` takes a literal path, so the list is hand-written;
/// [`the_bench_line_scan_reads_every_boot_stage_module`] holds it
/// against the directory.
const BOOT_SOURCES: [(&str, &str); 11] = [
    ("entry.rs", include_str!("../../boot/entry.rs")),
    ("finish.rs", include_str!("../../boot/finish.rs")),
    ("firmware.rs", include_str!("../../boot/firmware.rs")),
    ("host.rs", include_str!("../../boot/host.rs")),
    ("image.rs", include_str!("../../boot/image.rs")),
    ("loaders.rs", include_str!("../../boot/loaders.rs")),
    (
        "module_start.rs",
        include_str!("../../boot/module_start.rs"),
    ),
    ("params.rs", include_str!("../../boot/params.rs")),
    ("prepare.rs", include_str!("../../boot/prepare.rs")),
    ("providers.rs", include_str!("../../boot/providers.rs")),
    ("types.rs", include_str!("../../boot/types.rs")),
];

/// The `bench` submodules the `BENCH_` scan reads, hand-written for
/// the reason [`BOOT_SOURCES`] gives.
///
/// [`the_bench_line_scan_reads_every_bench_module`] holds it against
/// the directory.
const BENCH_SOURCES: [(&str, &str); 10] = [
    ("anchor.rs", include_str!("../anchor.rs")),
    ("divergence.rs", include_str!("../divergence.rs")),
    ("options.rs", include_str!("../options.rs")),
    ("result_line.rs", include_str!("../result_line.rs")),
    ("run_one.rs", include_str!("../run_one.rs")),
    ("runs.rs", include_str!("../runs.rs")),
    ("spawn.rs", include_str!("../spawn.rs")),
    ("throughput.rs", include_str!("../throughput.rs")),
    ("types.rs", include_str!("../types.rs")),
    ("witnesses.rs", include_str!("../witnesses.rs")),
];

/// Every `.rs` file directly under `dir`, which is itself relative to
/// the crate root.
///
/// The set omits `mod.rs`, which declares modules and re-exports only.
fn modules_on_disk(dir: &str) -> BTreeSet<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("module directory") {
        let path = entry.expect("module directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("module file name")
            .to_string();
        if name != "mod.rs" {
            out.insert(name);
        }
    }
    out
}

#[test]
fn the_bench_line_scan_reads_every_boot_stage_module() {
    let scanned: BTreeSet<String> = BOOT_SOURCES.iter().map(|(n, _)| (*n).to_string()).collect();
    assert_eq!(
        modules_on_disk("src/game/boot"),
        scanned,
        "boot stage modules the BENCH_ line scan does not read"
    );
}

#[test]
fn the_bench_line_scan_reads_every_bench_module() {
    let scanned: BTreeSet<String> = BENCH_SOURCES
        .iter()
        .map(|(n, _)| (*n).to_string())
        .collect();
    assert_eq!(
        modules_on_disk("src/game/bench"),
        scanned,
        "bench modules the BENCH_ line scan does not read"
    );
}

#[test]
fn every_emitted_bench_line_is_tracked_or_reasoned_diagnostic() {
    let mut emitted = BTreeSet::new();
    for source in [
        include_str!("../../child_init.rs"),
        include_str!("../../prx/module_start.rs"),
    ]
    .into_iter()
    .chain(BENCH_SOURCES.iter().map(|(_, source)| *source))
    .chain(BOOT_SOURCES.iter().map(|(_, source)| *source))
    {
        emitted.extend(emitted_bench_prefixes(source));
    }
    assert!(
        emitted.len() > 20,
        "the scan found only {emitted:?}; the literal shape it keys on has moved"
    );

    let tracked: BTreeSet<String> = cellgov_compare::witness_parse::tracked_line_prefixes()
        .into_iter()
        .map(str::to_string)
        .collect();
    let diagnostic: BTreeSet<String> = cellgov_compare::witness_parse::diagnostic_lines()
        .iter()
        .map(|(p, _)| (*p).to_string())
        .collect();

    let unclassified: Vec<&String> = emitted
        .iter()
        .filter(|p| !tracked.contains(*p) && !diagnostic.contains(*p))
        .collect();
    assert!(
        unclassified.is_empty(),
        "emitted BENCH_ lines with no witness and no stated diagnostic-only reason: {unclassified:?}"
    );

    let stale: Vec<&String> = tracked
        .iter()
        .chain(diagnostic.iter())
        .filter(|p| !emitted.contains(*p))
        .collect();
    assert!(
        stale.is_empty(),
        "line-table rows no emitter produces: {stale:?}"
    );
}
