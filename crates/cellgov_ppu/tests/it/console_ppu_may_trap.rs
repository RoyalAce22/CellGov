//! CellGov against the may-trap console captures: one instruction per
//! probe, each alone in its run because the PPE may end the process on
//! it.

use crate::console_cases::{capture, cellgov_after, table, Capture};
use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::CaseOutcome;
use cellgov_ppu::instruction::PpuInstruction;

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
fn ctrl_and_sprg3_reads_run_on_the_console_and_cellgov_does_not_decode_them_yet() {
    for (name, value) in [
        ("ppu_may_trap_mfspr136", 0x4040_0000u64),
        ("ppu_may_trap_mfspr259", 0),
    ] {
        let (cellgov, console, _) = probe(name);
        let Capture::Done(runs) = console else {
            panic!("{name}: the console ended the run");
        };
        assert_eq!(runs[0][0].slots[0], value, "{name}: run 1");
        assert_eq!(runs[0][1].slots[0], value, "{name}: run 2");
        assert!(
            matches!(cellgov, Err(CaseOutcome::DecodeError(_))),
            "{name} decodes now: compare it with the console: {cellgov:?}"
        );
    }
}
