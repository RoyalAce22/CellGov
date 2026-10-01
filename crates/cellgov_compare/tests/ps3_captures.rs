//! Convention guard: every microtest has a console capture under the
//! reference profile, or a named gap in `tests/micro/ps3_gaps.tsv`.
//!
//! A missing capture is never a silent skip. The ledger names each gap
//! and its class: `pending` (portable, not yet captured),
//! `not-portable` (cannot run on hardware as written; the manifest says
//! so with the same reason) or `no-frame` (emits no CGOV frame).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::{profile_directories, CAPTURE_DIR};
use cellgov_compare::manifest;

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
