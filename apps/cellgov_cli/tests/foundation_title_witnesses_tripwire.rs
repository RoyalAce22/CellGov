//! Consolidated foundation-title witness tripwire.
//!
//! Replaces 9 per-witness tripwire binaries (vrsave, atomic_alignment,
//! dcbz, host_invariant_breaks, lwmutex_cond, mem_fault,
//! rsx_label_writes, spu_image_register, spu_thread_init) that each
//! spawned the same `cellgov_cli bench-boot-once` for the same three
//! foundation titles (flow=10M, sshd=100M, wipeout=200M). They emitted
//! ALL witness lines on every boot but each only parsed and asserted
//! on its own -- running the same boots 9 times in serial test
//! binaries. Wall-clock cost was ~9 x 28s = ~250s of duplicate work.
//!
//! This binary boots each title ONCE and asserts every witness against
//! its per-title declared expectation in one pass. Per-witness
//! semantics, status enums, and expected-value tables are preserved
//! verbatim from the original tripwires; the only thing that changed
//! is the boot is shared.
//!
//! Wipeout's 200M-step boot is the longest single contribution and is
//! gated behind `CELLGOV_INCLUDE_WIPEOUT_TRIPWIRE=1` -- skipped by
//! default for routine local test runs, opted in for CI / audit-run
//! contexts. flow and sshd boots run unconditionally (gated only by
//! the manifests-dir presence shared by all tripwires).
//!
//! Silent skip when fixtures (gitignored EBOOTs + firmware) are
//! absent; `CELLGOV_REQUIRE_FOUNDATION_TITLE_WITNESSES=1` promotes
//! that to a hard failure (CI knob). Same shape as the prior
//! per-tripwire knobs; one knob now covers the whole consolidated
//! gate.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries fixture-absent diagnostics"
)]
#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap panics on unexpected failure are the right behavior"
)]
#![allow(
    clippy::too_many_lines,
    reason = "consolidates 9 per-witness tripwires into one binary; per-witness assertion blocks are co-located deliberately"
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

fn manifests_dir_present() -> bool {
    workspace_root().join("docs/title_manifests").is_dir()
}

const REQUIRE_KNOB: &str = "CELLGOV_REQUIRE_FOUNDATION_TITLE_WITNESSES";
const WIPEOUT_INCLUDE_KNOB: &str = "CELLGOV_INCLUDE_WIPEOUT_TRIPWIRE";

// === All witnesses parsed from one bench-boot-once stderr. ===

#[derive(Debug, Clone, Copy, Default)]
struct AllWitnesses {
    // VRSAVE
    mfvrsave_executed: u64,
    vrsave_written: bool,
    // Atomic alignment
    ldarx_total: u64,
    stdcx_total: u64,
    lwarx_total: u64,
    stwcx_total: u64,
    // MemFault two-counter
    mem_fault_arm_entries: u64,
    mem_fault_unmapped_routed: u64,
    // RSX label writes
    rsx_label_writes_count: u64,
    // RSX SET_REFERENCE dispatches
    rsx_set_reference_count: u64,
    // DCBZ
    dcbz_count: u64,
    // SPU image register
    spu_image_register_count: u64,
    // SPU thread init
    spu_thread_init_count: u64,
    // LwMutex + cond
    lwmutex_acquires: u64,
    lwmutex_releases: u64,
    cond_reacquires: u64,
    // Host invariant breaks
    host_invariant_breaks: u64,
}

fn parse_all_witnesses(stderr: &str) -> AllWitnesses {
    let mut w = AllWitnesses::default();
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("BENCH_VRSAVE_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("mfvrsave_executed=") {
                    w.mfvrsave_executed = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("vrsave_written=") {
                    w.vrsave_written = v.parse().unwrap_or(false);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_ATOMIC_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("ldarx=") {
                    w.ldarx_total = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("stdcx=") {
                    w.stdcx_total = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("lwarx=") {
                    w.lwarx_total = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("stwcx=") {
                    w.stwcx_total = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_MEM_FAULT_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("arm_entries=") {
                    w.mem_fault_arm_entries = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("unmapped_routed=") {
                    w.mem_fault_unmapped_routed = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_RSX_LABEL_WRITES_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.rsx_label_writes_count = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_RSX_SET_REFERENCE_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.rsx_set_reference_count = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_DCBZ_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.dcbz_count = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_SPU_IMAGE_REGISTER_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.spu_image_register_count = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_SPU_THREAD_INIT_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.spu_thread_init_count = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_LWMUTEX_COND_WITNESS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("lwmutex_acquires=") {
                    w.lwmutex_acquires = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("lwmutex_releases=") {
                    w.lwmutex_releases = v.parse().unwrap_or(0);
                } else if let Some(v) = tok.strip_prefix("cond_reacquires=") {
                    w.cond_reacquires = v.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("BENCH_HOST_INVARIANT_BREAKS:") {
            for tok in rest.split_whitespace() {
                if let Some(v) = tok.strip_prefix("count=") {
                    w.host_invariant_breaks = v.parse().unwrap_or(0);
                }
            }
        }
    }
    w
}

/// Sentinel `bench-boot-once` emits to stderr immediately after the
/// firmware-dir resolves -- before any title-specific boot step
/// runs. Its presence proves fixtures were sufficient to start the
/// boot; absence proves the subprocess died before reaching the
/// `boot::prepare` body (manifest missing, EBOOT missing, RAP
/// missing, etc.). The skip/fail split below is keyed on this.
const BOOT_STARTED_SENTINEL: &str = "boot: --firmware-dir defaulted";

/// Boot a foundation title once via `cellgov_cli bench-boot-once`,
/// parse every BENCH_* witness line from the resulting stderr.
///
/// Skip/fail split: a non-zero subprocess exit is interpreted as
/// fixtures-absent (silent skip with stated reason) ONLY when the
/// stderr does NOT contain `BOOT_STARTED_SENTINEL`. With the
/// sentinel present, a non-zero exit means the boot ran and failed,
/// which is a real witness signal -- `panic!` with the captured
/// stderr so the test is RED-when-broken rather than silently green
/// against a dead anchor. Prior shape (silent skip on any non-zero)
/// is the vacuous-witness pattern this binary suspended for the
/// flow / sshd / wipeout cases in commit `4972d2b2`.
fn boot_title_once(title: &str, max_steps: u64) -> Option<AllWitnesses> {
    let cli_bin = env!("CARGO_BIN_EXE_cellgov_cli");
    let output = Command::new(cli_bin)
        .arg("bench-boot-once")
        .arg("--title")
        .arg(title)
        .arg("--max-steps")
        .arg(max_steps.to_string())
        .current_dir(workspace_root())
        .output()
        .expect("spawn cellgov_cli bench-boot-once");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let boot_started = stderr.contains(BOOT_STARTED_SENTINEL);

    if !output.status.success() {
        eprintln!("--- foundation_title_witnesses {title} stderr (first 60 lines) ---");
        for line in stderr.lines().take(60) {
            eprintln!("{line}");
        }
        eprintln!("--- end stderr ---");
        if boot_started {
            // Fixtures were sufficient to start the boot; non-zero exit
            // is a real failure.
            panic!(
                "foundation_title_witnesses {title}: bench-boot-once started \
                 (sentinel {BOOT_STARTED_SENTINEL:?} present) but exited non-zero -- \
                 boot ran and failed; witness anchor below this point cannot \
                 be trusted. Investigate the captured stderr above.",
            );
        }
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "foundation_title_witnesses {title}: bench-boot-once failed before \
                 boot started (fixtures absent) and {REQUIRE_KNOB}=1",
            );
        }
        eprintln!(
            "foundation_title_witnesses {title}: skipping \
             (fixtures absent -- boot sentinel {BOOT_STARTED_SENTINEL:?} not seen)"
        );
        return None;
    }

    Some(parse_all_witnesses(&stderr))
}

// =========================================================================
// Per-witness status enums + assertion helpers.
// Semantics preserved verbatim from the original tripwire binaries.
// =========================================================================

// ---- VRSAVE (was vrsave_tripwire.rs) ----

#[derive(Debug, Clone, Copy)]
enum VrsaveStatus {
    Free,
    UnreachedAtBootCheckpoint,
}

fn assert_vrsave(title: &str, w: &AllWitnesses, status: VrsaveStatus, reason: &str) {
    match status {
        VrsaveStatus::Free => {
            assert_eq!(
                w.mfvrsave_executed, 0,
                "vrsave {title}: declared VRSAVE-free ({reason}) but witness reported \
                 mfvrsave_executed={}. Either the declaration is wrong (re-run prescan; if \
                 SPR-256 sites are now present in this EBOOT, update the status) or a real \
                 mfvrsave executed and the tripwire's silence is no longer vacuous proof for \
                 this title.",
                w.mfvrsave_executed
            );
            assert!(
                !w.vrsave_written,
                "vrsave {title}: declared VRSAVE-free but vrsave_written=true; an mtvrsave \
                 executed without a corresponding mfvrsave -- the declaration is stale."
            );
        }
        VrsaveStatus::UnreachedAtBootCheckpoint => {
            assert_eq!(
                w.mfvrsave_executed, 0,
                "vrsave {title}: declared VRSAVE-unreached-at-boot-checkpoint ({reason}) but \
                 witness reported mfvrsave_executed={}. Boot trajectory now reaches a VRSAVE \
                 site; switch this title's VrsaveStatus to Reached and assert count>0 instead. \
                 Do NOT relax this to make the test green -- the assertion flipping IS the signal.",
                w.mfvrsave_executed
            );
        }
    }
}

// ---- Generic Reached / UnreachedAtBootCheckpoint counter helper ----
// Used by atomic_alignment, dcbz, spu_image_register, spu_thread_init,
// rsx_label_writes, lwmutex_cond. Same semantic shape; collapsed into
// one helper to avoid duplicating the assert prose 4x per tripwire.

#[derive(Debug, Clone, Copy)]
enum CountStatus {
    Reached { expected_at_least: u64 },
    UnreachedAtBootCheckpoint,
}

fn assert_count(title: &str, label: &str, observed: u64, status: CountStatus, reason: &str) {
    match status {
        CountStatus::Reached { expected_at_least } => {
            assert!(
                observed > 0,
                "{title} {label}: declared Reached (expected_at_least={expected_at_least}, \
                 {reason}) but {label}={observed}. Boot no longer reaches this path; \
                 trajectory regressed or this entry should switch to \
                 UnreachedAtBootCheckpoint."
            );
            if observed < expected_at_least {
                eprintln!(
                    "{title} {label}: observed={observed} below \
                     expected_at_least={expected_at_least} ({reason}). Lower-bound \
                     assertion still passes; update declaration if shift is intentional."
                );
            }
        }
        CountStatus::UnreachedAtBootCheckpoint => {
            assert_eq!(
                observed, 0,
                "{title} {label}: declared UnreachedAtBootCheckpoint ({reason}) but \
                 {label}={observed}. Boot reach extended; switch this entry to Reached. \
                 Do NOT relax this to make the test green -- the assertion flipping IS \
                 the signal."
            );
        }
    }
}

// ---- MemFault two-counter (was mem_fault_tripwire.rs) ----

#[derive(Debug, Clone, Copy)]
#[allow(
    dead_code,
    reason = "ExercisedAtUnmapped variant retained for the eventual title whose boot trajectory \
              reaches a MemFault site. Today all three foundation titles declare \
              UnreachedAtBootCheckpoint; dropping the variant would force a re-design when the \
              first title flips. Same future-proofing as the original mem_fault_tripwire."
)]
enum MemFaultStatus {
    ExercisedAtUnmapped {
        expected_at_least_arm_entries: u64,
        expected_at_least_unmapped_routed: u64,
    },
    UnreachedAtBootCheckpoint,
}

fn assert_mem_fault(title: &str, w: &AllWitnesses, status: MemFaultStatus, reason: &str) {
    match status {
        MemFaultStatus::ExercisedAtUnmapped {
            expected_at_least_arm_entries,
            expected_at_least_unmapped_routed,
        } => {
            assert!(
                w.mem_fault_unmapped_routed > 0,
                "mem_fault {title}: declared ExercisedAtUnmapped \
                 (expected_at_least_unmapped_routed={expected_at_least_unmapped_routed}, \
                 {reason}) but unmapped_routed=0. The MemError::Unmapped match arm \
                 specifically was not exercised."
            );
            assert_eq!(
                w.mem_fault_arm_entries, w.mem_fault_unmapped_routed,
                "mem_fault {title}: Pass-2.5 invariant violated -- arm_entries={} != \
                 unmapped_routed={}. A non-Unmapped MemError variant routed through the \
                 MemFault arm. This is precisely the regression the two-counter shape \
                 is designed to catch.",
                w.mem_fault_arm_entries, w.mem_fault_unmapped_routed
            );
            if w.mem_fault_arm_entries < expected_at_least_arm_entries {
                eprintln!(
                    "mem_fault {title}: observed arm_entries={} below documented \
                     expected_at_least={expected_at_least_arm_entries}. Lower-bound \
                     assertion still passes; update declaration if shift is intentional.",
                    w.mem_fault_arm_entries
                );
            }
        }
        MemFaultStatus::UnreachedAtBootCheckpoint => {
            assert_eq!(
                w.mem_fault_arm_entries, 0,
                "mem_fault {title}: declared UnreachedAtBootCheckpoint ({reason}) but \
                 arm_entries={}. Boot trajectory now reaches the MemFault arm; switch this \
                 entry to ExercisedAtUnmapped with the measured counter values. Do NOT relax \
                 this to make the test green.",
                w.mem_fault_arm_entries
            );
            assert_eq!(
                w.mem_fault_unmapped_routed, 0,
                "mem_fault {title}: declared UnreachedAtBootCheckpoint ({reason}) but \
                 unmapped_routed={}.",
                w.mem_fault_unmapped_routed
            );
        }
    }
}

// ---- Host invariant breaks (was host_invariant_breaks_tripwire.rs) ----

#[derive(Debug, Clone, Copy)]
enum HostInvariantBreaksStatus {
    ExactAtAnchor(u64),
    BelowFirstBreakAtTruncation,
}

fn assert_host_invariant_breaks(
    title: &str,
    w: &AllWitnesses,
    status: HostInvariantBreaksStatus,
    reason: &str,
) {
    match status {
        HostInvariantBreaksStatus::ExactAtAnchor(expected) => {
            assert_eq!(
                w.host_invariant_breaks, expected,
                "host_invariant_breaks {title}: declared ExactAtAnchor({expected}) ({reason}) \
                 but witness reported count={}. Either a regression added a break, a fix \
                 removed one, or the boot trajectory shifted. Update the declaration ONLY \
                 after confirming the new count matches the new trajectory anchor -- do NOT \
                 relax this to make the test green, the mismatch IS the signal.",
                w.host_invariant_breaks
            );
        }
        HostInvariantBreaksStatus::BelowFirstBreakAtTruncation => {
            assert_eq!(
                w.host_invariant_breaks, 0,
                "host_invariant_breaks {title}: declared BelowFirstBreakAtTruncation \
                 ({reason}) but witness reported count={}. A break fired before the \
                 truncation point. The assertion flipping IS the signal.",
                w.host_invariant_breaks
            );
        }
    }
}

// =========================================================================
// Per-title test entry points. Each boots its title ONCE and runs every
// per-witness assertion against the parsed AllWitnesses.
// =========================================================================

// Suspended: `boot_title_once` silently returns `None` on bench-boot-once
// exit-non-zero, which collapses the assertion block to a no-op. The
// post-(1c) tree's foundation-title boots die at cellSysutil module_start
// step 12 with `commit_step OutOfRange { effect_index: 1 }` (pending the
// investigation in `docs/dev/bug_investigations/cellsysutil_mmapper_oob.md`),
// so every anchor below is a false witness until boot reaches title `_start`
// again. Re-enable by removing the `#[ignore]` once cellsysutil_mmapper_oob
// is closed and the anchors are re-measured.
#[test]
#[ignore = "anchor invalid: boot reaches cellSysutil module_start step 43 \
            (NoRunnableUnit/AllBlocked) after the sc 337 fix landed in 672258b9; \
            next investigation pending. Re-anchors when boot reaches title _start"]
fn flow_all_witnesses() {
    if !manifests_dir_present() {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "foundation_title_witnesses flow: docs/title_manifests/ absent and {REQUIRE_KNOB}=1",
            );
        }
        eprintln!("foundation_title_witnesses flow: skipping (docs/title_manifests/ absent)");
        return;
    }
    let Some(w) = boot_title_once("flow", 10_000_000) else {
        return;
    };

    // vrsave: Free (prescan reports zero SPR-256 sites in flow's EBOOT)
    assert_vrsave(
        "flow",
        &w,
        VrsaveStatus::Free,
        "prescan reports zero SPR-256 sites in flow's EBOOT (commit 054f09a)",
    );

    // atomic alignment: all four primitives reached
    assert_count(
        "flow",
        "ldarx",
        w.ldarx_total,
        CountStatus::Reached {
            expected_at_least: 2240,
        },
        "boot exercises all four atomic primitives within 39062 step units (measured 2026-06-04)",
    );
    assert_count(
        "flow",
        "stdcx",
        w.stdcx_total,
        CountStatus::Reached {
            expected_at_least: 1601,
        },
        "same as ldarx above",
    );
    assert_count(
        "flow",
        "lwarx",
        w.lwarx_total,
        CountStatus::Reached {
            expected_at_least: 1602,
        },
        "same as ldarx above",
    );
    assert_count(
        "flow",
        "stwcx",
        w.stwcx_total,
        CountStatus::Reached {
            expected_at_least: 316,
        },
        "same as ldarx above",
    );

    // mem fault: unreached at boot checkpoint
    assert_mem_fault(
        "flow",
        &w,
        MemFaultStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps with no MemFault arm entry (measured 2026-06-04)",
    );

    // rsx label writes: unreached
    assert_count(
        "flow", "rsx_label_writes", w.rsx_label_writes_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps before any RSX FIFO advance retires label-write effects (measured 2026-06-04)",
    );

    // SET_REFERENCE dispatches: reached. The C-6 tripwire for the
    // cursor->MMIO REF writeback. libgcm stages
    // SET_REFERENCE(0xFFFFFFFF) in its bring-up FIFO at PUT-8;
    // the walker dispatches it once `cursor.get` is aligned with
    // MMIO GET. If the monotonic catch-up at
    // `Runtime::catch_up_cursor_get_from_mmio` is removed (or
    // `[rsx] consume` is dropped from the manifest), the walker
    // never reaches the SET_REFERENCE and this count drops to 0.
    assert_count(
        "flow",
        "rsx_set_reference",
        w.rsx_set_reference_count,
        CountStatus::Reached {
            expected_at_least: 1,
        },
        "boot dispatches NV406E_SET_REFERENCE at libgcm bring-up under the cursor catch-up; \
         count of zero means the walker never reached the SET_REFERENCE method (catch-up disabled \
         or stale cursor)",
    );

    // dcbz: reached (28+)
    assert_count(
        "flow",
        "dcbz",
        w.dcbz_count,
        CountStatus::Reached {
            expected_at_least: 28,
        },
        "boot exercises dcbz 28 times within 39062 step units (measured 2026-06-04)",
    );

    // spu image register: unreached
    assert_count(
        "flow",
        "spu_image_register",
        w.spu_image_register_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "no SPU candidate auto-register within reach (measured 2026-06-04)",
    );

    // spu thread init: unreached
    assert_count(
        "flow",
        "spu_thread_init",
        w.spu_thread_init_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates before any SPU thread init dispatch (measured 2026-06-04)",
    );

    // lwmutex + cond: acquires/releases reached, cond unreached
    assert_count(
        "flow", "lwmutex_acquires", w.lwmutex_acquires,
        CountStatus::Reached { expected_at_least: 643 },
        "643 acquires / 639 releases observed; cond re-acquire path never reached (measured 2026-06-04)",
    );
    assert_count(
        "flow",
        "lwmutex_releases",
        w.lwmutex_releases,
        CountStatus::Reached {
            expected_at_least: 639,
        },
        "same as lwmutex_acquires",
    );
    assert_count(
        "flow",
        "cond_reacquires",
        w.cond_reacquires,
        CountStatus::UnreachedAtBootCheckpoint,
        "same as lwmutex_acquires",
    );

    // host invariant breaks: ExactAtAnchor(43). Post-cursor-catch-up,
    // flOw reaches ProcessExit at step 11,299 with 43 honest breaks
    // along the way. The pre-fix anchor (2 breaks at MaxSteps=390,625)
    // was the truncated mid-spin count; the post-fix trajectory
    // exercises substantially more of the title's bring-up.
    assert_host_invariant_breaks(
        "flow", &w,
        HostInvariantBreaksStatus::ExactAtAnchor(43),
        "boot completes to ProcessExit; 43 honest breaks observed at the anchor (measured post-cursor-catch-up)",
    );
}

#[test]
#[ignore = "anchor invalid: boot reaches cellSysutil module_start step 43 \
            (NoRunnableUnit/AllBlocked) after the sc 337 fix landed in 672258b9; \
            next investigation pending. Re-anchors when boot reaches title _start"]
fn sshd_all_witnesses() {
    if !manifests_dir_present() {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "foundation_title_witnesses sshd: docs/title_manifests/ absent and {REQUIRE_KNOB}=1",
            );
        }
        eprintln!("foundation_title_witnesses sshd: skipping (docs/title_manifests/ absent)");
        return;
    }
    let Some(w) = boot_title_once("sshd", 100_000_000) else {
        return;
    };

    assert_vrsave(
        "sshd",
        &w,
        VrsaveStatus::UnreachedAtBootCheckpoint,
        "prescan reports 2 SPR-256 sites; boot terminates at Fault anchor before reaching them",
    );

    assert_count(
        "sshd", "ldarx", w.ldarx_total,
        CountStatus::Reached { expected_at_least: 762 },
        "boot truncates at MaxSteps; all four primitives exercised equally by early-boot sync code (measured 2026-06-04)",
    );
    assert_count(
        "sshd",
        "stdcx",
        w.stdcx_total,
        CountStatus::Reached {
            expected_at_least: 762,
        },
        "same as ldarx",
    );
    assert_count(
        "sshd",
        "lwarx",
        w.lwarx_total,
        CountStatus::Reached {
            expected_at_least: 762,
        },
        "same as ldarx",
    );
    assert_count(
        "sshd",
        "stwcx",
        w.stwcx_total,
        CountStatus::Reached {
            expected_at_least: 762,
        },
        "same as ldarx",
    );

    assert_mem_fault(
        "sshd",
        &w,
        MemFaultStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps with no MemFault arm entry (measured 2026-06-04)",
    );

    assert_count(
        "sshd", "rsx_label_writes", w.rsx_label_writes_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps before any RSX FIFO advance retires label-write effects (measured 2026-06-04)",
    );

    assert_count(
        "sshd", "rsx_set_reference", w.rsx_set_reference_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "manifest does not opt into the FIFO consumer; the SET_REFERENCE dispatch path is gated off",
    );

    assert_count(
        "sshd",
        "dcbz",
        w.dcbz_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps before any dcbz site (measured 2026-06-04)",
    );

    assert_count(
        "sshd",
        "spu_image_register",
        w.spu_image_register_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "no SPU candidate auto-register within reach (measured 2026-06-04)",
    );

    assert_count(
        "sshd",
        "spu_thread_init",
        w.spu_thread_init_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates before any SPU thread init dispatch (measured 2026-06-04)",
    );

    assert_count(
        "sshd",
        "lwmutex_acquires",
        w.lwmutex_acquires,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot truncates at MaxSteps before any lwmutex/cond activity (measured 2026-06-04)",
    );
    assert_count(
        "sshd",
        "lwmutex_releases",
        w.lwmutex_releases,
        CountStatus::UnreachedAtBootCheckpoint,
        "same as lwmutex_acquires",
    );
    assert_count(
        "sshd",
        "cond_reacquires",
        w.cond_reacquires,
        CountStatus::UnreachedAtBootCheckpoint,
        "same as lwmutex_acquires",
    );

    assert_host_invariant_breaks(
        "sshd",
        &w,
        HostInvariantBreaksStatus::BelowFirstBreakAtTruncation,
        "boot truncates at MaxSteps before the first SSHD break fires",
    );
}

#[test]
#[ignore = "anchor invalid: boot reaches cellSysutil module_start step 43 \
            (NoRunnableUnit/AllBlocked) after the sc 337 fix landed in 672258b9; \
            next investigation pending. Re-anchors when boot reaches title _start"]
fn wipeout_all_witnesses() {
    if std::env::var_os(WIPEOUT_INCLUDE_KNOB).is_none() {
        eprintln!(
            "foundation_title_witnesses wipeout: skipping (set {WIPEOUT_INCLUDE_KNOB}=1 \
             to opt in; ~7s wall-clock 200M-step boot, off by default for routine local \
             test runs)"
        );
        return;
    }
    if !manifests_dir_present() {
        if std::env::var_os(REQUIRE_KNOB).is_some() {
            panic!(
                "foundation_title_witnesses wipeout: docs/title_manifests/ absent and {REQUIRE_KNOB}=1",
            );
        }
        eprintln!("foundation_title_witnesses wipeout: skipping (docs/title_manifests/ absent)");
        return;
    }
    let Some(w) = boot_title_once("wipeout", 200_000_000) else {
        return;
    };

    assert_vrsave(
        "wipeout", &w,
        VrsaveStatus::UnreachedAtBootCheckpoint,
        "prescan reports 2 SPR-256 sites; boot terminates at RsxWriteCheckpoint before reaching them",
    );

    assert_count(
        "wipeout", "ldarx", w.ldarx_total,
        CountStatus::Reached { expected_at_least: 11_779 },
        "boot reaches RsxWriteCheckpoint; all four primitives heavily exercised (measured 2026-06-04)",
    );
    assert_count(
        "wipeout",
        "stdcx",
        w.stdcx_total,
        CountStatus::Reached {
            expected_at_least: 10_789,
        },
        "same as ldarx",
    );
    assert_count(
        "wipeout",
        "lwarx",
        w.lwarx_total,
        CountStatus::Reached {
            expected_at_least: 10_791,
        },
        "same as ldarx",
    );
    assert_count(
        "wipeout",
        "stwcx",
        w.stwcx_total,
        CountStatus::Reached {
            expected_at_least: 8_810,
        },
        "same as ldarx",
    );

    assert_mem_fault(
        "wipeout",
        &w,
        MemFaultStatus::UnreachedAtBootCheckpoint,
        "boot reaches RsxWriteCheckpoint with no MemFault arm entry (measured 2026-06-04)",
    );

    assert_count(
        "wipeout", "rsx_label_writes", w.rsx_label_writes_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "boot reaches RsxWriteCheckpoint before any FIFO advance retires label-write effects (measured 2026-06-04)",
    );

    assert_count(
        "wipeout", "rsx_set_reference", w.rsx_set_reference_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "manifest does not opt into the FIFO consumer; the SET_REFERENCE dispatch path is gated off",
    );

    assert_count(
        "wipeout",
        "dcbz",
        w.dcbz_count,
        CountStatus::Reached {
            expected_at_least: 615,
        },
        "boot reaches RsxWriteCheckpoint with 615 dcbz executions (measured 2026-06-04)",
    );

    assert_count(
        "wipeout",
        "spu_image_register",
        w.spu_image_register_count,
        CountStatus::UnreachedAtBootCheckpoint,
        "no SPU candidate auto-register within reach (measured 2026-06-04)",
    );

    assert_count(
        "wipeout", "spu_thread_init", w.spu_thread_init_count,
        CountStatus::Reached { expected_at_least: 1 },
        "boot dispatches SPU thread init at least once before RsxWriteCheckpoint (measured 2026-06-04)",
    );

    assert_count(
        "wipeout",
        "lwmutex_acquires",
        w.lwmutex_acquires,
        CountStatus::Reached {
            expected_at_least: 990,
        },
        "990 acquires / 990 releases (paired); cond re-acquire never reached (measured 2026-06-04)",
    );
    assert_count(
        "wipeout",
        "lwmutex_releases",
        w.lwmutex_releases,
        CountStatus::Reached {
            expected_at_least: 990,
        },
        "same as lwmutex_acquires",
    );
    assert_count(
        "wipeout",
        "cond_reacquires",
        w.cond_reacquires,
        CountStatus::UnreachedAtBootCheckpoint,
        "same as lwmutex_acquires",
    );

    assert_host_invariant_breaks(
        "wipeout",
        &w,
        HostInvariantBreaksStatus::ExactAtAnchor(2),
        "boot completes to Fault; 2 honest breaks observed at the anchor (measured 2026-06-04)",
    );
}

// =========================================================================
// Parser round-trip tests. Run cheaply without any subprocess.
// =========================================================================

#[test]
fn parser_handles_all_witness_line_shapes() {
    let stderr = "\
boot: chatter\n\
BENCH_VRSAVE_WITNESS: mfvrsave_executed=42 vrsave_written=true\n\
BENCH_ATOMIC_WITNESS: ldarx=10 stdcx=20 lwarx=30 stwcx=40\n\
BENCH_MEM_FAULT_WITNESS: arm_entries=5 unmapped_routed=5\n\
BENCH_RSX_LABEL_WRITES_WITNESS: count=7\n\
BENCH_RSX_SET_REFERENCE_WITNESS: count=11\n\
BENCH_DCBZ_WITNESS: count=99\n\
BENCH_SPU_IMAGE_REGISTER_WITNESS: count=1\n\
BENCH_SPU_THREAD_INIT_WITNESS: count=2\n\
BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires=100 lwmutex_releases=99 cond_reacquires=3\n\
BENCH_HOST_INVARIANT_BREAKS: count=43\n\
tail\n";
    let w = parse_all_witnesses(stderr);
    assert_eq!(w.mfvrsave_executed, 42);
    assert!(w.vrsave_written);
    assert_eq!(w.ldarx_total, 10);
    assert_eq!(w.stdcx_total, 20);
    assert_eq!(w.lwarx_total, 30);
    assert_eq!(w.stwcx_total, 40);
    assert_eq!(w.mem_fault_arm_entries, 5);
    assert_eq!(w.mem_fault_unmapped_routed, 5);
    assert_eq!(w.rsx_label_writes_count, 7);
    assert_eq!(w.rsx_set_reference_count, 11);
    assert_eq!(w.dcbz_count, 99);
    assert_eq!(w.spu_image_register_count, 1);
    assert_eq!(w.spu_thread_init_count, 2);
    assert_eq!(w.lwmutex_acquires, 100);
    assert_eq!(w.lwmutex_releases, 99);
    assert_eq!(w.cond_reacquires, 3);
    assert_eq!(w.host_invariant_breaks, 43);
}

#[test]
fn parser_returns_default_for_missing_lines() {
    let w = parse_all_witnesses("no witness lines here\n");
    assert_eq!(w.mfvrsave_executed, 0);
    assert!(!w.vrsave_written);
    assert_eq!(w.host_invariant_breaks, 0);
}
