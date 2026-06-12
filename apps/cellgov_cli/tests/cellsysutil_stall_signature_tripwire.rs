//! cellSysutil honest-LLE stall-signature tripwire.
//!
//! Boots flOw with `CELLGOV_DISABLE_MODULE_START_HLE_STUBS=1` so
//! cellSysutil_Library module_start runs the real producer-consumer
//! path against the seeded ring, and asserts the declared-divergence
//! stall signature:
//!
//!   - module_start stalls AllBlocked at exactly 67 steps,
//!   - ring_wakes=0 (the cond[1] arm never fires under the V256
//!     seed),
//!   - cond0_producer_waits=1 (the terminal wait IS the producer-fed
//!     cond[0] record-finish wait, slot 0),
//!   - cond_signals=6 (the six non-zero-budget drain callsites),
//!     all on slot 0's cond[0] key.
//!
//! This is the adversarial guard for the cond[0]/cond[1]
//! discrimination in `Lv2Host::cond_ring_wake_check`: an arm that
//! wakes cond[0] waits live-locks the consumer (the post-wake guard
//! re-reads unchanged slot state and re-waits), producing a
//! MaxStepsExceeded spin at ~390k steps instead of the 67-step
//! AllBlocked stall -- this test goes red on that signature shift.
//!
//! Silent skip when fixtures (gitignored EBOOTs + firmware) are
//! absent; `CELLGOV_REQUIRE_FOUNDATION_TITLE_WITNESSES=1` promotes
//! that to a hard failure, matching the foundation-title tripwire.

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

/// See `foundation_title_witnesses_tripwire.rs` for the skip/fail
/// split this sentinel keys.
const BOOT_STARTED_SENTINEL: &str = "boot: --firmware-dir defaulted";

#[test]
fn flow_honest_lle_path_stalls_with_the_declared_signature() {
    if !workspace_root().join("docs/title_manifests").is_dir() {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "cellsysutil_stall_signature: docs/title_manifests/ absent and {REQUIRE_KNOB}=1"
            );
        }
        eprintln!("cellsysutil_stall_signature: skipping (docs/title_manifests/ absent)");
        return;
    }
    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let output = Command::new(cli_bin)
        .arg("bench-boot-once")
        .arg("--title")
        .arg("flow")
        .arg("--max-steps")
        .arg("1000000")
        .env("CELLGOV_DISABLE_MODULE_START_HLE_STUBS", "1")
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov_cli bench-boot-once");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if !combined.contains(BOOT_STARTED_SENTINEL) {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "cellsysutil_stall_signature: boot did not start (fixtures absent) \
                 and {REQUIRE_KNOB}=1",
            );
        }
        eprintln!(
            "cellsysutil_stall_signature: skipping \
             (fixtures absent -- boot sentinel {BOOT_STARTED_SENTINEL:?} not seen)"
        );
        return;
    }

    // The honest path must run (stub disabled), stall AllBlocked at
    // the exact anchor, and report the exact witness arithmetic.
    for needle in [
        "cellSysutil_Library HLE-stub DISABLED via env",
        "stalled after 67 steps (NoRunnableUnit/AllBlocked)",
        "module_start seed witnesses: ring_wakes=0 cond0_producer_waits=1 cond_signals=6",
        "module_start cond0 producer waits by slot: slot0=1",
        "module_start keyed cond signals: 0x8006010000000030=6",
    ] {
        assert!(
            combined.contains(needle),
            "cellsysutil_stall_signature: expected {needle:?} in the boot output. \
             A 67-step AllBlocked stall with ring_wakes=0 / cond0_producer_waits=1 / \
             cond_signals=6 (all on slot 0 cond[0]) is the declared-divergence \
             signature; a MaxStepsExceeded spin here means the ring-check arm woke a \
             cond[0] wait (discrimination regressed). Output:\n{combined}",
        );
    }
}
