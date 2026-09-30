//! `diff compare` refusal of a manifest region the run cannot read or
//! that declares zero bytes.
//!
//! The manifests name the synthetic `dma` scenario, so the tests need
//! no external data and no key vault.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use cellgov_testkit::scratch::scratch_labeled;
use std::path::Path;
use std::process::Command;

/// The `dma` scenario maps 256 bytes at address 0; 65536 is past the end.
const PAST_THE_END: &str = r#"
[test]
name = "region_past_the_end"

[cellgov]
scenario = "dma"

[observe]
memory_regions = [
  { name = "past_the_end", addr = 65536, size = 16 },
]

[expect]
outcome = "completed"
"#;

/// A synthetic scenario creates the boot space only.
const ABSENT_SPACE: &str = r#"
[test]
name = "region_in_absent_space"

[cellgov]
scenario = "dma"

[observe]
memory_regions = [
  { name = "ghost", space = 7, addr = 0, size = 16 },
]

[expect]
outcome = "completed"
"#;

/// The comparison pairs regions by name, so it would never compare the
/// second `twice`.
const DUPLICATE_NAME: &str = r#"
[test]
name = "region_named_twice"

[cellgov]
scenario = "dma"

[observe]
memory_regions = [
  { name = "twice", addr = 0, size = 16 },
  { name = "twice", addr = 16, size = 16 },
]

[expect]
outcome = "completed"
"#;

/// Zero bytes match any baseline.
const EMPTY: &str = r#"
[test]
name = "region_of_zero_bytes"

[cellgov]
scenario = "dma"

[observe]
memory_regions = [
  { name = "nothing", addr = 0, size = 0 },
]

[expect]
outcome = "completed"
"#;

fn save(manifest: &Path, baseline: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare"])
        .arg(manifest)
        .arg("--save-baseline")
        .arg(baseline)
        .output()
        .expect("spawn cellgov diff compare --save-baseline")
}

fn compare_against(manifest: &Path, baseline: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(["diff", "compare"])
        .arg(manifest)
        .arg("--against-baseline")
        .arg(baseline)
        .args(["--mode", "memory"])
        .output()
        .expect("spawn cellgov diff compare --against-baseline")
}

/// Both flags refuse the region before they touch the baseline file,
/// so the `--against-baseline` call needs no file at `baseline`.
fn assert_refused_naming_region(label: &str, manifest_text: &str, region: &str) {
    let scratch = scratch_labeled(&format!("compare_region_refusal_{label}"));
    let manifest = scratch.join(format!("{label}.toml"));
    let baseline = scratch.join(format!("{label}.json"));
    std::fs::write(&manifest, manifest_text).unwrap();

    let saved = save(&manifest, &baseline);
    let stderr = String::from_utf8_lossy(&saved.stderr);
    // 1 is `exit_codes::FAILED`: the operation ran and failed. A panic
    // exits 101.
    assert_eq!(
        saved.status.code(),
        Some(1),
        "{label}: --save-baseline on a region the run cannot read\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&saved.stdout),
    );
    assert!(
        stderr.contains(&format!("region {region} ")),
        "{label}: the refusal does not name the region\nstderr: {stderr}"
    );
    assert!(
        !baseline.exists(),
        "{label}: a refused save must not leave a baseline behind"
    );

    let against = compare_against(&manifest, &baseline);
    let stdout = String::from_utf8_lossy(&against.stdout);
    let stderr = String::from_utf8_lossy(&against.stderr);
    assert_eq!(
        against.status.code(),
        Some(1),
        "{label}: --against-baseline on a region the run cannot read\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("classification:"),
        "{label}: a refused region must not reach a classification\nstdout: {stdout}"
    );
    assert!(
        stderr.contains(&format!("region {region} ")),
        "{label}: the refusal does not name the region\nstderr: {stderr}"
    );
}

#[test]
fn a_region_past_the_end_of_the_space_does_not_round_trip_green() {
    assert_refused_naming_region("past_the_end", PAST_THE_END, "past_the_end");
}

#[test]
fn a_region_in_a_space_the_run_never_created_does_not_round_trip_green() {
    assert_refused_naming_region("absent_space", ABSENT_SPACE, "ghost");
}

#[test]
fn a_region_name_declared_twice_does_not_round_trip_green() {
    assert_refused_naming_region("duplicate_name", DUPLICATE_NAME, "twice");
}

#[test]
fn a_region_of_zero_bytes_does_not_round_trip_green() {
    assert_refused_naming_region("empty", EMPTY, "nothing");
}
