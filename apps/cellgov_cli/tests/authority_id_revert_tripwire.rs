//! Adversarial revert tripwire for the per-title program-authority-id
//! fix.
//!
//! Invariant pinned: the per-title authid is what retires the
//! cellSysmodule LoadModule failures. With the per-title authid
//! (`0x1010_0000_0100_0003`) libsysmodule's module_start runs full
//! init and creates the lwmutex every `cellSysmoduleLoadModule` locks,
//! so `lwmutex_unknown_locks=0`. Restoring the pre-fix bdj.self /
//! PAID_44 system authid (`CELLGOV_FORCE_SYSTEM_AUTHID=1`) skips that
//! init, so the LoadModule sequence locks the never-created id-0
//! lwmutex and fails ESRCH ten times (`lwmutex_unknown_locks=10`).
//!
//! Silent skip when fixtures (gitignored EBOOT + firmware) are
//! absent; `CELLGOV_REQUIRE_FOUNDATION_TITLE_WITNESSES=1` promotes
//! that to a hard failure.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries fixture-absent diagnostics"
)]

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if std::fs::read_to_string(p.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return p;
        }
        if !p.pop() {
            panic!(
                "workspace root not found above {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

const REQUIRE_KNOB: &str = "CELLGOV_REQUIRE_FOUNDATION_TITLE_WITNESSES";
const BOOT_STARTED_SENTINEL: &str = "boot: --firmware-dir defaulted";

/// Parsed `BENCH_AUTHORITY_ID_WITNESS` fields.
struct AuthorityWitness {
    program_authority_id: u64,
    lwmutex_unknown_locks: u64,
}

fn parse_authority_witness(stderr: &str) -> Option<AuthorityWitness> {
    let rest = stderr
        .lines()
        .find_map(|l| l.strip_prefix("BENCH_AUTHORITY_ID_WITNESS:"))?;
    let mut authid = None;
    let mut unknown = None;
    for tok in rest.split_whitespace() {
        if let Some(v) = tok.strip_prefix("program_authority_id=0x") {
            authid = u64::from_str_radix(v, 16).ok();
        } else if let Some(v) = tok.strip_prefix("lwmutex_unknown_locks=") {
            unknown = v.parse().ok();
        }
    }
    Some(AuthorityWitness {
        program_authority_id: authid?,
        lwmutex_unknown_locks: unknown?,
    })
}

/// Boot flOw once, optionally forcing the pre-fix system authid.
/// Returns `None` when fixtures are absent (skip).
fn boot_flow(force_system_authid: bool) -> Option<AuthorityWitness> {
    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let mut cmd = Command::new(cli_bin);
    cmd.arg("bench-boot-once")
        .arg("--title")
        .arg("flow")
        .arg("--max-steps")
        .arg("10000000")
        .current_dir(workspace_root());
    if force_system_authid {
        cmd.env("CELLGOV_FORCE_SYSTEM_AUTHID", "1");
    }
    let output = cmd.output().expect("spawn cellgov_cli bench-boot-once");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stderr.contains(BOOT_STARTED_SENTINEL) {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "authority_id_revert: boot did not start (fixtures absent) and {REQUIRE_KNOB}=1"
            );
        }
        eprintln!(
            "authority_id_revert: skipping (fixtures absent -- boot sentinel \
             {BOOT_STARTED_SENTINEL:?} not seen)"
        );
        return None;
    }
    Some(parse_authority_witness(&stderr).unwrap_or_else(|| {
        panic!("authority_id_revert: BENCH_AUTHORITY_ID_WITNESS line absent or unparseable")
    }))
}

#[test]
fn flow_default_authid_retires_loadmodule_lock_failures() {
    let Some(w) = boot_flow(false) else {
        return;
    };
    assert_eq!(
        w.program_authority_id, 0x1010_0000_0100_0003,
        "default boot must serve flOw's retail program-authority-id",
    );
    assert_eq!(
        w.lwmutex_unknown_locks, 0,
        "with the per-title authid, libsysmodule creates its lwmutex; \
         zero LoadModule lock failures expected",
    );
}

#[test]
fn flow_forced_system_authid_reintroduces_the_failure_signature() {
    let Some(w) = boot_flow(true) else {
        return;
    };
    assert_eq!(
        w.program_authority_id, 0x1070_0000_3A00_0001,
        "forced boot must serve the pre-fix bdj.self (PAID_44) system authid",
    );
    assert_eq!(
        w.lwmutex_unknown_locks, 10,
        "restoring the system authid skips libsysmodule's lwmutex creation; \
         the LoadModule sequence locks the never-created id-0 lwmutex 10 times. \
         This count flipping from 0 IS the proof that the per-title authid is \
         what retired the failures.",
    );
}
