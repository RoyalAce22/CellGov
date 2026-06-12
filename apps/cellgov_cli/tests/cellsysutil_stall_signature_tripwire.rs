//! cellSysutil honest-LLE stall-signature tripwire.
//!
//! Invariant pinned: the cond[0]/cond[1] discrimination in
//! `Lv2Host::cond_ring_wake_check`. An arm that wakes cond[0] waits
//! live-locks the consumer (the post-wake guard re-reads unchanged
//! slot state and re-waits), producing a MaxStepsExceeded spin instead
//! of the declared 67-step AllBlocked stall. Booting flOw with
//! `CELLGOV_DISABLE_MODULE_START_HLE_STUBS=1` runs cellSysutil_Library
//! module_start's real producer-consumer path against the seeded V256
//! ring; the test goes red on any shift from the signature (67-step
//! stall, ring_wakes=0, cond0_producer_waits=1, cond_signals=6 all on
//! slot 0's cond[0] key).
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

    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stderr.contains(BOOT_STARTED_SENTINEL) {
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

    let w = parse_cellsysutil_witness(&stderr).unwrap_or_else(|| {
        panic!(
            "cellsysutil_stall_signature: BENCH_CELLSYSUTIL_SEED_WITNESS line absent \
             or unparseable.\n{stderr}"
        )
    });

    assert_eq!(w.module, "cellSysutil_Library", "stub-disabled module name");
    assert_eq!(w.stalled, 1, "module_start must stall (not return)");
    assert_eq!(w.steps, 67, "stall at the declared 67-step anchor");
    assert_eq!(w.ring_wakes, 0, "cond[1] ring-check arm must never fire");
    assert_eq!(
        w.cond0_producer_waits, 1,
        "terminal wait is the producer-fed cond[0] record-finish wait",
    );
    assert_eq!(w.slot0_producer_waits, 1, "the producer wait is on slot 0");
    assert_eq!(
        w.cond_signals, 6,
        "six non-zero-budget drain callsites signal",
    );
    assert_eq!(
        w.cond0_slot0_signals, 6,
        "all six signals land on slot 0 cond[0]",
    );
}

struct CellsysutilWitness {
    module: String,
    stalled: u8,
    steps: u64,
    ring_wakes: u64,
    cond0_producer_waits: u64,
    slot0_producer_waits: u64,
    cond_signals: u64,
    cond0_slot0_signals: u64,
}

fn parse_cellsysutil_witness(stderr: &str) -> Option<CellsysutilWitness> {
    let rest = stderr
        .lines()
        .find_map(|l| l.strip_prefix("BENCH_CELLSYSUTIL_SEED_WITNESS:"))?;
    let mut module = None;
    let mut fields: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for tok in rest.split_whitespace() {
        let (k, v) = tok.split_once('=')?;
        if k == "module" {
            module = Some(v.to_string());
        } else {
            fields.insert(k, v.parse().ok()?);
        }
    }
    Some(CellsysutilWitness {
        module: module?,
        stalled: u8::try_from(*fields.get("stalled")?).ok()?,
        steps: *fields.get("steps")?,
        ring_wakes: *fields.get("ring_wakes")?,
        cond0_producer_waits: *fields.get("cond0_producer_waits")?,
        slot0_producer_waits: *fields.get("slot0_producer_waits")?,
        cond_signals: *fields.get("cond_signals")?,
        cond0_slot0_signals: *fields.get("cond0_slot0_signals")?,
    })
}
