//! What a process sees when an instruction in one of its child threads
//! raises an interrupt, checked against the `ppu_exception_seen_*`
//! console captures.
//!
//! On the console, a trap, an illegal word, a privileged instruction
//! and a misaligned `lwarx` or `ldarx` each end the child thread at the
//! instruction. LV2 removes the thread: a get-priority call on it
//! answers `ESRCH`. The process and its main thread keep running. A
//! `twi` whose condition is false and a misaligned `lwz` complete, and
//! the join returns the child's exit value.

use cellgov_effects::FaultKind;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ppu::differential::{execute_state_in, CaseOutcome, PpuStateSnapshot};
use cellgov_ppu::exec::{ExecuteVerdict, PpuFault};
use cellgov_ppu::{PpuExecutionUnit, PpuFaultClass};
use cellgov_ps3_abi::lv2::errno::CELL_ESRCH;
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner;
use cellgov_time::Budget;

/// Where a unit run places its words: an address in the scenario's memory.
const TEXT: u64 = 0x1_0000;

/// The done marker, "EXCP".
const DONE: u32 = 0x4558_4350;
/// The value the probe writes for a join it did not make.
const SKIP: u32 = 0x5C1F_5C1F;
/// The value the probe's child passes to its thread exit.
const CHILD_EXIT: u32 = 0xE5;
/// The value the child presets in its destination register.
const PRESET: u64 = 0xDEAD_BEEF_DEAD_BEEF;

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

/// What the console showed for a probe's child thread.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    /// The child passed the instruction; its destination register then
    /// held the value.
    Completed(u64),
    /// The child reached the instruction, never passed it, and LV2
    /// removed it; the main thread finished.
    ThreadRemoved,
}

/// Classifies a probe's payload and asserts the words that each outcome fixes.
fn seen(probe: &str) -> Seen {
    let w = payload(probe);
    assert_eq!(w[0], DONE, "{probe}: the main thread finished");
    assert_eq!(w[2..4], [0, 0], "{probe}: the create and the start");
    assert_eq!(w[4], 1, "{probe}: the child reached the instruction");
    if w[5] == 1 {
        assert_eq!(w[8], 0, "{probe}: the child exists after it passed");
        assert_eq!(w[9..11], [0, CHILD_EXIT], "{probe}: the join");
        return Seen::Completed(u64::from(w[6]) << 32 | u64::from(w[7]));
    }
    assert_eq!(w[5], 0, "{probe}: the passed flag");
    assert_eq!(w[6..8], [0, 0], "{probe}: no destination recorded");
    assert_eq!(w[8], CELL_ESRCH.code, "{probe}: the child is gone");
    assert_eq!(w[9..11], [SKIP, SKIP], "{probe}: no join");
    Seen::ThreadRemoved
}

/// Runs `word` in CellGov with the child's registers:
///
/// - r9: the probe's access address, over a 64-byte window of 0x10, 0x11, ...
/// - r3: 0
/// - r0: the child's preset
fn cellgov(probe: &str, word: u32, offset: u64) -> Result<PpuStateSnapshot, Box<CaseOutcome>> {
    let ea = u64::from(payload(probe)[11]);
    let mut s = PpuStateSnapshot::zero();
    s.gpr[0] = PRESET;
    s.gpr[9] = ea;
    let window_at = if ea == 0 { 0 } else { ea - offset };
    execute_state_in(word, &s, window_at, (0x10..0x50).collect()).map(|(state, _)| state)
}

/// Runs `word` and then a process exit in a PPU unit at [`TEXT`]. The unit
/// fetches and decodes the words as it does for a title. Returns the class
/// of the guest fault that ends the run.
fn unit_fault_class(word: u32) -> Option<PpuFaultClass> {
    let fixture = ScenarioFixture::builder()
        .memory_size(0x4_0000)
        .budget(Budget::new(64))
        .max_steps(1_000)
        .seed_memory(move |mem| {
            let exit = [14 << 26 | 11 << 21 | sc::PROCESS_EXIT as u32, 0x4400_0002];
            for (i, w) in [word, exit[0], exit[1]].iter().enumerate() {
                let range = ByteRange::new(GuestAddr::new(TEXT + 4 * i as u64), 4).expect("range");
                mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
            }
        })
        .register(|rt| {
            rt.register_unit_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                unit.state_mut().pc = TEXT;
                unit
            });
        })
        .build();
    match runner::run(fixture).first_fault {
        Some((_, FaultKind::Guest(code))) => PpuFaultClass::of_code(code),
        _ => None,
    }
}

#[test]
fn a_child_thread_that_takes_a_program_interrupt_is_removed_and_cellgov_takes_the_same_class() {
    // (probe, the word the child runs, the class CellGov must take)
    for (probe, word, class) in [
        (
            "ppu_exception_seen_tw",
            0x7FE0_0008u32,
            PpuFaultClass::ProgramTrap,
        ),
        (
            "ppu_exception_seen_illegal",
            0x0400_0000,
            PpuFaultClass::ProgramIllegal,
        ),
        (
            "ppu_exception_seen_mtmsrd",
            0x7C60_0164,
            PpuFaultClass::ProgramPrivileged,
        ),
    ] {
        assert_eq!(seen(probe), Seen::ThreadRemoved, "{probe}");
        assert_eq!(unit_fault_class(word), Some(class), "{probe}");
    }
}

#[test]
fn a_child_thread_whose_lwarx_or_ldarx_is_misaligned_is_removed_and_cellgov_takes_the_alignment_fault(
) {
    // lwarx r0,0,r9 two bytes past a word; ldarx r0,0,r9 four bytes past
    // a doubleword.
    for (probe, word, offset) in [
        ("ppu_exception_seen_lwarx", 0x7C00_4828u32, 2u64),
        ("ppu_exception_seen_ldarx", 0x7C00_48A8, 4),
    ] {
        assert_eq!(seen(probe), Seen::ThreadRemoved, "{probe}");
        let ea = u64::from(payload(probe)[11]);
        assert_eq!(ea % 8, offset, "{probe}: the access address");
        let outcome = cellgov(probe, word, offset).map(|_| ()).map_err(|e| *e);
        assert_eq!(
            outcome,
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                PpuFault::alignment(ea, false)
            ))),
            "{probe}"
        );
    }
}

#[test]
fn a_false_twi_and_a_misaligned_lwz_complete_in_a_child_thread_and_in_cellgov() {
    // twi 4,r3,1 with r3 = 0 leaves r0 at the preset; lwz r0,0(r9) one
    // byte past a word loads the window's bytes there.
    for (probe, word, offset) in [
        ("ppu_exception_seen_twi_false", 0x0C83_0001u32, 0u64),
        ("ppu_exception_seen_lwz", 0x8009_0000, 1),
    ] {
        let Seen::Completed(dest) = seen(probe) else {
            panic!("{probe}: the console removed the thread");
        };
        let after = cellgov(probe, word, offset).unwrap_or_else(|e| panic!("{probe}: {e:?}"));
        assert_eq!(after.gpr[0], dest, "{probe}: r0");
    }
}
