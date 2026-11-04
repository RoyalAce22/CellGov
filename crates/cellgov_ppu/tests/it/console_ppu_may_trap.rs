//! CellGov against the may-trap console captures: one instruction per
//! probe under test, each alone in its run because the PPE may end the
//! process on it.

use crate::console_cases::{capture, cellgov_after, table, Capture, Case};
use cellgov_event::UnitId;
use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::exec::{execute, ExecuteVerdict, PpuFault};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;
use cellgov_ppu::store_buffer::StoreBuffer;

/// The probe's one case, CellGov's answer for it, and the capture.
fn probe(name: &str) -> (Result<(), CaseOutcome>, Capture, u32) {
    let cases = table(name);
    assert_eq!(cases.len(), 1, "{name}: one case");
    let case = &cases[0];
    assert!(case.may_end_process, "{name}: a may-trap case");
    let ran = cellgov_after(case, 0).map(|_| ()).map_err(|e| *e);
    (ran, capture(name, 1), case.words[0])
}

#[test]
fn popcntb_and_mcrxr_end_the_process_on_the_console_and_refuse_in_cellgov() {
    for name in ["ppu_may_trap_popcntb", "ppu_may_trap_mcrxr"] {
        let (cellgov, console, _) = probe(name);
        assert!(
            matches!(console, Capture::Ended { case: 0 }),
            "{name}: the console ran it: {console:?}"
        );
        assert!(
            matches!(
                cellgov,
                Err(CaseOutcome::DecodeError(_) | CaseOutcome::UnexpectedVerdict(_))
            ),
            "{name}: CellGov runs it: {cellgov:?}"
        );
    }
}

/// The state after the case's words run from `BASE`. Each branch moves
/// the PC, and the run stops when the PC leaves the words.
fn run_with_branches(case: &Case) -> PpuState {
    const BASE: u64 = 0x1_0000;
    let mut s = PpuState::new();
    case.initial(0, 0).apply(&mut s);
    s.pc = BASE;
    let end = BASE + 4 * case.words.len() as u64;
    for _ in 0..4 * case.words.len() {
        if !(BASE..end).contains(&s.pc) {
            return s;
        }
        let word = case.words[((s.pc - BASE) / 4) as usize];
        let insn = decode(word).unwrap_or_else(|e| panic!("{}: {e}", case.id));
        let verdict = execute(
            &insn,
            &mut s,
            UnitId::new(0),
            &[],
            &mut Vec::new(),
            &mut StoreBuffer::new(),
        );
        match verdict {
            ExecuteVerdict::Continue => s.pc += 4,
            ExecuteVerdict::Branch => {}
            other => panic!("{}: {other:?} at {:#x}", case.id, s.pc),
        }
    }
    panic!("{}: the words did not run to their end", case.id)
}

#[test]
fn bcctr_with_bo_2_clear_decrements_ctr_and_branches_to_the_old_ctr_on_both() {
    // (probe, r3 after, or None for the r3 input): li r3,1 runs only when
    // bcctr falls through. The _taken and _ctr probes record CTR in r5
    // once bcctr has run.
    for (name, r3) in [
        ("ppu_may_trap_bcctr_bo2", None),
        ("ppu_may_trap_bcctr_bo2_taken", None),
        ("ppu_may_trap_bcctr_bo2_ctr", Some(1)),
    ] {
        let cases = table(name);
        let r3 = r3.unwrap_or(cases[0].initial(0, 0).gpr[3]);
        let Capture::Done(runs) = capture(name, 1) else {
            panic!("{name}: the console ended the run");
        };
        let cellgov = run_with_branches(&cases[0]);
        for run in &runs[0] {
            assert_eq!(run.slots[0], r3, "{name}: console r3");
        }
        assert_eq!(cellgov.gpr[3], r3, "{name}: CellGov r3");
        if cases[0].words.len() > 1 {
            // CTR held the word-aligned address of mfctr, so a decrement
            // by one leaves its low two bits set.
            for run in &runs[0] {
                assert_eq!(run.slots[1] & 3, 3, "{name}: console r5");
            }
            assert_eq!(cellgov.gpr[5] & 3, 3, "{name}: CellGov r5");
        }
    }
}

#[test]
fn a_load_string_with_an_address_register_in_range_ends_the_process_on_the_console_and_refuses_in_cellgov(
) {
    for (name, insn) in [
        ("ppu_may_trap_lswi_ra", "lswi"),
        ("ppu_may_trap_lswx_ra", "lswx"),
        ("ppu_may_trap_lswx_rb", "lswx"),
        ("ppu_may_trap_lswx_rt_ra_n0", "lswx"),
    ] {
        let (cellgov, console, _) = probe(name);
        assert!(
            matches!(console, Capture::Ended { case: 0 }),
            "{name}: the console ran it: {console:?}"
        );
        assert_eq!(
            cellgov,
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(
                PpuFault::InvalidForm(insn)
            ))),
            "{name}"
        );
    }
}

#[test]
fn privileged_and_reserved_reads_end_the_process_on_the_console_and_refuse_in_cellgov() {
    for (name, fault) in [
        (
            "ppu_may_trap_mfspr272",
            PpuFault::PrivilegedInstruction("mfspr"),
        ),
        (
            "ppu_may_trap_mfspr287",
            PpuFault::PrivilegedInstruction("mfspr"),
        ),
        (
            "ppu_may_trap_mfspr1022",
            PpuFault::PrivilegedInstruction("mfspr"),
        ),
        (
            "ppu_may_trap_mfmsr",
            PpuFault::PrivilegedInstruction("mfmsr"),
        ),
        // The figures list SPR 284 (TBL) and SPR 152 (CTRL) for mtspr only.
        (
            "ppu_may_trap_mfspr284",
            PpuFault::PrivilegedInstruction("mfspr"),
        ),
        (
            "ppu_may_trap_mfspr152",
            PpuFault::PrivilegedInstruction("mfspr"),
        ),
        (
            "ppu_may_trap_slbmfee",
            PpuFault::PrivilegedInstruction("slbmfee"),
        ),
        (
            "ppu_may_trap_slbmfev",
            PpuFault::PrivilegedInstruction("slbmfev"),
        ),
        (
            "ppu_may_trap_mfspr280",
            PpuFault::IllegalInstruction("mfspr"),
        ),
    ] {
        let (cellgov, console, _) = probe(name);
        assert!(
            matches!(console, Capture::Ended { case: 0 }),
            "{name}: the console ran it: {console:?}"
        );
        assert_eq!(
            cellgov,
            Err(CaseOutcome::UnexpectedVerdict(ExecuteVerdict::Fault(fault))),
            "{name}"
        );
    }
}

#[test]
fn lswx_with_ra_0_outside_the_range_loads_on_the_console_and_in_cellgov() {
    let name = "ppu_may_trap_lswx_ra0";
    let (_, console, _) = probe(name);
    let Capture::Done(runs) = console else {
        panic!("{name}: the console ended the run");
    };
    let cellgov = cellgov_after(&table(name)[0], 0).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    for run in &runs[0] {
        assert_eq!(run.slots[0], cellgov.gpr[4], "{name}: r4");
        assert_eq!(run.slots[1], cellgov.gpr[5], "{name}: r5");
    }
}

#[test]
fn mfspr_268_and_269_run_on_the_console_and_read_the_time_base_in_cellgov() {
    for (name, upper) in [
        ("ppu_may_trap_mfspr268", false),
        ("ppu_may_trap_mfspr269", true),
    ] {
        let (cellgov, console, word) = probe(name);
        let Capture::Done(runs) = console else {
            panic!("{name}: the console ended the run");
        };
        assert!(cellgov.is_ok(), "{name}: {cellgov:?}");
        let insn = decode(word).unwrap_or_else(|e| panic!("{name}: {e}"));
        let read_time_base = if upper {
            matches!(insn, PpuInstruction::Mftbu { rt: 5 })
        } else {
            matches!(insn, PpuInstruction::Mftb { rt: 5 })
        };
        assert!(read_time_base, "{name}: {insn:?}");
        // Two reads of a running time base, the second no lower.
        let [a, b] = runs[0].map(|r| r.slots[0]);
        assert!(a > 0 && b >= a, "{name}: {a:#x} then {b:#x}");
        if upper {
            assert_eq!(a >> 32, 0, "{name}: TBU is a 32-bit value");
        }
    }
}

#[test]
fn ctrl_and_sprg3_reads_match_the_console() {
    for (name, read) in [
        ("ppu_may_trap_mfspr136", PpuInstruction::Mfctrl { rt: 5 }),
        ("ppu_may_trap_mfspr259", PpuInstruction::Mfsprg3 { rt: 5 }),
    ] {
        let (_, console, word) = probe(name);
        let Capture::Done(runs) = console else {
            panic!("{name}: the console ended the run");
        };
        assert_eq!(decode(word), Ok(read), "{name}");
        let cellgov = cellgov_after(&table(name)[0], 0).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for run in &runs[0] {
            assert_eq!(run.slots[0], cellgov.gpr[5], "{name}");
        }
    }
}
