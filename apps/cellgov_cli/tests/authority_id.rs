//! Titles boot under a retail-class program-authority-id, never the
//! system-class one.
//!
//! Retail SELFs (disc and NPDRM alike) all carry the same retail-app
//! authority id; what firmware gates on is the class. Serving a
//! system-class id makes libsysmodule's `module_start` skip init, and
//! its LoadModule sequence then fails on a never-created lwmutex.
//!
//! Titles come from the shared registry; not-installed titles skip by
//! name and at least one must boot, mirroring `title_witnesses`.

#![allow(
    clippy::print_stderr,
    reason = "integration test: named not-installed skips are its only output channel"
)]

#[path = "common/registry.rs"]
mod registry;

use std::process::Command;

use cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL;
use cellgov_ps3_abi::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID;
use registry::{titles, workspace_root, TitleUnderTest};

struct AuthorityWitness {
    program_authority_id: u64,
    authid_source: String,
    lwmutex_unknown_locks: u64,
}

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
    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let mut cmd = Command::new(cli_bin);
    cmd.arg("bench-boot-once")
        .arg("--title")
        .arg(&title.short_name)
        .arg("--max-steps")
        .arg(title.max_steps.to_string())
        .current_dir(workspace_root());
    if force_system_authid {
        cmd.env("CELLGOV_FORCE_SYSTEM_AUTHID", "1");
    }
    let output = cmd.output().expect("spawn cellgov_cli bench-boot-once");
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
    let mut checked = 0usize;
    let mut skipped: Vec<&str> = Vec::new();
    let mut any_forced_delta = false;
    for title in &titles {
        let Some(normal) = boot(title, false) else {
            eprintln!(
                "{}: skipped -- not installed on this machine",
                title.short_name
            );
            skipped.push(&title.short_name);
            continue;
        };
        checked += 1;
        assert_eq!(
            normal.authid_source, "self",
            "{}: the served authority id must come from the SELF \
             identification header, not the shared retail fallback -- \
             a fallback here means the SELF parse regressed",
            title.short_name,
        );
        assert_ne!(
            normal.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: normal boot served the system-class bdj.self id; \
             titles must boot under a retail-class authority id",
            title.short_name,
        );
        assert_eq!(
            normal.lwmutex_unknown_locks, 0,
            "{}: unknown-lwmutex lock failures under a retail-class authid",
            title.short_name,
        );
        let forced = boot(title, true).expect("installed above; forced boot must also start");
        assert_eq!(
            forced.program_authority_id, BDJ_SELF_PROGRAM_AUTHORITY_ID,
            "{}: CELLGOV_FORCE_SYSTEM_AUTHID must serve the bdj.self constant",
            title.short_name,
        );
        if forced.lwmutex_unknown_locks > normal.lwmutex_unknown_locks {
            any_forced_delta = true;
        }
    }
    // Anti-vacuity floor, shared with title_witnesses: the feature
    // declares a corpus, so a run that booted nothing must not pass.
    assert!(
        checked > 0,
        "title-corpus is enabled but none of the {} registered title(s) is \
         installed (skipped: {})",
        titles.len(),
        skipped.join(", ")
    );
    // Negative control: the regression signature the `== 0` asserts
    // gate must still be reproducible somewhere in the installed set.
    assert!(
        any_forced_delta,
        "no installed title shows more unknown-lwmutex failures under the \
         forced system authid than under its own id; the class pin has lost \
         its negative control and the `== 0` assertions are unfalsifiable"
    );
}
