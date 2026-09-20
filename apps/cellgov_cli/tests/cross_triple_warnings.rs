//! `diff diverge` and `diff observations` report when their two sides
//! come from different identity triples.
//!
//! The fixtures are built in-test from `cellgov_compare`'s public
//! types and carry no title or external-data state.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]

use std::process::Command;

use cellgov_compare::{
    AppVersion, FirmwareIdentity, GameIdentity, Observation, ObservationMetadata, ObservedOutcome,
    RunIdentity,
};
use cellgov_testkit::scratch::scratch_labeled;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

fn identity(fw_version: &str) -> RunIdentity {
    RunIdentity {
        firmware: Some(FirmwareIdentity {
            version: fw_version.into(),
            image_version: format!("0x{}", fw_version.replace('.', "")),
            pup_sha256: "ab".repeat(32),
        }),
        game: Some(GameIdentity {
            title_id: "NPAA00001".into(),
            version: "base".into(),
            app_version: Some(AppVersion::AppVer("01.00".into())),
        }),
        overrides: Default::default(),
    }
}

fn observation(id: RunIdentity) -> Observation {
    Observation {
        outcome: ObservedOutcome::Completed,
        memory_regions: vec![cellgov_compare::NamedMemoryRegion {
            name: "scratch".into(),
            addr: 0x0001_0000,
            data: vec![0u8; 4],
        }],
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov-boot".into(),
            steps: Some(1),
        },
        tty_log: Vec::new(),
        identity: id,
        runner_firmware: None,
    }
}

/// The header for `id`, then one `PpuStateHash` record, so the scanner
/// has something to agree on past the header.
fn state_trace(id: &RunIdentity) -> Vec<u8> {
    let mut writer = TraceWriter::new();
    writer.record_header(&id.trace_header());
    writer.record(&TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x0001_0000,
        hash: StateHash::new(7),
    });
    writer.take_bytes()
}

/// The same stream with no header record, the shape of a trace written
/// before the header existed.
fn headerless_trace() -> Vec<u8> {
    let mut writer = TraceWriter::new();
    writer.record(&TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x0001_0000,
        hash: StateHash::new(7),
    });
    writer.take_bytes()
}

fn run(args: &[&std::path::Path], verb: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .arg("diff")
        .arg(verb)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn diverge_warns_when_the_two_traces_carry_different_triples() {
    let dir = scratch_labeled("diverge_cross");
    let a = dir.join("a.state");
    let b = dir.join("b.state");
    std::fs::write(&a, state_trace(&identity("4.91"))).unwrap();
    std::fs::write(&b, state_trace(&identity("4.93"))).unwrap();

    let out = run(&[&a, &b], "diverge");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cross-triple comparison"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("disagree on firmware"), "stderr: {stderr}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("IDENTICAL"),
        "the streams still agree past the header; the identity triple is context, not a verdict"
    );
}

#[test]
fn diverge_is_quiet_when_the_two_traces_carry_one_triple() {
    let dir = scratch_labeled("diverge_same");
    let a = dir.join("a.state");
    let b = dir.join("b.state");
    std::fs::write(&a, state_trace(&identity("4.91"))).unwrap();
    std::fs::write(&b, state_trace(&identity("4.91"))).unwrap();

    let out = run(&[&a, &b], "diverge");
    // A spawn that never parsed is silent too, so pin the verdict the
    // run has to have reached before reading anything into the silence.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("IDENTICAL"), "stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("cross-triple"), "stderr: {stderr}");
}

/// A zero fingerprint must not read as "no firmware".
#[test]
fn diverge_does_not_warn_when_one_trace_predates_the_header() {
    let dir = scratch_labeled("diverge_half");
    let a = dir.join("a.state");
    let b = dir.join("b.state");
    std::fs::write(&a, state_trace(&identity("4.91"))).unwrap();
    std::fs::write(&b, headerless_trace()).unwrap();

    let out = run(&[&a, &b], "diverge");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("cross-triple"), "stderr: {stderr}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("IDENTICAL"),
        "the header is skipped by the hash scan, so the two streams still match"
    );
}

#[test]
fn compare_observations_prints_both_triples_and_warns_across_them() {
    let dir = scratch_labeled("obs_cross");
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    for (path, fw) in [(&a, "4.91"), (&b, "4.93")] {
        let text = serde_json::to_string(&observation(identity(fw))).unwrap();
        std::fs::write(path, text).unwrap();
    }

    let out = run(&[&a, &b], "observations");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("4.91"), "stderr: {stderr}");
    assert!(stderr.contains("4.93"), "stderr: {stderr}");
    assert!(
        stderr.contains("cross-firmware comparison"),
        "stderr: {stderr}"
    );
    assert!(
        out.status.success(),
        "differing identity triples alone are not a divergence"
    );
}

#[test]
fn compare_observations_of_one_triple_prints_it_without_a_warning() {
    let dir = scratch_labeled("obs_same");
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    for path in [&a, &b] {
        let text = serde_json::to_string(&observation(identity("4.91"))).unwrap();
        std::fs::write(path, text).unwrap();
    }

    let stderr = String::from_utf8_lossy(&run(&[&a, &b], "observations").stderr).into_owned();
    assert!(stderr.contains("4.91"), "stderr: {stderr}");
    assert!(!stderr.contains("WARN"), "stderr: {stderr}");
}

/// A default identity makes no claim, so it contradicts nothing.
#[test]
fn compare_observations_names_an_unidentified_side_without_warning() {
    let dir = scratch_labeled("obs_half");
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    for (path, id) in [(&a, identity("4.91")), (&b, RunIdentity::default())] {
        let text = serde_json::to_string(&observation(id)).unwrap();
        std::fs::write(path, text).unwrap();
    }

    let stderr = String::from_utf8_lossy(&run(&[&a, &b], "observations").stderr).into_owned();
    assert!(stderr.contains("4.91"), "stderr: {stderr}");
    assert!(stderr.contains("(unidentified)"), "stderr: {stderr}");
    assert!(!stderr.contains("WARN"), "stderr: {stderr}");
}
