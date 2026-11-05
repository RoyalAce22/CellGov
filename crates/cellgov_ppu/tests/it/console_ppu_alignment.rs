//! Misaligned floating-point, load-multiple and store-multiple accesses,
//! and word-aligned accesses across a 64 KiB page, checked against the
//! console captures.
//!
//! The console ends the process when the EA of one of these accesses is
//! not a multiple of 4:
//!
//! - a floating-point load or store in any form (D, X, update and
//!   indexed update)
//! - an `lmw` or an `stmw`
//!
//! An `lfd`, `stfd` or `lwz` that crosses a 64 KiB page boundary
//! completes and moves the bytes in order.

use crate::console_cases::{capture, cellgov_after, table, Capture};
use cellgov_ppu::differential::{execute_state_in, CaseOutcome, PpuStateSnapshot};
use cellgov_ppu::exec::{ExecuteVerdict, PpuFault};

/// The done marker, "ALGN".
const DONE: u32 = 0x414C_474E;
/// The progress marker, "PROG".
const PROGRESS: u32 = 0x5052_4F47;
/// The progress step a hand-written probe records before its access.
const ACCESS_STEP: u32 = 2;

/// Returns a hand-written probe's sixteen payload words.
fn payload(probe: &str) -> [u32; 16] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 64, "{probe}");
    std::array::from_fn(|i| u32::from_be_bytes(p[4 * i..4 * i + 4].try_into().expect("4")))
}

/// The probe's 32-byte window as it was before the access: 0x10, 0x11, ...
fn window() -> Vec<u8> {
    (0x10..0x30).collect()
}

/// Runs `word` with r6 = `ea` over the probe's window, which starts at
/// `window_at`. Returns the state and the window after it.
fn run_at(
    word: u32,
    ea: u64,
    window_at: u64,
    set: impl FnOnce(&mut PpuStateSnapshot),
) -> Result<(PpuStateSnapshot, Vec<u8>), Box<CaseOutcome>> {
    let mut s = PpuStateSnapshot::zero();
    s.gpr[6] = ea;
    set(&mut s);
    execute_state_in(word, &s, window_at, window())
}

/// The big-endian words of `bytes`.
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4")))
        .collect()
}

#[test]
fn misaligned_floating_point_accesses_end_the_process_on_the_console_and_take_the_alignment_fault_in_cellgov(
) {
    for (op, store) in [
        ("lfs", false),
        ("lfd", false),
        ("stfs", true),
        ("stfd", true),
        ("stfiwx", true),
    ] {
        for k in 1..=3u64 {
            let name = format!("ppu_may_trap_{op}_m{k}");
            assert!(
                matches!(capture(&name, 1), Capture::Ended { case: 0 }),
                "{name}: the console ran it"
            );
            let cases = table(&name);
            assert_eq!(cases.len(), 1, "{name}: one case");
            assert!(cases[0].may_end_process, "{name}: a may-trap case");
            assert_eq!(
                cellgov_after(&cases[0], 0).map(|_| ()).map_err(|e| *e),
                Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                    PpuFault::alignment(k, store)
                ))),
                "{name}"
            );
        }
    }
}

#[test]
fn misaligned_indexed_and_update_floating_point_accesses_end_the_process_on_the_console_and_take_the_alignment_fault_in_cellgov(
) {
    // (probe, a store, the EA: the memory buffer sits at 0)
    for (name, store, ea) in [
        ("ppu_may_trap_lfsx_m1", false, 1),
        ("ppu_may_trap_lfdx_m2", false, 2),
        ("ppu_may_trap_lfsux_m3", false, 3),
        ("ppu_may_trap_lfdux_m1", false, 1),
        ("ppu_may_trap_lfsu_m3", false, 3),
        ("ppu_may_trap_lfdu_m1", false, 1),
        ("ppu_may_trap_stfsx_m2", true, 2),
        ("ppu_may_trap_stfdx_m3", true, 3),
        ("ppu_may_trap_stfsux_m1", true, 1),
        ("ppu_may_trap_stfdux_m2", true, 2),
        ("ppu_may_trap_stfsu_m2", true, 2),
        ("ppu_may_trap_stfdu_m3", true, 3),
    ] {
        assert!(
            matches!(capture(name, 1), Capture::Ended { case: 0 }),
            "{name}: the console ran it"
        );
        let cases = table(name);
        assert_eq!(cases.len(), 1, "{name}: one case");
        assert!(cases[0].may_end_process, "{name}: a may-trap case");
        assert_eq!(
            cellgov_after(&cases[0], 0).map(|_| ()).map_err(|e| *e),
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                PpuFault::alignment(ea, store)
            ))),
            "{name}"
        );
    }
}

#[test]
fn misaligned_lmw_and_stmw_end_the_process_on_the_console_and_take_the_alignment_fault_in_cellgov()
{
    // lmw r28,0(r6) and stmw r28,0(r6).
    for (op, word, store) in [("lmw", 0xBB86_0000, false), ("stmw", 0xBF86_0000, true)] {
        for k in 1..=3u64 {
            let name = format!("ppu_may_trap_{op}_m{k}");
            let console = payload(&name);
            assert_eq!(
                console[..2],
                [PROGRESS, ACCESS_STEP],
                "{name}: the run ends at the access"
            );
            let ea = u64::from(console[2]);
            assert_eq!(ea & 3, k, "{name}: the access's EA");
            // The window starts 16 bytes into the probe's buffer and the
            // EA is k bytes into the window.
            let window_at = ea - k;
            assert_eq!(
                run_at(word, ea, window_at, |_| {})
                    .map(|_| ())
                    .map_err(|e| *e),
                Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                    PpuFault::alignment(ea, store)
                ))),
                "{name}"
            );
        }
    }
}

#[test]
fn word_aligned_accesses_across_a_64k_page_complete_on_the_console_and_in_cellgov() {
    const PAGE: u64 = 0x1_0000;
    // (probe, word with RA = r6 and the data register 1 or 5, the EA's
    // offset below the page boundary)
    for (name, word, below) in [
        ("ppu_may_trap_lfd_page", 0xC826_0000u32, 4u64),
        ("ppu_may_trap_stfd_page", 0xD826_0000, 4),
        ("ppu_may_trap_lwz_page", 0x80A6_0000, 2),
    ] {
        let console = payload(name);
        assert_eq!(console[0], DONE, "{name}: the console completed the access");
        assert_eq!(console[3], 0, "{name}: the allocate succeeded");
        let ea = u64::from(console[2]);
        assert_eq!(ea % PAGE, PAGE - below, "{name}: the EA");
        // The window holds the 16 bytes on each side of the boundary.
        let window_at = ea + below - 16;
        let (after, bytes) = run_at(word, ea, window_at, |s| s.fpr[1] = 0x4009_21FB_5444_2D18)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(words(&bytes), console[8..], "{name}: the window");
        match name {
            "ppu_may_trap_lfd_page" => assert_eq!(
                after.fpr[1],
                u64::from(console[4]) << 32 | u64::from(console[5]),
                "{name}: f1"
            ),
            "ppu_may_trap_lwz_page" => {
                assert_eq!(after.gpr[5], u64::from(console[4]), "{name}: r5");
            }
            _ => {}
        }
    }
}
