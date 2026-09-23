//! `--progress` on a generated fuzz campaign never changes what stdout
//! says.
//!
//! Stderr is a pipe here, so the bar renders in `Plain`. Both runs use
//! one seed, one worker and one artifacts directory. A retained finding
//! then names the same artifact path on both stdouts, and the second
//! write of the same evidence is idempotent.

use std::path::Path;
use std::process::{Command, Output};

fn campaign(artifacts: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cellgov"))
        .args([
            "dev",
            "fuzz",
            "ppu-instruction",
            "--seed",
            "3",
            "--count",
            "200",
            "--workers",
            "1",
            "--artifacts-dir",
        ])
        .arg(artifacts)
        .args(extra)
        .output()
        .expect("spawn cellgov")
}

#[test]
fn the_bar_reaches_stderr_and_leaves_stdout_alone() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_progress_purity");
    let artifacts = scratch.join("findings");
    let with_bar = campaign(&artifacts, &["--progress"]);
    let without = campaign(&artifacts, &[]);
    let bar_stderr = String::from_utf8_lossy(&with_bar.stderr);
    let plain_stderr = String::from_utf8_lossy(&without.stderr);
    assert!(
        bar_stderr.contains("[fuzz]"),
        "--progress rendered no [fuzz] threshold line; stderr:\n{bar_stderr}"
    );
    assert!(
        !plain_stderr.contains("[fuzz]"),
        "a campaign without --progress rendered a [fuzz] line; stderr:\n{plain_stderr}"
    );
    assert_eq!(with_bar.status.code(), without.status.code());
    assert_eq!(
        String::from_utf8_lossy(&with_bar.stdout),
        String::from_utf8_lossy(&without.stdout),
        "the bar moved a byte of the result stream"
    );
    assert!(
        String::from_utf8_lossy(&with_bar.stdout).starts_with("fuzz: PpuInstruction cases=200"),
        "no summary line; stdout:\n{}",
        String::from_utf8_lossy(&with_bar.stdout)
    );
}

#[test]
fn quiet_silences_the_bar_the_flag_asked_for() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_progress_quiet");
    let quiet = campaign(&scratch.join("findings"), &["--progress", "--quiet"]);
    let stderr = String::from_utf8_lossy(&quiet.stderr);
    assert!(
        !stderr.contains("[fuzz]"),
        "--quiet still rendered a [fuzz] line; stderr:\n{stderr}"
    );
    assert!(String::from_utf8_lossy(&quiet.stdout).starts_with("fuzz: PpuInstruction cases=200"));
}
