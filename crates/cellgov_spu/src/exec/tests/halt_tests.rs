//! The six halt instructions stop the SPU when their condition holds and
//! continue when it does not.

use super::*;
use crate::decode::decode;
use crate::state::SpuState;
use crate::stop::SpuStopKind;

const HALT: SpuStepOutcome = SpuStepOutcome::Stop {
    kind: SpuStopKind::Halt,
    signal: 0,
};

/// An RR halt `op11` with RA = r1, RB = r2 and a false RT of r3.
fn rr(op11: u32) -> u32 {
    (op11 << 21) | (2 << 14) | (1 << 7) | 3
}

/// An RI10 halt `op8` with RA = r1, a false RT of r3 and `imm`.
fn ri10(op8: u32, imm: i16) -> u32 {
    (op8 << 24) | ((imm as u32 & 0x3FF) << 14) | (1 << 7) | 3
}

/// The outcome of `word` with r1 = `a` and r2 = `b`. RT, r3, must not
/// change.
fn run(word: u32, a: u32, b: u32) -> SpuStepOutcome {
    let insn = decode(word).expect("a halt decodes");
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, a);
    s.set_reg_word_splat(2, b);
    s.set_reg_word_splat(3, 0x5A5A_5A5A);
    let out = execute(&insn, &mut s, UnitId::new(0));
    assert_eq!(
        s.reg_word(3),
        0x5A5A_5A5A,
        "{insn:?} wrote its false target"
    );
    out
}

/// [CBEA p:94 s:8.5.2] H (bit 29) marks a stop by a halt instruction.
/// [CBEA p:93 s:8.5.2] the StopCode field is not valid without P.
#[test]
fn a_met_halt_stops_the_unit_with_the_halt_bit_and_the_next_word() {
    use cellgov_exec::{ExecutionContext, ExecutionUnit, StopRegisters, YieldReason};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    let mut unit = crate::SpuExecutionUnit::new(UnitId::new(1));
    unit.state_mut().ls[8..12].copy_from_slice(&rr(0x3D8).to_be_bytes());
    unit.state_mut().pc = 8;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(10), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(
        unit.state().stop.map(|stop| stop.kind),
        Some(SpuStopKind::Halt)
    );
    assert_eq!(
        unit.stop_registers(),
        Some(StopRegisters {
            status: 0x0000_0004,
            npc: 12
        })
    );
}

/// [SPU-ISA p:150 s:7] heq halts when RA's preferred word equals RB's.
#[test]
fn heq_halts_only_on_equal_words() {
    assert_eq!(run(rr(0x3D8), 5, 5), HALT);
    assert_eq!(run(rr(0x3D8), 5, 6), SpuStepOutcome::Continue);
}

/// [SPU-ISA p:150 s:7] heq compares the preferred slots only, RA0:3 and RB0:3.
#[test]
fn a_halt_reads_only_the_preferred_slot() {
    let heq = decode(rr(0x3D8)).expect("heq decodes");
    for (preferred_equal, expected) in [(true, HALT), (false, SpuStepOutcome::Continue)] {
        let mut s = SpuState::new();
        for slot in 1..4 {
            s.set_reg_word_slot(1, slot, 1);
            s.set_reg_word_slot(2, slot, 2);
        }
        s.set_reg_word_slot(1, 0, 7);
        s.set_reg_word_slot(2, 0, if preferred_equal { 7 } else { 8 });
        assert_eq!(execute(&heq, &mut s, UnitId::new(0)), expected);
    }
}

/// [SPU-ISA p:151 s:7] heqi compares against I10 sign-extended to 32 bits.
#[test]
fn heqi_compares_against_the_sign_extended_immediate() {
    assert_eq!(run(ri10(0x7F, -1), 0xFFFF_FFFF, 0), HALT);
    assert_eq!(run(ri10(0x7F, -1), 0x3FF, 0), SpuStepOutcome::Continue);
}

/// [SPU-ISA p:152 s:7] hgt is an algebraic compare.
#[test]
fn hgt_compares_signed() {
    assert_eq!(run(rr(0x258), 1, 0x8000_0000), HALT);
    assert_eq!(run(rr(0x258), 0x8000_0000, 1), SpuStepOutcome::Continue);
    assert_eq!(run(rr(0x258), 7, 7), SpuStepOutcome::Continue);
}

/// [SPU-ISA p:153 s:7] hgti compares algebraically against the sign-extended I10.
#[test]
fn hgti_compares_signed_against_the_immediate() {
    assert_eq!(run(ri10(0x4F, -1), 0, 0), HALT);
    assert_eq!(
        run(ri10(0x4F, -1), 0x8000_0000, 0),
        SpuStepOutcome::Continue
    );
}

/// [SPU-ISA p:154 s:7] hlgt is a logical (unsigned) compare.
#[test]
fn hlgt_compares_unsigned() {
    assert_eq!(run(rr(0x2D8), 0x8000_0000, 1), HALT);
    assert_eq!(run(rr(0x2D8), 1, 0x8000_0000), SpuStepOutcome::Continue);
    assert_eq!(run(rr(0x2D8), 7, 7), SpuStepOutcome::Continue);
}

/// [SPU-ISA p:155 s:7] hlgti extends I10 to 32 bits, then compares unsigned.
#[test]
fn hlgti_compares_unsigned_against_the_sign_extended_immediate() {
    assert_eq!(run(ri10(0x5F, -512), 0xFFFF_FFFF, 0), HALT);
    assert_eq!(run(ri10(0x5F, -512), 1, 0), SpuStepOutcome::Continue);
    assert_eq!(
        run(ri10(0x5F, -512), 0x8000_0000, 0),
        SpuStepOutcome::Continue
    );
}
