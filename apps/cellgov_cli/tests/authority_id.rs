//! Titles boot under a retail-class program-authority-id, never the
//! system-class one.
//!
//! Retail SELFs (disc and NPDRM alike) all carry the same retail-app
//! authority id; what firmware gates on is the class. Serving a
//! system-class id makes libsysmodule's `module_start` skip init, and
//! its LoadModule sequence then fails on a never-created lwmutex.
//!
//! Titles come from the shared registry. Pending cells and
//! not-installed titles skip by name. At least one title must boot.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named pending and not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::process::Command;

use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use registry::{titles, workspace_root, TitleUnderTest};

#[path = "common/title_suite_checks.rs"]
mod title_suite_checks;
use title_suite_checks::{check_titles, AuthorityWitness};

fn parse_authority_witness(stderr: &str) -> Option<AuthorityWitness> {
    let rest = stderr
        .lines()
        .find_map(|l| l.strip_prefix("BENCH_AUTHORITY_ID_WITNESS:"))?;
    let mut authid = None;
    let mut source = None;
    let mut unknown = None;
    for tok in rest.split_whitespace() {
        if let Some(v) = tok.strip_prefix("program_authority_id=0x") {
            authid = u64::from_str_radix(v, 16).ok();
        } else if let Some(v) = tok.strip_prefix("authid_source=") {
            source = Some(v.to_string());
        } else if let Some(v) = tok.strip_prefix("lwmutex_unknown_locks=") {
            unknown = v.parse().ok();
        }
    }
    Some(AuthorityWitness {
        program_authority_id: authid?,
        authid_source: source?,
        lwmutex_unknown_locks: unknown?,
    })
}

/// Boot one title; `None` when its dump is not installed. Any other
/// failure panics: past the not-installed marker there is no
/// legitimate reason for this boot to break.
fn boot(title: &TitleUnderTest, force_system_authid: bool) -> Option<AuthorityWitness> {
    let cli_bin = env!("CARGO_BIN_EXE_cellgov");
    let mut cmd = Command::new(cli_bin);
    cmd.args(["boot", "bench-once"])
        .arg("--title")
        .arg(&title.short_name)
        .arg("--fw")
        .arg(&title.reference.fw)
        .arg("--max-steps")
        .arg(title.max_steps.to_string())
        .current_dir(workspace_root());
    // A firmware-shipped title has no game-version axis. The
    // composition refuses `--game-ver` for such a title.
    if let Some(v) = &title.reference.game_ver {
        cmd.arg("--game-ver").arg(v);
    }
    if force_system_authid {
        cmd.arg("--force-system-authid");
    }
    let output = cmd.output().expect("spawn cellgov boot bench-once");
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        if stderr.contains(TITLE_NOT_INSTALLED_SENTINEL) {
            return None;
        }
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        panic!(
            "{}: boot failed (exit {:?}). Last stderr lines (newest first):\n  {}",
            title.short_name,
            output.status.code(),
            tail.join("\n  ")
        );
    }
    Some(parse_authority_witness(&stderr).unwrap_or_else(|| {
        panic!(
            "{}: BENCH_AUTHORITY_ID_WITNESS line absent or unparseable",
            title.short_name
        )
    }))
}

#[test]
fn every_installed_title_is_served_its_own_authority_id() {
    let titles = titles();
    check_titles(&titles, boot);
}
