//! The registers a new PPU unit starts with, against the
//! `ppu_entry_state` console capture of a main thread and a created
//! thread.
//!
//! LV2's process and thread setup writes the GPRs, LR and CTR at entry,
//! so this file leaves them out.

use cellgov_event::UnitId;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::PpuExecutionUnit;

const CONSOLE: &str = "../../tests/micro/ppu_entry_state/ps3/cech20-cex-493/cgov_frame.bin";

/// Bytes of one thread's save area.
const SAVE: usize = 1088;

/// Word 3 once the probe wrote every word.
const DONE: u32 = 0x454e_5452;

/// The exit value the child stub passes.
const CHILD_EXIT: u32 = 0x5afe;

/// One thread's registers at its first instruction.
struct Entry {
    cr: u32,
    xer: u64,
    vrsave: u32,
    fpscr: u32,
    fpr: [u64; 32],
    vr: [u128; 32],
    vscr: u32,
}

fn parse(save: &[u8]) -> Entry {
    let d = |at: usize| u64::from_be_bytes(save[at..at + 8].try_into().expect("8 bytes"));
    let q = |at: usize| u128::from_be_bytes(save[at..at + 16].try_into().expect("16 bytes"));
    Entry {
        cr: d(256) as u32,
        xer: d(264),
        vrsave: d(288) as u32,
        fpscr: d(296) as u32,
        fpr: std::array::from_fn(|k| d(304 + 8 * k)),
        vr: std::array::from_fn(|k| q(560 + 16 * k)),
        vscr: q(1072) as u32,
    }
}

/// The main thread's and the child thread's entry registers.
fn console() -> [Entry; 2] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let p = &frame[8..];
    assert_eq!(p.len(), 16 + 2 * SAVE);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "no step failed");
    assert_eq!(word(8), CHILD_EXIT, "the child ran its stub");
    assert_eq!(word(12), DONE, "the probe wrote every word");
    [parse(&p[16..16 + SAVE]), parse(&p[16 + SAVE..])]
}

/// The state a new unit starts with, as the main thread and as a
/// created thread.
fn cellgov() -> [PpuState; 2] {
    [
        PpuState::new(),
        PpuExecutionUnit::new(UnitId::new(1)).state().clone(),
    ]
}

#[test]
fn a_new_thread_starts_with_the_console_xer_vrsave_fpscr_vrs_and_vscr() {
    for ((thread, console), s) in ["main", "child"].iter().zip(console()).zip(cellgov()) {
        assert_eq!(s.xer(), console.xer, "{thread} XER");
        assert_eq!(s.vrsave(), console.vrsave, "{thread} VRSAVE");
        assert_eq!(s.fpscr(), console.fpscr, "{thread} FPSCR");
        assert_eq!(*s.vr.as_array(), console.vr, "{thread} VRs");
        assert_eq!(s.vscr(), console.vscr, "{thread} VSCR");
    }
}

#[test]
fn a_created_thread_starts_with_the_console_cr() {
    let [_, child] = console();
    assert_eq!(cellgov()[1].cr(), child.cr);
}

#[test]
fn the_main_thread_starts_at_the_primary_entry_cr_and_a_new_unit_at_cr_0() {
    let [main, _] = console();
    assert_eq!(main.cr, cellgov_ps3_abi::lv2::process::PRIMARY_ENTRY_CR);
    assert_eq!(cellgov()[0].cr(), 0);
}

#[test]
fn the_fprs_hold_an_unspecified_leftover_on_the_console_and_the_declared_value_in_cellgov() {
    let [main, child] = console();
    for (thread, e) in [("main", &main), ("child", &child)] {
        assert!(
            e.fpr.iter().all(|&f| f == e.fpr[0] && f != 0),
            "{thread}: {:x?}",
            e.fpr
        );
    }
    assert_ne!(main.fpr[0], child.fpr[0]);
    for s in cellgov() {
        assert!(s
            .fpr
            .as_array()
            .iter()
            .all(|&f| f == cellgov_ps3_abi::hw::ppu::PPU_THREAD_ENTRY_FPR));
    }
}
