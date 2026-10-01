//! Convention guard: every microtest has a console capture under the
//! reference profile, or a named gap in `tests/micro/ps3_gaps.tsv`.
//!
//! A missing capture is never a silent skip. The ledger names each gap
//! and its class: `pending` (portable, not yet captured),
//! `not-portable` (cannot run on hardware as written; the manifest says
//! so with the same reason) or `no-frame` (emits no CGOV frame).
//!
//! Two more guards hold the captures themselves: every committed capture
//! is well formed and carries no console identifier, and every byte an
//! emulator's observation answers differently from the console's is
//! classified in the test's `hardware_peer.tsv`. The console is the
//! reference; nothing is copied from either side.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cellgov_compare::baseline;
use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::{
    self, profile_directories, CaptureProvenance, CAPTURE_DIR, RUNNER_PS3_CEX, TRANSCRIPT_FILE,
};
use cellgov_compare::manifest;
use cellgov_compare::observation::{blank_volatile, NamedMemoryRegion, Observation};

/// The ledger, under the microtest root.
const LEDGER: &str = "ps3_gaps.tsv";

/// The ledger's header line.
const HEADER: &str = "name\tclass\treason\tissue";

/// The gap classes the ledger may name.
const CLASSES: &[&str] = &["pending", "not-portable", "no-frame"];

/// Floor on the microtest population, so a walk that finds nothing reads
/// as broken rather than as a tree with no gaps.
const MIN_MICROTESTS: usize = 20;

/// One ledger row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Gap {
    class: String,
    reason: String,
    issue: String,
}

fn micro_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro")
}

/// The ledger rows by microtest name, or every line that is malformed.
fn parse_ledger(text: &str) -> Result<BTreeMap<String, Gap>, Vec<String>> {
    let mut lines = text.lines();
    let mut problems = Vec::new();
    if lines.next() != Some(HEADER) {
        problems.push(format!("the first line is not the header {HEADER:?}"));
    }
    let mut rows = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let n = index + 2;
        let fields: Vec<&str> = line.split('\t').collect();
        let [name, class, reason, issue] = fields[..] else {
            problems.push(format!(
                "line {n}: {} fields, the header has 4",
                fields.len()
            ));
            continue;
        };
        if !CLASSES.contains(&class) {
            problems.push(format!(
                "line {n}: class {class:?} is not one of {CLASSES:?}"
            ));
        }
        if reason.trim().is_empty() {
            problems.push(format!("line {n}: {name} gives no reason"));
        }
        if issue != "-" && (issue.is_empty() || !issue.bytes().all(|b| b.is_ascii_digit())) {
            problems.push(format!(
                "line {n}: issue {issue:?} is neither a number nor -"
            ));
        }
        let gap = Gap {
            class: class.to_string(),
            reason: reason.to_string(),
            issue: issue.to_string(),
        };
        if rows.insert(name.to_string(), gap).is_some() {
            problems.push(format!("line {n}: {name} has a second row"));
        }
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}

/// Every microtest directory, by name.
fn microtests() -> BTreeMap<String, PathBuf> {
    let root = micro_root();
    fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_dir() && path.join("manifest.toml").is_file())
        .map(|path| {
            let name = path
                .file_name()
                .expect("a microtest directory has a name")
                .to_string_lossy()
                .into_owned();
            (name, path)
        })
        .collect()
}

#[test]
fn the_ledger_parser_reads_rows_and_names_every_malformed_line() {
    let good = format!("{HEADER}\nx\tpending\tnot yet\t12\ny\tno-frame\tlocal store only\t-\n");
    let rows = parse_ledger(&good).expect("parses");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows["y"].class, "no-frame");
    assert_eq!(rows["x"].issue, "12");

    let bad = format!(
        "{HEADER}\nx\tlater\twhy\t1\ny\tpending\t \t-\nz\tpending\twhy\t#3\nx\tpending\twhy\t1\nw\tpending\n"
    );
    let problems = parse_ledger(&bad).expect_err("refused");
    for said in [
        "class \"later\"",
        "y gives no reason",
        "issue \"#3\"",
        "x has a second row",
        "2 fields",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(said)),
            "{said:?} not among {problems:?}"
        );
    }
    assert!(
        parse_ledger("name,class\n").is_err(),
        "a wrong header is refused"
    );
}

#[test]
fn every_microtest_has_a_hardware_capture_or_a_named_gap() {
    let root = micro_root();
    let profiles = ConsoleProfiles::load(&root.join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load");
    let text = fs::read_to_string(root.join(LEDGER))
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.join(LEDGER).display()));
    let ledger = parse_ledger(&text)
        .unwrap_or_else(|problems| panic!("{LEDGER} is malformed:\n  {}", problems.join("\n  ")));
    let tests = microtests();
    assert!(
        tests.len() >= MIN_MICROTESTS,
        "gate went vacuous: only {} microtests under {}",
        tests.len(),
        root.display()
    );

    let mut problems = Vec::new();
    let mut uncovered = Vec::new();
    for (name, dir) in &tests {
        let reference = dir.join(CAPTURE_DIR).join(&profiles.reference);
        let captured = profile_directories(dir)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .contains(&reference);
        let manifest_path = dir.join("manifest.toml");
        let m = manifest::load_console(&manifest_path)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", manifest_path.display()));
        match (captured, ledger.get(name)) {
            (true, Some(gap)) => problems.push(format!(
                "{name} has a capture under {} and a {} row; drop the row",
                profiles.reference, gap.class
            )),
            (false, None) => uncovered.push(name.clone()),
            _ => {}
        }
        let not_portable = ledger.get(name).filter(|gap| gap.class == "not-portable");
        match (not_portable, m.ps3.portable) {
            (Some(_), true) => problems.push(format!(
                "{name} is not-portable in the ledger but its manifest's [ps3] portable is true"
            )),
            (Some(gap), false) if m.ps3.not_portable_reason.as_deref() != Some(gap.reason.as_str()) => {
                problems.push(format!(
                    "{name}: the ledger's reason {:?} and the manifest's not_portable_reason {:?} differ",
                    gap.reason, m.ps3.not_portable_reason
                ))
            }
            (None, false) => problems.push(format!(
                "{name}'s manifest says [ps3] portable = false but the ledger has no not-portable row"
            )),
            _ => {}
        }
    }
    for name in ledger.keys().filter(|name| !tests.contains_key(*name)) {
        problems.push(format!(
            "the ledger names {name}, which has no tests/micro/{name}/manifest.toml"
        ));
    }
    if !uncovered.is_empty() {
        problems.push(format!(
            "no capture under {} and no row in {LEDGER}: {}",
            profiles.reference,
            uncovered.join(", ")
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The peer table, in each microtest that has both a capture and
/// emulator baselines.
const PEER_TABLE: &str = "hardware_peer.tsv";

/// The peer table's header line.
const PEER_HEADER: &str = "region\tdecoder\toffset\tsize\thardware\tpeer\tclass\tnote";

/// The classes a peer row may name. The console is the reference, so a
/// difference is the emulator's (`peer-inaccuracy`), a value the
/// documents leave open that the two answer differently
/// (`implementation-defined`), or a byte a hardware run may vary that
/// the manifest has not yet declared volatile (`volatile`).
const PEER_CLASSES: &[&str] = &["peer-inaccuracy", "implementation-defined", "volatile"];

/// Where the emulator baselines sit, under the workspace's tests tree.
fn scenario_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/scenario_observations")
}

/// The decoder a baseline file names: `rpcs3_<decoder>.json`.
fn decoder_of(file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_str()?;
    stem.strip_prefix("rpcs3_").map(str::to_string)
}

/// One peer-table row: a byte span of one region where one decoder's
/// observation differs from the console's, with both sides' bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerRow {
    region: String,
    decoder: String,
    offset: usize,
    size: usize,
    hardware: String,
    peer: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The peer table's rows, or every malformed line. Lines starting `#`
/// are comments.
fn parse_peer_table(text: &str) -> Result<Vec<PeerRow>, Vec<String>> {
    let mut problems = Vec::new();
    let mut rows = Vec::new();
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    if lines.next() != Some(PEER_HEADER) {
        problems.push(format!("the first line is not the header {PEER_HEADER:?}"));
    }
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        let [region, decoder, offset, size, hardware, peer, class, note] = fields[..] else {
            problems.push(format!(
                "{line:?}: {} fields, the header has 8",
                fields.len()
            ));
            continue;
        };
        let (Ok(offset), Ok(size)) = (offset.parse::<usize>(), size.parse::<usize>()) else {
            problems.push(format!("{line:?}: offset or size is not a byte count"));
            continue;
        };
        if size == 0 {
            problems.push(format!("{line:?}: an empty span"));
        }
        if !PEER_CLASSES.contains(&class) {
            problems.push(format!(
                "{line:?}: class {class:?} is not one of {PEER_CLASSES:?}"
            ));
        }
        if note.trim().is_empty() {
            problems.push(format!("{line:?}: no note"));
        }
        rows.push(PeerRow {
            region: region.to_string(),
            decoder: decoder.to_string(),
            offset,
            size,
            hardware: hardware.to_string(),
            peer: peer.to_string(),
        });
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}

/// Hold one decoder's observation against the console's: every byte
/// that differs lies in a row of that decoder, and every row of that
/// decoder records both sides' bytes of its span and covers at least one
/// differing byte. Problems are pushed with `name` in front.
fn check_peer(
    name: &str,
    decoder: &str,
    console: &Observation,
    peer: &Observation,
    rows: &[PeerRow],
    problems: &mut Vec<String>,
) {
    let peer_regions: BTreeMap<&str, &[u8]> = peer
        .memory_regions
        .iter()
        .map(|r| (r.name.as_str(), r.data.as_slice()))
        .collect();
    for region in &console.memory_regions {
        let hw = region.data.as_slice();
        let Some(other) = peer_regions.get(region.name.as_str()) else {
            problems.push(format!("{name}: {decoder} has no region {}", region.name));
            continue;
        };
        if hw.len() != other.len() {
            problems.push(format!(
                "{name}: {decoder} holds region {} at {} bytes, the console at {}",
                region.name,
                other.len(),
                hw.len()
            ));
            continue;
        }
        let mine: Vec<&PeerRow> = rows
            .iter()
            .filter(|r| r.decoder == decoder && r.region == region.name)
            .collect();
        for row in &mine {
            let Some(end) = row.offset.checked_add(row.size).filter(|&e| e <= hw.len()) else {
                problems.push(format!("{name}: row {row:?} runs past the region"));
                continue;
            };
            let (h, p) = (&hw[row.offset..end], &other[row.offset..end]);
            if row.hardware != hex(h) || row.peer != hex(p) {
                problems.push(format!(
                    "{name}: row at {} {decoder} records {} / {}, the observations hold {} / {}",
                    row.offset,
                    row.hardware,
                    row.peer,
                    hex(h),
                    hex(p)
                ));
            }
            if h == p {
                problems.push(format!(
                    "{name}: row at {} {decoder} covers no difference; delete it",
                    row.offset
                ));
            }
        }
        let mut at = 0;
        while at < hw.len() {
            if hw[at] == other[at]
                || mine
                    .iter()
                    .any(|r| (r.offset..r.offset + r.size).contains(&at))
            {
                at += 1;
                continue;
            }
            let start = at;
            while at < hw.len() && hw[at] != other[at] {
                at += 1;
            }
            problems.push(format!(
                "{name}: unclassified, add to {PEER_TABLE}: {}\t{decoder}\t{start}\t{}\t{}\t{}\t<class>\t<note>",
                region.name,
                at - start,
                hex(&hw[start..at]),
                hex(&other[start..at])
            ));
        }
    }
}

/// The reference profile's capture of every microtest that has one.
fn reference_captures(profiles: &ConsoleProfiles) -> Vec<(String, PathBuf)> {
    microtests()
        .into_iter()
        .filter_map(|(name, dir)| {
            let capture = dir.join(CAPTURE_DIR).join(&profiles.reference);
            capture.is_dir().then_some((name, capture))
        })
        .collect()
}

fn tracked_profiles() -> ConsoleProfiles {
    ConsoleProfiles::load(&micro_root().join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load")
}

fn one_region_observation(data: Vec<u8>) -> Observation {
    let mut o: Observation = serde_json::from_str(
        r#"{"outcome":"Completed","memory_regions":[],"events":[],"state_hashes":null,"metadata":{"runner":"t","steps":null}}"#,
    )
    .expect("minimal observation");
    o.memory_regions.push(NamedMemoryRegion {
        name: "result".to_string(),
        addr: 0,
        data,
    });
    o
}

fn row(offset: usize, size: usize, hardware: &str, peer: &str) -> PeerRow {
    PeerRow {
        region: "result".to_string(),
        decoder: "llvm".to_string(),
        offset,
        size,
        hardware: hardware.to_string(),
        peer: peer.to_string(),
    }
}

#[test]
fn a_peer_row_must_cover_every_differing_byte_and_record_both_sides() {
    let hw = one_region_observation(vec![1, 2, 3, 4, 5, 6]);
    let peer = one_region_observation(vec![1, 9, 9, 4, 5, 7]);
    let check = |rows: &[PeerRow]| {
        let mut problems = Vec::new();
        check_peer("t", "llvm", &hw, &peer, rows, &mut problems);
        problems
    };
    assert!(check(&[row(0, 6, "010203040506", "010909040507")]).is_empty());
    assert!(check(&[row(1, 2, "0203", "0909"), row(5, 1, "06", "07")]).is_empty());

    let missing = check(&[row(1, 2, "0203", "0909")]);
    assert_eq!(missing.len(), 1);
    assert!(
        missing[0].contains("unclassified") && missing[0].contains("result\tllvm\t5\t1\t06\t07")
    );
    assert!(
        check(&[row(1, 2, "0203", "0808"), row(5, 1, "06", "07")])[0]
            .contains("records 0203 / 0808")
    );
    let idle = check(&[
        row(1, 2, "0203", "0909"),
        row(5, 1, "06", "07"),
        row(3, 2, "0405", "0405"),
    ]);
    assert!(idle[0].contains("covers no difference"), "{idle:?}");
    assert!(check(&[row(5, 9, "06", "07")])
        .iter()
        .any(|p| p.contains("runs past the region")));
    let mut other_decoder = check(&[]);
    other_decoder.sort();
    assert_eq!(other_decoder.len(), 2, "rows of no decoder cover nothing");
}

#[test]
fn the_peer_table_parser_reads_rows_and_names_every_malformed_line() {
    let good =
        format!("# a comment\n{PEER_HEADER}\nresult\tllvm\t4\t1\t01\t02\tpeer-inaccuracy\twhy\n");
    assert_eq!(parse_peer_table(&good).expect("parses").len(), 1);
    let bad = format!(
        "{PEER_HEADER}\nresult\tllvm\tfour\t1\t01\t02\tpeer-inaccuracy\twhy\nresult\tllvm\t4\t1\t01\t02\tguess\twhy\nresult\tllvm\t4\t1\t01\t02\tvolatile\t \nresult\tllvm\t4\t0\t\t\tvolatile\twhy\nresult\tllvm\n"
    );
    let problems = parse_peer_table(&bad).expect_err("refused");
    for said in [
        "not a byte count",
        "class \"guess\"",
        "no note",
        "an empty span",
        "2 fields",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(said)),
            "{said:?} not among {problems:?}"
        );
    }
}

#[test]
fn every_emulator_disagreement_with_hardware_is_classified() {
    let profiles = tracked_profiles();
    let mut compared = 0usize;
    let mut problems = Vec::new();
    for (name, capture) in reference_captures(&profiles) {
        let baselines = scenario_root().join(&name);
        if !baselines.is_dir() {
            continue;
        }
        let dir = micro_root().join(&name);
        let manifest_path = dir.join("manifest.toml");
        let m = manifest::load_console(&manifest_path)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", manifest_path.display()));
        let mut console = hardware_capture::load(&capture, &profiles)
            .unwrap_or_else(|e| panic!("{}: {e}", capture.display()))
            .observation;
        blank_volatile(&mut console, &m.ps3.volatile);
        let table_path = dir.join(PEER_TABLE);
        let rows = if table_path.is_file() {
            let text = fs::read_to_string(&table_path)
                .unwrap_or_else(|e| panic!("{}: {e}", table_path.display()));
            parse_peer_table(&text).unwrap_or_else(|p| {
                panic!(
                    "{} is malformed:\n  {}",
                    table_path.display(),
                    p.join("\n  ")
                )
            })
        } else {
            Vec::new()
        };
        let mut decoders = Vec::new();
        for (file, mut peer) in baseline::load_dir(&baselines)
            .unwrap_or_else(|e| panic!("{}: {e}", baselines.display()))
        {
            let Some(decoder) = decoder_of(&file) else {
                continue;
            };
            compared += 1;
            blank_volatile(&mut peer, &m.ps3.volatile);
            check_peer(&name, &decoder, &console, &peer, &rows, &mut problems);
            decoders.push(decoder);
        }
        for orphan in rows.iter().filter(|r| !decoders.contains(&r.decoder)) {
            problems.push(format!(
                "{name}: row {orphan:?} names a decoder with no baseline"
            ));
        }
    }
    assert!(
        compared >= 2,
        "gate went vacuous: only {compared} emulator baselines met a console capture"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The transcript tokens that would identify one console: a 32-hex-digit
/// run on a line naming `IDPS` or `PSID`, and a MAC-shaped token.
fn identifier_leaks(transcript: &str) -> Vec<String> {
    let mut leaks = Vec::new();
    for (n, line) in transcript.lines().enumerate() {
        if line.contains("IDPS") || line.contains("PSID") {
            let mut run = 0usize;
            for c in line.chars() {
                run = if c.is_ascii_hexdigit() { run + 1 } else { 0 };
                if run == 32 {
                    leaks.push(format!("line {}: a 32-hex token beside IDPS/PSID", n + 1));
                }
            }
        }
        for word in line.split(|c: char| !(c.is_ascii_hexdigit() || c == ':' || c == '-')) {
            let parts: Vec<&str> = word.split([':', '-']).collect();
            if parts.len() == 6 && parts.iter().all(|p| p.len() == 2) {
                leaks.push(format!("line {}: a MAC-shaped token", n + 1));
            }
        }
    }
    leaks
}

#[test]
fn the_identifier_scan_finds_ids_and_macs_and_spares_redactions() {
    assert_eq!(
        identifier_leaks("IDPS: 0123456789ABCDEF0123456789ABCDEF\n").len(),
        1
    );
    assert_eq!(identifier_leaks("MAC 02:00:5e:10:00:01\n").len(), 1);
    assert_eq!(identifier_leaks("mac 02-00-5E-10-00-01\n").len(), 1);
    assert!(
        identifier_leaks("IDPS: <id>\nMAC <mac>\nsha256 0123456789abcdef0123456789abcdef01\n")
            .is_empty()
    );
}

#[test]
fn every_hardware_capture_is_well_formed() {
    let profiles = tracked_profiles();
    let captures = reference_captures(&profiles);
    assert!(
        captures.len() >= 5,
        "gate went vacuous: only {} captures under {}",
        captures.len(),
        profiles.reference
    );
    let mut problems = Vec::new();
    for (name, capture) in captures {
        let loaded = match hardware_capture::load(&capture, &profiles) {
            Ok(loaded) => loaded,
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let observation = &loaded.observation;
        if observation.metadata.runner != RUNNER_PS3_CEX {
            problems.push(format!("{name}: runner {:?}", observation.metadata.runner));
        }
        if observation.state_hashes.is_some() {
            problems.push(format!("{name}: carries state hashes"));
        }
        let m = manifest::load_console(&micro_root().join(&name).join("manifest.toml"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let declared: Vec<(&str, u64)> = m
            .observe
            .memory_regions
            .iter()
            .map(|r| (r.name.as_str(), r.size))
            .collect();
        let held: Vec<(&str, u64)> = observation
            .memory_regions
            .iter()
            .map(|r| (r.name.as_str(), r.data.len() as u64))
            .collect();
        if declared != held {
            problems.push(format!(
                "{name}: regions {held:?}, the manifest declares {declared:?}"
            ));
        }
        let empty = empty_provenance_fields(&loaded.provenance);
        if !empty.is_empty() {
            problems.push(format!("{name}: empty provenance fields {empty:?}"));
        }
        let transcript = fs::read_to_string(capture.join(TRANSCRIPT_FILE))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for leak in identifier_leaks(&transcript) {
            problems.push(format!("{name}: {TRANSCRIPT_FILE} {leak}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The text fields of a provenance record that are empty.
fn empty_provenance_fields(p: &CaptureProvenance) -> Vec<&'static str> {
    [
        ("capture_id", p.capture_id.as_str()),
        ("captured_at", p.captured_at.as_str()),
        ("console.profile", p.console.profile.as_str()),
        ("console.model", p.console.model.as_str()),
        ("console.kernel", p.console.kernel.as_str()),
        ("console.firmware", p.console.firmware.as_str()),
        ("console.cfw", p.console.cfw.as_str()),
        ("console.cobra", p.console.cobra.as_str()),
        ("transport.kind", p.transport.kind.as_str()),
        ("harness.runner", p.harness.runner.as_str()),
        ("harness.revision", p.harness.revision.as_str()),
        ("harness.link", p.harness.link.as_str()),
        ("microtest.name", p.microtest.name.as_str()),
        (
            "microtest.manifest_sha256",
            p.microtest.manifest_sha256.as_str(),
        ),
        ("artifacts.eboot_sha256", p.artifacts.eboot_sha256.as_str()),
        (
            "artifacts.ps3_elf_sha256",
            p.artifacts.ps3_elf_sha256.as_str(),
        ),
        (
            "artifacts.reference_elf_sha256",
            p.artifacts.reference_elf_sha256.as_str(),
        ),
        (
            "artifacts.param_sfo_sha256",
            p.artifacts.param_sfo_sha256.as_str(),
        ),
        ("frame.result_path", p.frame.result_path.as_str()),
        ("digest", p.digest.as_str()),
    ]
    .into_iter()
    .filter(|(_, value)| value.trim().is_empty())
    .map(|(field, _)| field)
    .chain(
        p.microtest
            .sources
            .is_empty()
            .then_some("microtest.sources"),
    )
    .collect()
}
