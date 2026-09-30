//! `diff compare` given a baseline flag and a manifest that names no
//! scenario this runner has.
//!
//! A plain run reports such a manifest UNSUPPORTED and exits 0. A run
//! asked to record or compare a baseline has nothing to record and
//! nothing to compare, so it refuses: a scripted refresh over a
//! directory of manifests must not see green for a manifest that
//! produced no file.
//!
//! Neither manifest runs anything, so the tests need no external data and no
//! key vault.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use cellgov_testkit::scratch::scratch_labeled;
use std::path::Path;
use std::process::Command;

const NO_CELLGOV_SECTION: &str = r#"
[test]
name = "no_cellgov_section"

[observe]

[expect]
outcome = "completed"
"#;

const UNKNOWN_SCENARIO: &str = r#"
[test]
name = "unknown_scenario"

[cellgov]
scenario = "no_such_scenario"

[observe]

[expect]
outcome = "completed"
"#;

fn compare(manifest: &Path, flag: &str, value: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare"])
        .arg(manifest)
        .arg(flag)
        .arg(value)
        .output()
        .expect("spawn cellgov diff compare")
}

/// Every baseline flag refuses before it touches the baseline path, so
/// `--against-baseline` needs no file and `--observations-dir` no
/// directory.
fn assert_refused(label: &str, manifest_text: &str, section_text: &str) {
    let scratch = scratch_labeled(&format!("compare_unsupported_baseline_{label}"));
    let manifest = scratch.join(format!("{label}.toml"));
    let baseline = scratch.join(format!("{label}.json"));
    let observations = scratch.join("observations");
    std::fs::write(&manifest, manifest_text).unwrap();

    for (flag, value) in [
        ("--save-baseline", &baseline),
        ("--against-baseline", &baseline),
        ("--observations-dir", &observations),
    ] {
        let out = compare(&manifest, flag, value);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // 1 is `exit_codes::FAILED`: the operation ran and failed. A
        // panic exits 101.
        assert_eq!(
            out.status.code(),
            Some(1),
            "{label} {flag}: an unsupported manifest given a baseline flag\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            !stdout.contains("classification:"),
            "{label} {flag}: a refusal must not reach a classification\nstdout: {stdout}"
        );
        for expected in [manifest.display().to_string().as_str(), section_text, flag] {
            assert!(
                stderr.contains(expected),
                "{label} {flag}: the refusal does not name {expected:?}\nstderr: {stderr}"
            );
        }
    }
    assert!(
        !baseline.exists(),
        "{label}: a refused save must not leave a baseline behind"
    );
}

#[test]
fn a_manifest_with_no_cellgov_section_given_a_baseline_flag_is_refused() {
    assert_refused("no_cellgov_section", NO_CELLGOV_SECTION, "[cellgov]");
}

#[test]
fn a_manifest_naming_an_unknown_scenario_given_a_baseline_flag_is_refused() {
    assert_refused("unknown_scenario", UNKNOWN_SCENARIO, "no_such_scenario");
}
