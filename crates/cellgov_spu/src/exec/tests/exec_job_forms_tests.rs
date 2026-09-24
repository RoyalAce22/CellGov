//! Execution of the forms a compiler-built job's crt0 and main use.

use crate::exec::{execute, SpuFault, SpuStepOutcome};
use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::spu;

const PATTERN: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
];

fn uid() -> UnitId {
    UnitId::new(1)
}

fn identity_mask() -> [u8; 16] {
    let mut m = [0u8; 16];
    for (i, b) in m.iter_mut().enumerate() {
        *b = 0x10 + i as u8;
    }
    m
}

fn run(insn: SpuInstruction, s: &mut SpuState) -> SpuStepOutcome {
    execute(&insn, s, uid())
}

// [SPU-ISA p:41 s:3. Memory-Load/Store Instructions] cbx: the byte at RA + RB holds selector 0x03.
#[test]
fn cbx_marks_the_addressed_byte() {
    let mut s = SpuState::new();
    // 0x1011 + 0x0002 = 0x1013 -> byte 3. Both operands carry low bits,
    // so dropping either one moves the marked byte.
    s.set_reg_word_splat(9, 0x1011);
    s.set_reg_word_splat(10, 0x0002);
    assert!(matches!(
        run(
            SpuInstruction::Cbx {
                rt: 16,
                ra: 9,
                rb: 10
            },
            &mut s
        ),
        SpuStepOutcome::Continue
    ));
    let mut want = identity_mask();
    want[3] = 0x03;
    assert_eq!(s.regs[16], want);
}

// [SPU-ISA p:42 s:3. Memory-Load/Store Instructions] chd: the aligned halfword at RA + I7 holds 0x02 0x03.
#[test]
fn chd_marks_the_aligned_halfword() {
    let mut s = SpuState::new();
    // 0x1003 + 8 = 0x100B -> halfword 10 once the low bit is forced off.
    // The odd sum fails an implementation that keeps that bit.
    s.set_reg_word_splat(1, 0x1003);
    run(
        SpuInstruction::Chd {
            rt: 6,
            ra: 1,
            imm: 8,
        },
        &mut s,
    );
    let mut want = identity_mask();
    want[10] = 0x02;
    want[11] = 0x03;
    assert_eq!(s.regs[6], want);
}

// [SPU-ISA p:43 s:3. Memory-Load/Store Instructions] chx: the same mask from RA + RB.
#[test]
fn chx_marks_the_aligned_halfword() {
    let mut s = SpuState::new();
    // 0x1005 + 0x0006 = 0x100B -> halfword 10, same as the d-form.
    s.set_reg_word_splat(4, 0x1005);
    s.set_reg_word_splat(5, 0x0006);
    run(
        SpuInstruction::Chx {
            rt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
    );
    let mut want = identity_mask();
    want[10] = 0x02;
    want[11] = 0x03;
    assert_eq!(s.regs[7], want);
}

// [SPU-ISA p:45 s:3. Memory-Load/Store Instructions] cwx: the aligned word at RA + RB holds 0x00..0x03.
#[test]
fn cwx_marks_the_aligned_word() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(4, 0x0004);
    s.set_reg_word_splat(5, 0x0009);
    run(
        SpuInstruction::Cwx {
            rt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
    );
    let mut want = identity_mask();
    want[12..16].copy_from_slice(&[0x00, 0x01, 0x02, 0x03]);
    assert_eq!(s.regs[7], want);
}

// [SPU-ISA p:46 s:3. Memory-Load/Store Instructions] cdd: the aligned doubleword at RA + I7 holds 0x00..0x07.
#[test]
fn cdd_marks_the_aligned_doubleword() {
    let mut s = SpuState::new();
    // 0x000F + 8 = 0x17 -> doubleword 0. Either operand alone selects
    // doubleword 8, and the low three bits must be forced off.
    s.set_reg_word_splat(1, 0x000F);
    run(
        SpuInstruction::Cdd {
            rt: 6,
            ra: 1,
            imm: 8,
        },
        &mut s,
    );
    let mut want = identity_mask();
    want[0..8].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(s.regs[6], want);
}

// [SPU-ISA p:47 s:3. Memory-Load/Store Instructions] cdx: the same mask from RA + RB.
#[test]
fn cdx_marks_the_upper_doubleword() {
    let mut s = SpuState::new();
    // 0x0005 + 0x0005 = 0x0A -> doubleword 8; neither operand reaches
    // bit 3 alone.
    s.set_reg_word_splat(4, 0x0005);
    s.set_reg_word_splat(5, 0x0005);
    run(
        SpuInstruction::Cdx {
            rt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
    );
    let mut want = identity_mask();
    want[8..16].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(s.regs[7], want);
}

// [SPU-ISA p:141 s:6. Shift and Rotate Instructions] rotqmbyi: shift right by (0 - I7) mod 32 bytes; 16 or more clears.
#[test]
fn rotqmbyi_shifts_right_by_the_negated_count_and_zero_fills() {
    let mut s = SpuState::new();
    // Every source byte is non-zero, so a byte that survives the shift
    // is distinguishable from the zero fill.
    let src: [u8; 16] = [
        0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE,
        0xAF,
    ];
    s.regs[4] = src;
    // (0 - 0x72) mod 32 = 14 bytes right.
    run(
        SpuInstruction::Rotqmbyi {
            rt: 5,
            ra: 4,
            imm: 0x72,
        },
        &mut s,
    );
    let mut want = [0u8; 16];
    want[14] = 0xA0;
    want[15] = 0xA1;
    assert_eq!(s.regs[5], want);
    run(
        SpuInstruction::Rotqmbyi {
            rt: 5,
            ra: 4,
            imm: 0x70,
        },
        &mut s,
    );
    assert_eq!(s.regs[5], [0u8; 16]);
    run(
        SpuInstruction::Rotqmbyi {
            rt: 5,
            ra: 4,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.regs[5], src);
}

// [SPU-ISA p:157 s:7. Compare, Branch, and Halt Instructions] ceqbi: all ones per byte that equals the immediate.
#[test]
fn ceqbi_marks_each_matching_byte() {
    let mut s = SpuState::new();
    s.regs[2] = [
        0x22, 0x00, 0x22, 0xFF, 0x22, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0x22,
    ];
    run(
        SpuInstruction::Ceqbi {
            rt: 3,
            ra: 2,
            imm: 0x22,
        },
        &mut s,
    );
    assert_eq!(
        s.regs[3],
        [0xFF, 0, 0xFF, 0, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF]
    );
}

// [SPU-ISA p:185 s:7. Compare, Branch, and Halt Instructions] brhz: taken on a zero low halfword.
#[test]
fn brhz_branches_only_when_the_low_halfword_is_zero() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(2, 0x1234_0000);
    assert!(matches!(
        run(SpuInstruction::Brhz { rt: 2, offset: 5 }, &mut s),
        SpuStepOutcome::Branch
    ));
    assert_eq!(s.pc, 0x114);
    s.set_reg_word_splat(2, 0x0000_0001);
    assert!(matches!(
        run(SpuInstruction::Brhz { rt: 2, offset: 5 }, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_eq!(s.pc, 0x114);
}

// [SPU-ISA p:186 s:7. Compare, Branch, and Halt Instructions] biz: PC <- RA masked when RT's preferred word is zero.
#[test]
fn biz_and_binz_test_the_preferred_word() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(0, 0x3ffc3);
    s.set_reg_word_splat(78, 0);
    assert!(matches!(
        run(SpuInstruction::Biz { rt: 78, ra: 0 }, &mut s),
        SpuStepOutcome::Branch
    ));
    assert_eq!(s.pc, 0x3ffc0);
    s.pc = 0x100;
    assert!(matches!(
        run(SpuInstruction::Binz { rt: 78, ra: 0 }, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_eq!(s.pc, 0x100);
    s.set_reg_word_splat(78, 1);
    assert!(matches!(
        run(SpuInstruction::Binz { rt: 78, ra: 0 }, &mut s),
        SpuStepOutcome::Branch
    ));
    assert_eq!(s.pc, 0x3ffc0);
}

// [SPU-ISA p:188 s:7. Compare, Branch, and Halt Instructions] bihz / bihnz: the low halfword decides, the high halfword does not.
#[test]
fn bihz_and_bihnz_test_only_the_low_halfword() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(0, 0x200);
    s.set_reg_word_splat(3, 0x0001_0000);
    assert!(matches!(
        run(SpuInstruction::Bihz { rt: 3, ra: 0 }, &mut s),
        SpuStepOutcome::Branch
    ));
    assert_eq!(s.pc, 0x200);
    s.pc = 0x100;
    assert!(matches!(
        run(SpuInstruction::Bihnz { rt: 3, ra: 0 }, &mut s),
        SpuStepOutcome::Continue
    ));
    s.set_reg_word_splat(3, 0x0000_0001);
    assert!(matches!(
        run(SpuInstruction::Bihnz { rt: 3, ra: 0 }, &mut s),
        SpuStepOutcome::Branch
    ));
    assert_eq!(s.pc, 0x200);
}

// [SPU-ISA p:90 s:5. Integer and Logical Instructions] gb: word 0's low bit is the leftmost of the nibble.
#[test]
fn gb_gathers_word_low_bits_word_zero_leftmost() {
    let mut s = SpuState::new();
    s.regs[3] = [0xAA; 16];
    for (slot, v) in [1u32, 0, 1, 1].into_iter().enumerate() {
        s.set_reg_word_slot(5, slot, 0xFFFF_FFFE | v);
    }
    run(SpuInstruction::Gb { rt: 3, ra: 5 }, &mut s);
    let mut want = [0u8; 16];
    want[3] = 0b1011;
    assert_eq!(s.regs[3], want);
}

// [SPU-ISA p:89 s:5. Integer and Logical Instructions] gbh: halfword 0's low bit is the leftmost of the byte.
#[test]
fn gbh_gathers_halfword_low_bits_halfword_zero_leftmost() {
    let mut s = SpuState::new();
    s.regs[3] = [0xAA; 16];
    s.regs[5] = [0xFF; 16];
    // Halfword low bits 1,0,0,0,0,0,1,1: not a palindrome, so a
    // reversed gather order reads 0b1100_0001 and fails.
    for hw in 1..6 {
        s.regs[5][hw * 2 + 1] = 0xFE;
    }
    run(SpuInstruction::Gbh { rt: 3, ra: 5 }, &mut s);
    let mut want = [0u8; 16];
    want[3] = 0b1000_0011;
    assert_eq!(s.regs[3], want);
}

// [SPU-ISA p:249 s:11. Channel Instructions] rchcnt: the capacity in the preferred slot, other slots zero.
#[test]
fn rchcnt_answers_one_for_machine_status() {
    let mut s = SpuState::new();
    s.regs[2] = [0xAA; 16];
    assert!(matches!(
        run(
            SpuInstruction::Rchcnt {
                rt: 2,
                channel: spu::SPU_RD_MACH_STAT
            },
            &mut s
        ),
        SpuStepOutcome::Continue
    ));
    let mut want = [0u8; 16];
    want[3] = 1;
    assert_eq!(s.regs[2], want);
}

#[test]
fn rchcnt_on_an_unmodeled_channel_faults_by_name() {
    let mut s = SpuState::new();
    s.regs[2] = PATTERN;
    assert!(matches!(
        run(
            SpuInstruction::Rchcnt {
                rt: 2,
                channel: spu::SPU_RD_IN_MBOX
            },
            &mut s
        ),
        SpuStepOutcome::Fault(SpuFault::UnsupportedChannelCount(29))
    ));
    assert_eq!(s.regs[2], PATTERN);
}

#[test]
fn a_refused_rchcnt_faults_in_its_own_class_not_the_rdch_one() {
    use crate::fault_codes::{FAULT_UNSUPPORTED_CHANNEL, FAULT_UNSUPPORTED_CHANNEL_COUNT};
    use crate::SpuExecutionUnit;
    use cellgov_effects::FaultKind;
    use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    // rchcnt $2, $8: the decrementer count is not modeled.
    let raw: u32 = (0x00Fu32 << 21) | (8u32 << 7) | 2;
    let mut unit = SpuExecutionUnit::new(uid());
    unit.state_mut().ls[0..4].copy_from_slice(&raw.to_be_bytes());
    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_UNSUPPORTED_CHANNEL_COUNT | 8))
    );
    assert_ne!(
        result.fault,
        Some(FaultKind::Guest(FAULT_UNSUPPORTED_CHANNEL | 8))
    );
}

// [CBEA p:141 s:9.8 SPU Read Machine Status Channel] Both status bits read as zero in this model.
#[test]
fn rdch_machine_status_reads_zero() {
    let mut s = SpuState::new();
    s.regs[78] = PATTERN;
    assert!(matches!(
        run(
            SpuInstruction::Rdch {
                rt: 78,
                channel: spu::SPU_RD_MACH_STAT
            },
            &mut s
        ),
        SpuStepOutcome::Continue
    ));
    assert_eq!(s.regs[78], [0u8; 16]);
}

#[test]
fn hbra_and_dsync_change_no_state() {
    let mut s = SpuState::new();
    s.regs[2] = PATTERN;
    let before = s.clone();
    assert!(matches!(
        run(SpuInstruction::Hbra, &mut s),
        SpuStepOutcome::Continue
    ));
    assert!(matches!(
        run(SpuInstruction::Dsync, &mut s),
        SpuStepOutcome::Continue
    ));
    assert_eq!(s.regs, before.regs);
    assert_eq!(s.pc, before.pc);
}
