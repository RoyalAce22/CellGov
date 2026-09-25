//! `diff diverge`, `diff observations` and `diff compare` exit 32 and
//! name the scheme mismatch when their two sides hold state hashes of
//! two schemes. `diff compare` does so against a baseline and against an
//! observations directory. A real divergence still exits 1.
//!
//! Each test builds its own fixtures, which carry no title or
//! external-data state.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use std::path::Path;
use std::process::{Command, Output};

use cellgov_compare::{
    Observation, ObservationMetadata, ObservedHashes, ObservedOutcome, CHECKPOINT_HASH_SCHEME,
};
use cellgov_ppu::multilinear::SCHEME_ID;
use cellgov_ppu::state::FNV1A_SCHEME_ID;
use cellgov_testkit::scratch::scratch_labeled;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

const EXIT_SCHEME_MISMATCH: i32 = 32;

fn cellgov(args: &[&str], paths: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args(args)
        .args(paths)
        .output()
        .unwrap()
}

/// A trace of one `PpuStateHash` record, after a scheme record when
/// `scheme` is `Some`. A trace without a scheme record reads as the
/// FNV-1a scheme.
fn state_trace(scheme: Option<u64>) -> Vec<u8> {
    let mut writer = TraceWriter::new();
    if let Some(ppu) = scheme {
        writer.record(&TraceRecord::StateHashScheme { ppu });
    }
    writer.record(&TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x0001_0000,
        hash: StateHash::new(7),
    });
    writer.take_bytes()
}

#[test]
fn diverge_exits_32_for_two_schemes() {
    let dir = scratch_labeled("scheme_diverge");
    let (a, b) = (dir.join("a.state"), dir.join("b.state"));
    std::fs::write(&a, state_trace(None)).unwrap();
    std::fs::write(&b, state_trace(Some(SCHEME_ID))).unwrap();
    let out = cellgov(&["diff", "diverge"], &[&a, &b]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    assert!(stdout.starts_with("SCHEME_MISMATCH"), "{stdout}");
    assert!(!stdout.contains("DIVERGE"), "{stdout}");
}

#[test]
fn diverge_of_one_scheme_still_compares() {
    let dir = scratch_labeled("scheme_diverge_same");
    let (a, b) = (dir.join("a.state"), dir.join("b.state"));
    std::fs::write(&a, state_trace(None)).unwrap();
    std::fs::write(&b, state_trace(Some(FNV1A_SCHEME_ID))).unwrap();
    let out = cellgov(&["diff", "diverge"], &[&a, &b]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("IDENTICAL"));
}

fn observation(scheme: u64) -> Observation {
    Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: Vec::new(),
        events: Vec::new(),
        state_hashes: Some(ObservedHashes {
            memory: StateHash::new(1),
            unit_status: StateHash::new(2),
            sync: StateHash::new(3),
            scheme,
        }),
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(1),
        },
        tty_log: Vec::new(),
        identity: Default::default(),
        runner_firmware: None,
    }
}

#[test]
fn observations_exit_32_for_two_schemes() {
    let dir = scratch_labeled("scheme_observations");
    let (a, b) = (dir.join("a.json"), dir.join("b.json"));
    for (path, scheme) in [(&a, FNV1A_SCHEME_ID), (&b, SCHEME_ID)] {
        std::fs::write(path, serde_json::to_string(&observation(scheme)).unwrap()).unwrap();
    }
    let out = cellgov(&["diff", "observations"], &[&a, &b]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    assert!(stdout.contains("SCHEME_MISMATCH"), "{stdout}");
    assert!(!stdout.contains("DIVERGE"), "{stdout}");
}

#[test]
fn compare_against_a_baseline_of_another_scheme_exits_32() {
    let dir = scratch_labeled("scheme_baseline");
    let baseline = dir.join("mailbox.json");
    let save = cellgov(
        &["diff", "compare", "mailbox", "--save-baseline"],
        &[&baseline],
    );
    assert!(save.status.success(), "{save:?}");
    let mut saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&baseline).unwrap()).unwrap();
    assert_eq!(saved["state_hashes"]["scheme"], CHECKPOINT_HASH_SCHEME);
    saved["state_hashes"]["scheme"] = SCHEME_ID.into();
    std::fs::write(&baseline, saved.to_string()).unwrap();

    let out = cellgov(
        &["diff", "compare", "mailbox", "--against-baseline"],
        &[&baseline],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    assert!(
        stdout.contains("classification: SCHEME_MISMATCH"),
        "{stdout}"
    );
}

/// A manifest whose CellGov side is the `mailbox` scenario, observed
/// with no memory region.
const MAILBOX_MANIFEST: &str = r#"
[test]
name = "scheme_manifest"

[cellgov]
scenario = "mailbox"

[observe]

[expect]
outcome = "completed"
"#;

/// Save `target`'s baseline to `baseline`, then restamp it with the
/// multilinear scheme id.
fn save_restamped_baseline(target: &str, baseline: &Path) {
    let save = cellgov(&["diff", "compare", target, "--save-baseline"], &[baseline]);
    assert!(save.status.success(), "{save:?}");
    let mut saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(baseline).unwrap()).unwrap();
    assert_eq!(saved["state_hashes"]["scheme"], CHECKPOINT_HASH_SCHEME);
    saved["state_hashes"]["scheme"] = SCHEME_ID.into();
    std::fs::write(baseline, saved.to_string()).unwrap();
}

#[test]
fn a_manifest_against_a_baseline_of_another_scheme_exits_32() {
    let dir = scratch_labeled("scheme_manifest_baseline");
    let manifest = dir.join("scheme.toml");
    std::fs::write(&manifest, MAILBOX_MANIFEST).unwrap();
    let manifest = manifest.to_str().unwrap();
    let baseline = dir.join("scheme.json");
    save_restamped_baseline(manifest, &baseline);

    let out = cellgov(
        &["diff", "compare", manifest, "--against-baseline"],
        &[&baseline],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    assert!(
        stdout.contains("classification: SCHEME_MISMATCH"),
        "{stdout}"
    );
}

#[test]
fn a_manifest_against_an_observations_dir_of_another_scheme_exits_32() {
    let dir = scratch_labeled("scheme_manifest_dir");
    let manifest = dir.join("scheme.toml");
    std::fs::write(&manifest, MAILBOX_MANIFEST).unwrap();
    let manifest = manifest.to_str().unwrap();
    let observations = dir.join("observations");
    std::fs::create_dir_all(&observations).unwrap();
    save_restamped_baseline(manifest, &observations.join("scheme.json"));

    let out = cellgov(
        &["diff", "compare", manifest, "--observations-dir"],
        &[&observations],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    assert!(
        stdout.contains("classification: SCHEME_MISMATCH"),
        "{stdout}"
    );
}

#[test]
fn a_json_compare_report_names_both_schemes() {
    let dir = scratch_labeled("scheme_baseline_json");
    let baseline = dir.join("mailbox.json");
    save_restamped_baseline("mailbox", &baseline);

    let out = cellgov(
        &[
            "diff",
            "compare",
            "mailbox",
            "--format",
            "json",
            "--against-baseline",
        ],
        &[&baseline],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["classification"], "scheme_mismatch", "{stdout}");
    assert_eq!(report["scheme_mismatch"]["expected"], SCHEME_ID, "{stdout}");
    assert_eq!(
        report["scheme_mismatch"]["actual"], CHECKPOINT_HASH_SCHEME,
        "{stdout}"
    );
}

#[test]
fn a_json_observations_report_names_the_scheme_mismatch() {
    let dir = scratch_labeled("scheme_observations_json");
    let (a, b) = (dir.join("a.json"), dir.join("b.json"));
    for (path, scheme) in [(&a, FNV1A_SCHEME_ID), (&b, SCHEME_ID)] {
        std::fs::write(path, serde_json::to_string(&observation(scheme)).unwrap()).unwrap();
    }
    let out = cellgov(&["diff", "observations", "--format", "json"], &[&a, &b]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(EXIT_SCHEME_MISMATCH), "{stdout}");
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        report["state_hash_compare"]["kind"], "scheme_mismatch",
        "{stdout}"
    );
    assert_eq!(report["state_hash_compare"]["b"], SCHEME_ID, "{stdout}");
}

#[test]
fn observations_that_diverge_exit_1_even_across_two_schemes() {
    let dir = scratch_labeled("scheme_observations_diverge");
    let (a, b) = (dir.join("a.json"), dir.join("b.json"));
    let mut faulted = observation(SCHEME_ID);
    faulted.outcome = ObservedOutcome::Fault;
    std::fs::write(
        &a,
        serde_json::to_string(&observation(FNV1A_SCHEME_ID)).unwrap(),
    )
    .unwrap();
    std::fs::write(&b, serde_json::to_string(&faulted).unwrap()).unwrap();
    let out = cellgov(&["diff", "observations"], &[&a, &b]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("DIVERGE"), "{stdout}");
}
