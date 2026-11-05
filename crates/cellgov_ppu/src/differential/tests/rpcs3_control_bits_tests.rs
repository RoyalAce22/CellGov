//! Tests of the FPSCR and VSCR bits the runner compares for each
//! source.

use super::{run_case, CaseOutcome};
use crate::differential::{
    ControlRegisters, InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot,
};
use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_FPSCR_FI, PPC_FPSCR_FPCC, PPC_FPSCR_FPRF, PPC_FPSCR_FX, PPC_FPSCR_XX,
};
use cellgov_ps3_abi::hw::ppu::{VSCR_NJ, VSCR_SAT};

/// `ori 0, 0, 0`.
const NOP: u32 = 0x6000_0000;
/// `fadd f3, f1, f2`.
const FADD_F3_F1_F2: u32 = 0xFC61_102A;
/// 1.0.
const ONE: u64 = 0x3FF0_0000_0000_0000;
/// 2^-60, too small to change 1.0 in a double sum.
const TINY: u64 = 0x3C30_0000_0000_0000;
/// FPSCR FX, bit 0.
const FPSCR_FX: u32 = 0x8000_0000;
/// VSCR bit 23, reserved.
const VSCR_RESERVED_BIT: u32 = 0x0000_0100;

fn rpcs3() -> OracleSource {
    OracleSource::Rpcs3Capture {
        capture_id: "control-bits",
        control_registers: ControlRegisters::Recorded,
    }
}

fn case(
    raw: u32,
    initial: PpuStateSnapshot,
    expected: PpuStateSnapshot,
    source: OracleSource,
) -> InstructionCase {
    InstructionCase {
        label: "control-bits".to_string(),
        initial_state: initial,
        initial_memory: MemorySnapshot::empty(),
        raw_instruction: raw,
        expected_state: expected,
        expected_memory: MemorySnapshot::empty(),
        source,
    }
}

/// An inexact `fadd` whose expected FPSCR holds FPCC alone, as an RPCS3
/// capture records it: positive result, FG set.
fn inexact_fadd(source: OracleSource) -> CaseOutcome {
    let mut initial = PpuStateSnapshot::zero();
    initial.fpr[1] = ONE;
    initial.fpr[2] = TINY;
    let mut expected = initial.clone();
    expected.fpr[3] = ONE;
    expected.fpscr = 0x0000_4000;
    run_case(&case(FADD_F3_F1_F2, initial, expected, source))
}

#[test]
fn an_rpcs3_fadd_record_without_the_inexact_bits_passes() {
    assert_eq!(inexact_fadd(rpcs3()), CaseOutcome::Pass);
}

#[test]
fn a_spec_fadd_case_without_the_inexact_bits_is_a_mismatch() {
    let source = OracleSource::Spec {
        rationale: "full compare",
    };
    match inexact_fadd(source) {
        CaseOutcome::StateMismatch(diff) => {
            let (expected, observed) = diff.fpscr.expect("FPSCR differs");
            assert_eq!(expected, 0x0000_4000);
            assert_eq!(observed & PPC_FPSCR_FPCC, 0x0000_4000);
            assert_eq!(
                observed & !PPC_FPSCR_FPCC,
                PPC_FPSCR_FX | PPC_FPSCR_XX | PPC_FPSCR_FI
            );
        }
        other => panic!("expected StateMismatch, got {other:?}"),
    }
}

#[test]
fn an_rpcs3_record_whose_fpcc_differs_is_a_mismatch() {
    for bit in [0x0000_8000, 0x0000_4000, 0x0000_2000, 0x0000_1000] {
        let mut expected = PpuStateSnapshot::zero();
        expected.fpscr = FPSCR_FX | bit;
        match run_case(&case(NOP, PpuStateSnapshot::zero(), expected, rpcs3())) {
            CaseOutcome::StateMismatch(diff) => {
                assert_eq!(diff.fpscr, Some((FPSCR_FX | bit, 0)));
            }
            other => panic!("expected StateMismatch for 0x{bit:08x}, got {other:?}"),
        }
    }
}

#[test]
fn an_rpcs3_record_ignores_the_fprf_class_bit() {
    let mut expected = PpuStateSnapshot::zero();
    expected.fpscr = PPC_FPSCR_FPRF & !PPC_FPSCR_FPCC;
    let outcome = run_case(&case(NOP, PpuStateSnapshot::zero(), expected, rpcs3()));
    assert_eq!(outcome, CaseOutcome::Pass);
}

#[test]
fn an_rpcs3_record_ignores_vscr_bits_outside_nj_and_sat() {
    let mut expected = PpuStateSnapshot::zero();
    expected.vscr = VSCR_RESERVED_BIT;
    let outcome = run_case(&case(NOP, PpuStateSnapshot::zero(), expected, rpcs3()));
    assert_eq!(outcome, CaseOutcome::Pass);
}

#[test]
fn an_rpcs3_record_compares_nj_and_sat() {
    for bit in [VSCR_NJ, VSCR_SAT] {
        let mut expected = PpuStateSnapshot::zero();
        expected.vscr = bit;
        match run_case(&case(NOP, PpuStateSnapshot::zero(), expected, rpcs3())) {
            CaseOutcome::StateMismatch(diff) => assert_eq!(diff.vscr, Some((bit, 0))),
            other => panic!("expected StateMismatch for 0x{bit:08x}, got {other:?}"),
        }
    }
}

#[test]
fn every_other_source_compares_the_whole_fpscr_and_vscr() {
    let sources = [
        OracleSource::Spec {
            rationale: "full compare",
        },
        OracleSource::ConsoleCapture {
            capture_id: "control-bits",
        },
        OracleSource::Manual,
    ];
    for source in sources {
        let mut expected = PpuStateSnapshot::zero();
        expected.fpscr = FPSCR_FX;
        expected.vscr = VSCR_RESERVED_BIT;
        match run_case(&case(NOP, PpuStateSnapshot::zero(), expected, source)) {
            CaseOutcome::StateMismatch(diff) => {
                assert_eq!(diff.fpscr, Some((FPSCR_FX, 0)), "{source:?}");
                assert_eq!(diff.vscr, Some((VSCR_RESERVED_BIT, 0)), "{source:?}");
            }
            other => panic!("expected StateMismatch for {source:?}, got {other:?}"),
        }
    }
}
