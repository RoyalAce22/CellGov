use super::*;
use cellgov_event::UnitId;

fn uid() -> UnitId {
    UnitId::new(0)
}

fn exec_no_mem(insn: &PpuInstruction, s: &mut PpuState) -> ExecuteVerdict {
    let mut effects = Vec::new();
    let mut store_buf = crate::store_buffer::StoreBuffer::new();
    execute(insn, s, uid(), &[], &mut effects, &mut store_buf)
}

/// Flushes the store buffer into `effects` after execution.
fn exec_with_mem(
    insn: &PpuInstruction,
    s: &mut PpuState,
    base: u64,
    mem: &[u8],
    effects: &mut Vec<Effect>,
) -> ExecuteVerdict {
    let views: [(u64, &[u8]); 1] = [(base, mem)];
    let mut store_buf = crate::store_buffer::StoreBuffer::new();
    let v = execute(insn, s, uid(), &views, effects, &mut store_buf);
    store_buf.flush(effects, uid());
    v
}

#[test]
fn addi_with_ra_zero_is_li() {
    let mut s = PpuState::new();
    exec_no_mem(
        &PpuInstruction::Addi {
            rt: 3,
            ra: 0,
            imm: 42,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 42);
}

#[test]
fn addi_with_ra_nonzero_adds() {
    let mut s = PpuState::new();
    s.gpr[5] = 100;
    exec_no_mem(
        &PpuInstruction::Addi {
            rt: 3,
            ra: 5,
            imm: -10,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 90);
}

#[test]
fn addis_shifts_left_16() {
    let mut s = PpuState::new();
    exec_no_mem(
        &PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x10000);
}

#[test]
fn ori_zero_is_move() {
    let mut s = PpuState::new();
    s.gpr[5] = 0xCAFE;
    exec_no_mem(
        &PpuInstruction::Ori {
            ra: 3,
            rs: 5,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xCAFE);
}

#[test]
fn cmpwi_sets_cr_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 10,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010); // EQ
}

#[test]
fn branch_unconditional() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    let result = exec_no_mem(
        &PpuInstruction::B {
            offset: -8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x0FF8);
}

#[test]
fn bl_sets_lr() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    exec_no_mem(
        &PpuInstruction::B {
            offset: 0x100,
            aa: false,
            link: true,
        },
        &mut s,
    );
    assert_eq!(s.lr, 0x1004);
    assert_eq!(s.pc, 0x1100);
}

#[test]
fn ba_branches_to_absolute_address() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    let result = exec_no_mem(
        &PpuInstruction::B {
            offset: 0x100,
            aa: true,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(
        s.pc, 0x100,
        "aa=true: target is offset itself, not PC+offset"
    );
}

#[test]
fn bla_sets_lr_and_branches_absolute() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    exec_no_mem(
        &PpuInstruction::B {
            offset: 0x400,
            aa: true,
            link: true,
        },
        &mut s,
    );
    assert_eq!(s.lr, 0x2004);
    assert_eq!(s.pc, 0x400);
}

#[test]
fn blr_returns_to_lr() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.lr = 0x1000;
    // BO=0x14: always taken, CTR not decremented.
    let result = exec_no_mem(
        &PpuInstruction::Bclr {
            bo: 0x14,
            bi: 0,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn mflr_mtlr_roundtrip() {
    let mut s = PpuState::new();
    s.gpr[5] = 0xABCD;
    exec_no_mem(&PpuInstruction::Mtlr { rs: 5 }, &mut s);
    assert_eq!(s.lr, 0xABCD);
    exec_no_mem(&PpuInstruction::Mflr { rt: 3 }, &mut s);
    assert_eq!(s.gpr[3], 0xABCD);
}

#[test]
fn rlwinm_slwi() {
    let mut s = PpuState::new();
    s.gpr[5] = 0x0001;
    // slwi r3, r5, 16 == rlwinm r3, r5, 16, 0, 15
    exec_no_mem(
        &PpuInstruction::Rlwinm {
            ra: 3,
            rs: 5,
            sh: 16,
            mb: 0,
            me: 15,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x10000);
}

#[test]
fn rlwinm_mask_contiguous() {
    assert_eq!(rlwinm_mask(0, 31), 0xFFFFFFFF);
    assert_eq!(rlwinm_mask(16, 31), 0x0000FFFF);
    assert_eq!(rlwinm_mask(0, 15), 0xFFFF0000);
}

#[test]
fn rlwinm_mask_wrapped() {
    // mb > me: mask wraps around; here bits [0..3] and [28..31].
    assert_eq!(rlwinm_mask(28, 3), 0xF000000F);
}

#[test]
fn ldu_writes_ea_back_to_ra() {
    let mut mem = vec![0u8; 0x1028];
    mem[0x1018..0x1020].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x1020;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Ldu {
            rt: 7,
            ra: 4,
            imm: -8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[7], 0xDEAD_BEEF_CAFE_BABE);
    assert_eq!(s.gpr[4], 0x1018);
}

#[test]
fn rlwnm_rotates_by_rb_low_5_bits() {
    let mut s = PpuState::new();
    s.gpr[0] = 0x0000_0000_1234_5678;
    s.gpr[8] = 8;
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 0,
            rs: 0,
            rb: 8,
            mb: 0,
            me: 31,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[0], 0x3456_7812);
}

#[test]
fn rlwnm_ignores_high_bits_of_rb() {
    // 0x20 == 32: only low 5 bits feed the rotate, so rotation == 0.
    let mut s = PpuState::new();
    s.gpr[1] = 0x0000_0000_DEAD_BEEF;
    s.gpr[2] = 0x20;
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 3,
            rs: 1,
            rb: 2,
            mb: 0,
            me: 31,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn vxor_self_zeros_vector_register() {
    let mut s = PpuState::new();
    s.vr[5] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
    exec_no_mem(
        &PpuInstruction::Vxor {
            vt: 5,
            va: 5,
            vb: 5,
        },
        &mut s,
    );
    assert_eq!(s.vr[5], 0);
}

#[test]
fn stvx_aligns_ea_and_emits_store_effect() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[8] = 0x1F;
    s.vr[0] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stvx {
            vs: 0,
            ra: 1,
            rb: 8,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            // stvx forces EA to 16-byte alignment: 0x1000+0x1F -> 0x1010.
            assert_eq!(range.start().raw(), 0x1010);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn extsw_sign_extends_low_32_bits() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_8000_0000;
    exec_no_mem(
        &PpuInstruction::Extsw {
            ra: 4,
            rs: 3,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
}

#[test]
fn sc_returns_syscall() {
    let mut s = PpuState::new();
    let result = exec_no_mem(&PpuInstruction::Sc { lev: 0 }, &mut s);
    assert!(matches!(result, ExecuteVerdict::Syscall));
}

#[test]
fn lwz_loads_from_memory() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1008..0x100C].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn lwz_mem_fault_on_bad_address() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let result = exec_no_mem(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s,
    );
    assert_eq!(result, ExecuteVerdict::MemFault(0x1008));
}

#[test]
fn lha_sign_extends_halfword() {
    // 0xFF80 == -128 as i16; lha sign-extends to the full GPR width.
    let mut mem = vec![0u8; 0x2000];
    mem[0x1002..0x1004].copy_from_slice(&0xFF80u16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lha {
            rt: 3,
            ra: 1,
            imm: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3] as i64, -128);
}

#[test]
fn stw_emits_store_effect() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[5] = 0xDEADBEEF;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x1000);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEFu32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn bc_beq_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    // BO=0x0C: branch on CR true, no CTR decrement. BI=2: EQ bit of cr0.
    s.set_cr_field(0, 0b0010);
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1008);
}

#[test]
fn bc_beq_not_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.set_cr_field(0, 0b0100); // GT
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn divdu_basic() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 7;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 14);
}

#[test]
fn divdu_divide_by_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn divdu_large_values() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0x7FFF_FFFF_FFFF_FFFF);
}

#[test]
fn divd_signed() {
    let mut s = PpuState::new();
    s.gpr[3] = (-100i64) as u64;
    s.gpr[4] = 7;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i64, -14);
}

#[test]
fn divd_divide_by_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn mulld_basic() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulld {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 56);
}

#[test]
fn mulld_wraps_on_overflow() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulld {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    // -1 * 2 = -2 (wrapping) = 0xFFFF_FFFF_FFFF_FFFE
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFE);
}

#[test]
fn adde_adds_with_carry_in_and_sets_carry_out() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert!(s.xer_ca());
}

#[test]
fn adde_without_carry_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 5;
    s.gpr[4] = 3;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 8);
    assert!(!s.xer_ca());
}

#[test]
fn mulhdu_takes_high_64_bits_of_u128_product() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 1);
}

#[test]
fn mulhdu_small_product_is_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn ldarx_loads_from_memory() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1008..0x1010].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[5], 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn stdcx_with_matching_reservation_emits_conditional_store() {
    let mut s = PpuState::new();
    // Pre-seed the reservation the way a prior ldarx at this line would.
    s.reservation = Some(cellgov_sync::ReservedLine::containing(0x1008));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b0010);
    assert!(s.reservation.is_none());
    // stdcx must emit ConditionalStore, never a SharedWriteIntent.
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::ConditionalStore { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x1008);
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        }
        other => panic!("expected ConditionalStore, got {other:?}"),
    }
}

#[test]
fn stdcx_without_reservation_fails_silently() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert!(effects.is_empty());
}

#[test]
fn stwcx_with_reservation_on_different_line_fails() {
    // 128-byte reservation granule: 0x1000 and 0x1080 sit on different lines.
    let mut s = PpuState::new();
    s.reservation = Some(cellgov_sync::ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x80;
    s.gpr[5] = 0xDEAD_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr_field(0), 0b0000);
    assert!(effects.is_empty());
    // PowerPC ABI: stwcx retires the reservation even on failure.
    assert!(s.reservation.is_none());
}

#[test]
fn same_unit_store_to_reserved_line_clears_local_reservation() {
    // Cross-unit contract: any plain store overlapping the reserved
    // 128-byte line must drop the local reservation so a later stwcx
    // on that same line fails.
    let mut mem = vec![0u8; 0x2000];
    mem[0x1000..0x1004].copy_from_slice(&0xdeadbeefu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    let mut effects = Vec::new();

    exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));

    s.gpr[6] = 0x1040;
    s.gpr[7] = 0xAAAA_BBBBu64;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stw {
            rs: 7,
            ra: 6,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects2,
    );
    assert!(
        s.reservation.is_none(),
        "same-unit store to reserved line must drop the local reservation"
    );

    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0x5555_6666u64;
    let mut effects3 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects3,
    );
    assert_eq!(
        s.cr_field(0),
        0b0000,
        "stwcx must fail after self-invalidation"
    );
    assert!(effects3.is_empty());
}

#[test]
fn lwarx_sets_local_reservation_and_emits_acquire() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1040..0x1044].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x40;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lwarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[5], 0xDEAD_BEEF);
    // Reservation tracks the enclosing 128-byte line, not the raw EA.
    assert_eq!(
        s.reservation.map(|l| l.addr()),
        Some(0x1000),
        "local reservation must be set to the enclosing line"
    );
    let acquires: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::ReservationAcquire { line_addr, source } => Some((*line_addr, *source)),
            _ => None,
        })
        .collect();
    assert_eq!(acquires, vec![(0x1000, UnitId::new(0))]);
}

#[test]
fn ldarx_sets_local_reservation_and_emits_acquire() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x8;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldarx {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x1000));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::ReservationAcquire {
            line_addr: 0x1000,
            ..
        }
    )));
}

#[test]
fn stwcx_on_matching_line_retires_local_reservation() {
    let mut s = PpuState::new();
    s.reservation = Some(cellgov_sync::ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xCAFE_BABE;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stwcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b0010);
    assert!(
        s.reservation.is_none(),
        "stwcx must retire the local reservation on success"
    );
}

#[test]
fn stdcx_on_matching_line_retires_local_reservation() {
    let mut s = PpuState::new();
    s.reservation = Some(cellgov_sync::ReservedLine::containing(0x1000));
    s.gpr[3] = 0x1000;
    s.gpr[4] = 0x0;
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdcx {
            rs: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert!(s.reservation.is_none());
}

#[test]
fn mulhw_signed_high_32_bits() {
    let mut s = PpuState::new();
    s.gpr[4] = (-2i32) as u32 as u64;
    s.gpr[5] = 3;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 3,
            ra: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xFFFFFFFF_FFFFFFFFu64);
}

#[test]
fn mulhw_positive_produces_zero_high_bits() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x0001_0000;
    s.gpr[5] = 0x0001_0000;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 3,
            ra: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 1);
}

#[test]
fn cntlzd_counts_64_for_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 0;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 3,
            rs: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 64);
}

#[test]
fn cntlzd_high_bit_set_returns_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 1u64 << 63;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 3,
            rs: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn addze_with_ca_zero_copies_ra() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 42);
    assert!(!s.xer_ca());
}

#[test]
fn addze_with_ca_set_adds_one() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 43);
    assert!(!s.xer_ca());
}

#[test]
fn addze_overflow_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[4] = u64::MAX;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
    assert!(s.xer_ca());
}

#[test]
fn orc_is_or_with_complement_rb() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x00FF_0000;
    s.gpr[5] = 0x0000_00FF;
    exec_no_mem(
        &PpuInstruction::Orc {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    // orc is 32-bit, result sign-extended to 64 bits on this operand.
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FF00);
}

#[test]
fn stfsu_updates_ra_and_emits_store_effect() {
    let mut s = PpuState::new();
    s.gpr[8] = 0x2000;
    s.fpr[13] = 0x4000_0000_0000_0000;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfsu {
            frs: 13,
            ra: 8,
            imm: 8,
        },
        &mut s,
        0,
        &[0u8; 0x4000],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[8], 0x2008);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x2008);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stfdu_updates_ra_and_emits_store_effect() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.fpr[1] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfdu {
            frs: 1,
            ra: 1,
            imm: -8,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[1], 0xF8);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0xF8);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stfiwx_stores_low_32_bits_of_fpr_as_integer_word() {
    // stfiwx writes the low 32 bits of the FPR bit pattern verbatim;
    // no single-precision round-convert (unlike stfs).
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000;
    s.gpr[5] = 0x20;
    s.fpr[13] = 0x4040_4040_1234_5678;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfiwx {
            frs: 13,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x1020);
            assert_eq!(bytes.bytes(), &0x1234_5678u32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn li_matches_addi_ra0() {
    let mut s1 = PpuState::new();
    exec_no_mem(
        &PpuInstruction::Addi {
            rt: 3,
            ra: 0,
            imm: 42,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    exec_no_mem(&PpuInstruction::Li { rt: 3, imm: 42 }, &mut s2);

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s2.gpr[3], 42);
}

#[test]
fn li_negative_sign_extends() {
    let mut s = PpuState::new();
    exec_no_mem(&PpuInstruction::Li { rt: 5, imm: -1 }, &mut s);
    assert_eq!(s.gpr[5], u64::MAX);
}

#[test]
fn mr_matches_or_same_reg() {
    let mut s1 = PpuState::new();
    s1.gpr[4] = 0xDEAD_BEEF;
    exec_no_mem(
        &PpuInstruction::Or {
            ra: 3,
            rs: 4,
            rb: 4,
            rc: false,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[4] = 0xDEAD_BEEF;
    exec_no_mem(&PpuInstruction::Mr { ra: 3, rs: 4 }, &mut s2);

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s2.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn slwi_shifts_left() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x0000_0001;
    exec_no_mem(&PpuInstruction::Slwi { ra: 3, rs: 4, n: 8 }, &mut s);
    assert_eq!(s.gpr[3], 0x100);
}

#[test]
fn srwi_shifts_right() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x0000_FF00;
    exec_no_mem(&PpuInstruction::Srwi { ra: 3, rs: 4, n: 8 }, &mut s);
    assert_eq!(s.gpr[3], 0xFF);
}

#[test]
fn clrlwi_clears_high_bits() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Clrlwi {
            ra: 3,
            rs: 4,
            n: 16,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x0000_FFFF);
}

#[test]
fn clrlwi_n32_zeroes_all() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Clrlwi {
            ra: 3,
            rs: 4,
            n: 32,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn lwz_cmpwi_matches_separate_execution() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1008..0x100C].copy_from_slice(&42u32.to_be_bytes());
    let mut s1 = PpuState::new();
    s1.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwz {
            rt: 3,
            ra: 1,
            imm: 8,
        },
        &mut s1,
        0,
        &mem,
        &mut effects,
    );
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 42,
        },
        &mut s1,
    );

    // Execute fused LwzCmpwi
    let mut s2 = PpuState::new();
    s2.gpr[1] = 0x1000;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 8,
            bf: 0,
            cmp_imm: 42,
        },
        &mut s2,
        0,
        &mem,
        &mut effects2,
    );

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s1.cr, s2.cr);
    assert_eq!(s2.gpr[3], 42);
    assert_eq!(s2.cr_field(0), 0b0010);
}

#[test]
fn lwz_cmpwi_lt_and_gt() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x100..0x104].copy_from_slice(&5u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 0,
            bf: 2,
            cmp_imm: 10,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 5);
    assert_eq!(s.cr_field(2), 0b1000); // LT

    mem[0x100..0x104].copy_from_slice(&20u32.to_be_bytes());
    let mut s2 = PpuState::new();
    s2.gpr[1] = 0x100;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 0,
            bf: 2,
            cmp_imm: 10,
        },
        &mut s2,
        0,
        &mem,
        &mut effects2,
    );
    assert_eq!(s2.gpr[3], 20);
    assert_eq!(s2.cr_field(2), 0b0100); // GT
}

#[test]
fn lwz_cmpwi_mem_fault() {
    let mut s = PpuState::new();
    s.gpr[1] = 0xFFFF_FFFF;
    let result = exec_no_mem(
        &PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 0,
            bf: 0,
            cmp_imm: 0,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::MemFault(_)));
}

#[test]
fn li_stw_matches_separate_execution() {
    let mut s1 = PpuState::new();
    s1.gpr[1] = 0x1000;
    let mut effects1 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Li { rt: 5, imm: 99 },
        &mut s1,
        0,
        &[0u8; 0x2000],
        &mut effects1,
    );
    exec_with_mem(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s1,
        0,
        &[0u8; 0x2000],
        &mut effects1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[1] = 0x1000;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::LiStw {
            rt: 5,
            imm: 99,
            ra_store: 1,
            store_offset: 0,
        },
        &mut s2,
        0,
        &[0u8; 0x2000],
        &mut effects2,
    );

    assert_eq!(s1.gpr[5], s2.gpr[5]);
    assert_eq!(s2.gpr[5], 99);
    assert!(!effects1.is_empty());
    assert!(!effects2.is_empty());
}

#[test]
fn li_stw_negative_imm() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::LiStw {
            rt: 3,
            imm: -1,
            ra_store: 1,
            store_offset: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.gpr[3], u64::MAX);
}

#[test]
fn mflr_stw_matches_separate_execution() {
    let mut s1 = PpuState::new();
    s1.lr = 0x0040_0100;
    s1.gpr[1] = 0x1000;
    let mut effects1 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Mflr { rt: 0 },
        &mut s1,
        0,
        &[0u8; 0x2000],
        &mut effects1,
    );
    exec_with_mem(
        &PpuInstruction::Stw {
            rs: 0,
            ra: 1,
            imm: 16,
        },
        &mut s1,
        0,
        &[0u8; 0x2000],
        &mut effects1,
    );

    let mut s2 = PpuState::new();
    s2.lr = 0x0040_0100;
    s2.gpr[1] = 0x1000;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::MflrStw {
            rt: 0,
            ra_store: 1,
            store_offset: 16,
        },
        &mut s2,
        0,
        &[0u8; 0x2000],
        &mut effects2,
    );

    assert_eq!(s1.gpr[0], s2.gpr[0]);
    assert_eq!(s2.gpr[0], 0x0040_0100);
    assert!(!effects1.is_empty());
    assert!(!effects2.is_empty());
}

#[test]
fn lwz_mtlr_matches_separate_execution() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1010..0x1014].copy_from_slice(&0x0040_0100u32.to_be_bytes());
    let mut s1 = PpuState::new();
    s1.gpr[1] = 0x1000;
    let mut effects1 = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwz {
            rt: 0,
            ra: 1,
            imm: 16,
        },
        &mut s1,
        0,
        &mem,
        &mut effects1,
    );
    exec_no_mem(&PpuInstruction::Mtlr { rs: 0 }, &mut s1);

    let mut s2 = PpuState::new();
    s2.gpr[1] = 0x1000;
    let mut effects2 = Vec::new();
    exec_with_mem(
        &PpuInstruction::LwzMtlr {
            rt: 0,
            ra_load: 1,
            offset: 16,
        },
        &mut s2,
        0,
        &mem,
        &mut effects2,
    );

    assert_eq!(s1.gpr[0], s2.gpr[0]);
    assert_eq!(s1.lr, s2.lr);
    assert_eq!(s2.gpr[0], 0x0040_0100);
    assert_eq!(s2.lr, 0x0040_0100);
}

#[test]
fn lwz_mtlr_mem_fault() {
    let mut s = PpuState::new();
    s.gpr[1] = 0xFFFF_FFFF;
    let result = exec_no_mem(
        &PpuInstruction::LwzMtlr {
            rt: 0,
            ra_load: 1,
            offset: 0,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::MemFault(_)));
}

#[test]
fn nop_matches_ori_same_reg_zero() {
    let mut s1 = PpuState::new();
    s1.gpr[5] = 0xDEAD;
    exec_no_mem(
        &PpuInstruction::Ori {
            ra: 5,
            rs: 5,
            imm: 0,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[5] = 0xDEAD;
    exec_no_mem(&PpuInstruction::Nop, &mut s2);

    assert_eq!(s1.gpr[5], s2.gpr[5]);
}

#[test]
fn nop_returns_continue() {
    let mut s = PpuState::new();
    let result = exec_no_mem(&PpuInstruction::Nop, &mut s);
    assert_eq!(result, ExecuteVerdict::Continue);
}

#[test]
fn cmpw_zero_matches_cmpwi_zero() {
    let mut s1 = PpuState::new();
    s1.gpr[3] = 42;
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 0,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[3] = 42;
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s2);

    assert_eq!(s1.cr, s2.cr);
    assert_eq!(s2.cr_field(0), 0b0100); // GT
}

#[test]
fn cmpw_zero_negative() {
    let mut s = PpuState::new();
    s.gpr[3] = (-5i64) as u64;
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
    assert_eq!(s.cr_field(0), 0b1000); // LT
}

#[test]
fn cmpw_zero_equal() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
    assert_eq!(s.cr_field(0), 0b0010); // EQ
}

#[test]
fn cmpw_zero_cr_field_2() {
    let mut s1 = PpuState::new();
    s1.gpr[7] = 0;
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 2,
            ra: 7,
            imm: 0,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[7] = 0;
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 2, ra: 7 }, &mut s2);

    assert_eq!(s1.cr, s2.cr);
}

#[test]
fn clrldi_matches_rldicl_sh0() {
    let mut s1 = PpuState::new();
    s1.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Rldicl {
            ra: 3,
            rs: 4,
            sh: 0,
            mb: 32,
            rc: false,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Clrldi {
            ra: 3,
            rs: 4,
            n: 32,
        },
        &mut s2,
    );

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s2.gpr[3], 0x0000_0000_FFFF_FFFF);
}

#[test]
fn clrldi_zero_mask_clears_all() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE;
    exec_no_mem(
        &PpuInstruction::Clrldi {
            ra: 3,
            rs: 4,
            n: 64,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn sldi_matches_rldicr() {
    let mut s1 = PpuState::new();
    s1.gpr[4] = 0x0000_0000_0000_00FF;
    exec_no_mem(
        &PpuInstruction::Rldicr {
            ra: 3,
            rs: 4,
            sh: 8,
            me: 55,
            rc: false,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[4] = 0x0000_0000_0000_00FF;
    exec_no_mem(&PpuInstruction::Sldi { ra: 3, rs: 4, n: 8 }, &mut s2);

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s2.gpr[3], 0x0000_0000_0000_FF00);
}

#[test]
fn sldi_large_shift() {
    let mut s = PpuState::new();
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Sldi {
            ra: 3,
            rs: 4,
            n: 63,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x8000_0000_0000_0000);
}

#[test]
fn srdi_matches_rldicl() {
    let mut s1 = PpuState::new();
    s1.gpr[4] = 0xFF00_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Rldicl {
            ra: 3,
            rs: 4,
            sh: 56,
            mb: 8,
            rc: false,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.gpr[4] = 0xFF00_0000_0000_0000;
    exec_no_mem(&PpuInstruction::Srdi { ra: 3, rs: 4, n: 8 }, &mut s2);

    assert_eq!(s1.gpr[3], s2.gpr[3]);
    assert_eq!(s2.gpr[3], 0x00FF_0000_0000_0000);
}

#[test]
fn srdi_large_shift() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x8000_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Srdi {
            ra: 3,
            rs: 4,
            n: 63,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 1);
}

#[test]
fn cmpwi_bc_taken_matches_separate() {
    // Equivalent sequence: cmpwi cr0, r3, 10 ; beq cr0, +16.
    let mut s1 = PpuState::new();
    s1.pc = 0x1000;
    s1.gpr[3] = 10;
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 10,
        },
        &mut s1,
    );
    s1.pc = 0x1004;
    let v1 = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 16,
            link: false,
        },
        &mut s1,
    );

    // Super sits at the cmpwi slot; bc offset is relative to super_pc + 4.
    let mut s2 = PpuState::new();
    s2.pc = 0x1000;
    s2.gpr[3] = 10;
    let v2 = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 10,
            bo: 0x0C,
            bi: 2,
            target_offset: 16,
        },
        &mut s2,
    );

    assert_eq!(s1.cr, s2.cr);
    assert_eq!(v1, ExecuteVerdict::Branch);
    assert_eq!(v2, ExecuteVerdict::Branch);
    assert_eq!(s1.pc, s2.pc);
    assert_eq!(s2.pc, 0x1014);
}

#[test]
fn cmpwi_bc_not_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.gpr[3] = 5;
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 10,
            bo: 0x0C,
            bi: 2,
            target_offset: 16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b1000); // LT
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn cmpwi_bc_gt_taken() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.gpr[3] = 10;
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 5,
            bo: 0x0C,
            bi: 1, // GT bit of cr0
            target_offset: 20,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Branch);
    assert_eq!(s.cr_field(0), 0b0100); // GT
    assert_eq!(s.pc, 0x2018);
}

#[test]
fn cmpw_bc_taken_matches_separate() {
    // Equivalent sequence: cmpw cr0, r3, r4 ; beq cr0, +16.
    let mut s1 = PpuState::new();
    s1.pc = 0x1000;
    s1.gpr[3] = 42;
    s1.gpr[4] = 42;
    exec_no_mem(
        &PpuInstruction::Cmpw {
            bf: 0,
            ra: 3,
            rb: 4,
        },
        &mut s1,
    );
    s1.pc = 0x1004;
    let v1 = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 16,
            link: false,
        },
        &mut s1,
    );

    let mut s2 = PpuState::new();
    s2.pc = 0x1000;
    s2.gpr[3] = 42;
    s2.gpr[4] = 42;
    let v2 = exec_no_mem(
        &PpuInstruction::CmpwBc {
            bf: 0,
            ra: 3,
            rb: 4,
            bo: 0x0C,
            bi: 2,
            target_offset: 16,
        },
        &mut s2,
    );

    assert_eq!(s1.cr, s2.cr);
    assert_eq!(v1, ExecuteVerdict::Branch);
    assert_eq!(v2, ExecuteVerdict::Branch);
    assert_eq!(s1.pc, s2.pc);
    assert_eq!(s2.pc, 0x1014);
}

#[test]
fn cmpw_bc_not_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.gpr[3] = 5;
    s.gpr[4] = 10;
    let v = exec_no_mem(
        &PpuInstruction::CmpwBc {
            bf: 0,
            ra: 3,
            rb: 4,
            bo: 0x0C,
            bi: 2,
            target_offset: 16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b1000); // LT
}

#[test]
fn subfc_computes_rb_minus_ra_and_sets_ca_on_no_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 7);
    assert!(s.xer_ca());
}

#[test]
fn subfc_borrow_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    s.gpr[4] = 3;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 3u64.wrapping_sub(10));
    assert!(!s.xer_ca());
}

#[test]
fn subfe_uses_carry_in() {
    // rt = ~ra + rb + CA: CA=1 gives rb - ra, CA=0 gives rb - ra - 1.
    let mut s = PpuState::new();
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 7);

    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 6);
}

#[test]
fn sraw_preserves_sign_and_caps_at_31() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_8000_0000;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i32 as i64, -2147483648i64 >> 4);
}

#[test]
fn srad_signed_64_bit_shift() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i64, (0x8000_0000_0000_0000u64 as i64) >> 4);
}

#[test]
fn sradi_shift_zero_clears_ca_and_preserves_value() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF_CAFE_F00D;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 4,
            rs: 3,
            sh: 0,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_F00D);
    assert!(!s.xer_ca());
}

#[test]
fn mulhd_signed_high_doubleword() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.gpr[4] = u64::MAX;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);

    s.gpr[3] = u64::MAX;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], u64::MAX);
}

#[test]
fn lhzu_loads_halfword_and_updates_base() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x1010..0x1012].copy_from_slice(&0xBEEFu16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lhzu {
            rt: 3,
            ra: 4,
            imm: 0x10,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xBEEF);
    assert_eq!(s.gpr[4], 0x1010);
}

#[test]
fn stdux_stores_doubleword_and_updates_base() {
    let mem = vec![0u8; 0x2000];
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF_CAFE_F00D;
    s.gpr[4] = 0x1000;
    s.gpr[5] = 0x40;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stdux {
            rs: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0x1040);
    assert!(!effects.is_empty());
}

#[test]
fn lvlx_aligned_address_matches_lvx() {
    // 16-aligned EA degenerates lvlx to lvx: zero-bit shift.
    let mut mem = vec![0u8; 0x2000];
    let pattern = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x00,
    ];
    mem[0x1000..0x1010].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvlx {
            vt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[7], u128::from_be_bytes(pattern));
}

#[test]
fn lvlx_unaligned_shifts_high_bytes_up() {
    // lvlx: result = (aligned_block << (EA & 15) * 8), low bytes zeroed.
    let mut mem = vec![0u8; 0x2000];
    let pattern = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x10,
    ];
    mem[0x1000..0x1010].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1003;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvlx {
            vt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    let expected = u128::from_be_bytes(pattern) << 24;
    assert_eq!(s.vr[7], expected);
}

#[test]
fn lvrx_unaligned_shifts_low_bytes_down() {
    // lvrx: result = (aligned_block >> (16 - (EA & 15)) * 8), high bytes zeroed.
    let mut mem = vec![0u8; 0x2000];
    let pattern = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x10,
    ];
    mem[0x1000..0x1010].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1003;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvrx {
            vt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    let expected = u128::from_be_bytes(pattern) >> 104;
    assert_eq!(s.vr[7], expected);
}

#[test]
fn lvrx_aligned_ea_zero_bytes() {
    // 16-aligned EA: (16 - 0)*8 == 128-bit shift, so lvrx result is zero.
    let mut mem = vec![0u8; 0x2000];
    mem[0x1000..0x1010].copy_from_slice(&[0xFF; 16]);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000;
    s.gpr[5] = 0;
    s.vr[7] = u128::MAX;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvrx {
            vt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[7], 0);
}

#[test]
fn cmpdi_compares_full_64_bits() {
    // With only the low 32 bits examined, 0x1_0000_0000 would compare
    // equal to zero. cmpdi must see the full doubleword.
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000_0000;
    exec_no_mem(
        &PpuInstruction::Cmpdi {
            bf: 0,
            ra: 3,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100); // GT
}

#[test]
fn cmpldi_compares_full_64_bits_unsigned() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000_0000;
    exec_no_mem(
        &PpuInstruction::Cmpldi {
            bf: 1,
            ra: 3,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(1), 0b0100); // GT
}

#[test]
fn stbu_updates_ra_with_effective_address() {
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[1] = 0x20;
    s.gpr[6] = 0xAB;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stbu {
            rs: 6,
            ra: 1,
            imm: -4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    // EA = 0x20 + (-4) = 0x1C; RA receives EA after the store.
    assert_eq!(s.gpr[1], 0x1C);
    let found = effects.iter().any(|e| match e {
        Effect::SharedWriteIntent { range, .. } => range.start().raw() == 0x1C,
        _ => false,
    });
    assert!(found, "stbu should emit a byte store at EA");
}

#[test]
fn sthu_updates_ra_with_effective_address() {
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[1] = 0x40;
    s.gpr[5] = 0xBEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Sthu {
            rs: 5,
            ra: 1,
            imm: -8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[1], 0x38);
    let found = effects.iter().any(|e| match e {
        Effect::SharedWriteIntent { range, .. } => range.start().raw() == 0x38,
        _ => false,
    });
    assert!(found, "sthu should emit a halfword store at EA");
}

#[test]
fn rldic_clears_both_sides() {
    // rldic RA, RS, SH=4, MB=32: rotate left 4, keep bits 32..=(63-4)=59.
    // RS=0xFFFF_FFFF_FFFF_FFFF, rotated left 4 still saturated, mask zeroes
    // bits 0..=31 and 60..=63.
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Rldic {
            ra: 5,
            rs: 4,
            sh: 4,
            mb: 32,
            rc: false,
        },
        &mut s,
    );
    // bits 32..=59 set, others clear.
    let expected: u64 = ((1u64 << 28) - 1) << 4;
    assert_eq!(s.gpr[5], expected);
}

#[test]
fn rldimi_preserves_prior_ra_outside_mask() {
    // rldimi RA, RS, SH=16, MB=0: mask = 0..=(63-16)=47, preserve 48..=63.
    let mut s = PpuState::new();
    s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE; // RS
    s.gpr[5] = 0x1111_2222_3333_4444; // prior RA
    exec_no_mem(
        &PpuInstruction::Rldimi {
            ra: 5,
            rs: 4,
            sh: 16,
            mb: 0,
            rc: false,
        },
        &mut s,
    );
    // rotated = RS rotl 16 = 0xBEEF_CAFE_BABE_DEAD
    // mask = 0xFFFF_FFFF_FFFF_0000 (bits 0..=47 set)
    // merged = (rotated & mask) | (prior & !mask)
    //        = 0xBEEF_CAFE_BABE_0000 | 0x0000_0000_0000_4444
    //        = 0xBEEF_CAFE_BABE_4444
    assert_eq!(s.gpr[5], 0xBEEF_CAFE_BABE_4444);
}

#[test]
fn srad_shifts_full_64_bits_arithmetically() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFF0; // -16
    s.gpr[5] = 4;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    // -16 >> 4 = -1, sign-extended across all 64 bits.
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFF);
}

#[test]
fn mftbu_returns_upper_32_bits_of_tb() {
    let mut s = PpuState::new();
    s.tb = 0xAAAA_BBBB_0000_0000 - 1; // post-increment lands at 0xAAAA_BBBB_0000_0000
    exec_no_mem(&PpuInstruction::Mftbu { rt: 6 }, &mut s);
    assert_eq!(s.gpr[6], 0xAAAA_BBBB);
}

#[test]
fn mftb_returns_strictly_increasing_values_within_step() {
    // Two consecutive mftb reads in the same step must differ so a
    // guest doing `delta = t2 - t1` never observes zero.
    let mut s = PpuState::new();
    s.tb = 100;
    exec_no_mem(&PpuInstruction::Mftb { rt: 3 }, &mut s);
    let t1 = s.gpr[3];
    exec_no_mem(&PpuInstruction::Mftb { rt: 4 }, &mut s);
    let t2 = s.gpr[4];
    assert!(
        t2 > t1,
        "mftb must strictly increase per read: {t1} -> {t2}"
    );
}

#[test]
fn vsldoi_shifts_by_shb_bytes() {
    let mut s = PpuState::new();
    s.vr[1] = u128::from_be_bytes([
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF,
    ]);
    s.vr[2] = u128::from_be_bytes([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ]);
    exec_no_mem(
        &PpuInstruction::Vsldoi {
            vt: 3,
            va: 1,
            vb: 2,
            shb: 4,
        },
        &mut s,
    );
    // Shift left by 4 bytes: result[0..12] = va[4..16], result[12..16] = vb[0..4].
    let expected = u128::from_be_bytes([
        0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x01, 0x02, 0x03,
        0x04,
    ]);
    assert_eq!(s.vr[3], expected);
}

// -- Rc / OE regression tests --
// Record form (Rc=1) must set CR0 LT/GT/EQ from the signed 64-bit
// result, plus the sticky SO from XER. OE=1 must set XER OV and the
// sticky SO on overflow.

#[test]
fn add_dot_sets_cr0_eq_when_result_is_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = (-1i64) as u64;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn add_dot_sets_cr0_lt_when_result_is_negative() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = (-2i64) as u64;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn add_rc_zero_leaves_cr0_untouched() {
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b0100);
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100, "CR0 preserved when Rc=0");
}

#[test]
fn addo_sets_xer_ov_and_sticky_so() {
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");

    // Non-overflow op clears OV but SO stays sticky.
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO remains sticky");
}

#[test]
fn or_dot_sets_cr0_without_touching_result() {
    // Catches the regression where `or. rA, rS, rS` was quickened to
    // `Mr`, which does not update CR0.
    let mut s = PpuState::new();
    s.gpr[4] = (-5i64) as u64;
    exec_no_mem(
        &PpuInstruction::Or {
            ra: 3,
            rs: 4,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], (-5i64) as u64);
    assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
}

#[test]
fn and_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFF00;
    s.gpr[4] = 0x00FF;
    exec_no_mem(
        &PpuInstruction::And {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn slw_dot_sets_cr0_from_sign_extended_low_32() {
    // Result is 0x8000_0000 as u32, which sign-extends to a negative
    // i64 -- CR0 should read LT.
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 31;
    exec_no_mem(
        &PpuInstruction::Slw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0x8000_0000);
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn srad_dot_sets_cr0_and_preserves_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = (-1i64) as u64; // all-ones, guaranteed 1-bit shifted out.
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    // -1 >> 1 = -1, and a 1 bit was shifted out of a negative value: CA set.
    assert!(s.xer_ca(), "CA set from nonzero bits shifted out");
    assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
}

#[test]
fn sradi_dot_sets_cr0() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 5,
            rs: 3,
            sh: 8,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn cntlzd_dot_sets_cr0_gt_when_value_nonzero() {
    let mut s = PpuState::new();
    s.gpr[3] = 1u64 << 40;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 23);
    assert_eq!(s.cr_field(0), 0b0100);
}

#[test]
fn rldicl_dot_sets_cr0_and_does_not_quicken_to_clrldi() {
    // Verifies the shadow-layer guard: rldicl. with sh=0 cannot be
    // quickened to Clrldi because Clrldi does not update CR0.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    exec_no_mem(
        &PpuInstruction::Rldicl {
            ra: 5,
            rs: 3,
            sh: 0,
            mb: 32,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn rldimi_dot_sets_cr0_from_merged_value() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1; // RS
    s.gpr[5] = 0xFFFF_FFFF_FFFF_FFFF; // prior RA (bits outside mask preserved)
                                      // rldimi. rA, rS, 32, 0: mask = bits 0..=31, merge RS<<32 into high half.
    exec_no_mem(
        &PpuInstruction::Rldimi {
            ra: 5,
            rs: 3,
            sh: 32,
            mb: 0,
            rc: true,
        },
        &mut s,
    );
    // rotated = 1 rotl 32 = 0x0000_0001_0000_0000
    // mask = 0xFFFF_FFFF_0000_0000
    // merged = (rotated & mask) | (prior & !mask)
    //        = 0x0000_0001_0000_0000 | 0x0000_0000_FFFF_FFFF
    //        = 0x0000_0001_FFFF_FFFF
    assert_eq!(s.gpr[5], 0x0000_0001_FFFF_FFFF);
    assert_eq!(s.cr_field(0), 0b0100, "positive nonzero");
}

#[test]
fn nego_of_int_min_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Neg {
            rt: 5,
            ra: 3,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn divwo_div_by_zero_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divw {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn mullwo_with_overflow_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000;
    s.gpr[4] = 0x1_0000;
    exec_no_mem(
        &PpuInstruction::Mullw {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    // 0x1_0000 * 0x1_0000 = 0x1_0000_0000, overflows 32-bit signed.
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn cr0_so_bit_tracks_sticky_xer_so() {
    // After an overflow, every record-form instruction must copy the
    // current (sticky) SO into CR0.SO.
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    // SO is set. A subsequent dot-form should carry SO into CR0.
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0101, "GT plus sticky SO");
}

#[test]
fn addo_dot_combined_sets_both_ov_and_cr0() {
    // oe=rc=true: executor must act on both bits independently.
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
    // Result is INT_MIN, negative -- CR0 = LT plus sticky SO.
    assert_eq!(s.cr_field(0), 0b1001);
}

#[test]
fn srawi_dot_sets_both_ca_and_cr0() {
    let mut s = PpuState::new();
    s.gpr[3] = (-1i32) as u32 as u64;
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 1,
            rc: true,
        },
        &mut s,
    );
    // -1 arithmetic-shift-right-by-1 yields -1; negative RS with a
    // 1-bit shifted out sets CA; Rc sets CR0 LT from the negative result.
    assert!(s.xer_ca());
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn srawi_sh_zero_clears_ca() {
    // Book I p. 80: "A shift amount of zero causes RA to receive
    // EXTS(RS[32:63]), and CA to be set to 0." CA is explicitly
    // cleared, not computed from the (nonexistent) shifted-out bits.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 0,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "sh=0 must clear CA regardless of prior value");
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF, "EXTS of -1 low word");
}

#[test]
fn dcbz_zeroes_128_byte_aligned_block() {
    // Buffer spans three cache lines with sentinel bytes before and
    // after the target block so we can confirm dcbz does not spill.
    let base = 0x1000u64;
    let mut mem = vec![0xAAu8; 384];
    let mut s = PpuState::new();
    // ea = 0x1000 + 100 = 0x1064; aligned block starts at 0x1000.
    s.gpr[3] = 100;
    s.gpr[4] = 0x1000;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        base,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    // Reconstruct what committed memory would look like after the 16
    // SharedWriteIntent effects land.
    for eff in &effects {
        if let cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } = eff {
            let start = (range.start().raw() - base) as usize;
            let end = start + range.length() as usize;
            mem[start..end].copy_from_slice(bytes.bytes());
        }
    }
    // Bytes outside the 128-byte aligned block are untouched.
    for (i, b) in mem.iter().enumerate().take(384) {
        let in_block = i < 128;
        let expected = if in_block { 0 } else { 0xAA };
        assert_eq!(*b, expected, "mem[{i}] (in_block={in_block})");
    }
    assert_eq!(effects.len(), 16, "16 doubleword zero effects");
}

#[test]
fn dcbz_ea_is_aligned_down_to_block_boundary() {
    let base = 0x2000u64;
    let mem = vec![0xFFu8; 256];
    let mut s = PpuState::new();
    // EA = 0x2000 + 0x7F = 0x207F. Aligned EA = 0x2000.
    s.gpr[3] = 0x7F;
    s.gpr[4] = 0x2000;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        base,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    // All effect ranges must start in [0x2000, 0x2080).
    for eff in &effects {
        if let cellgov_effects::Effect::SharedWriteIntent { range, .. } = eff {
            let addr = range.start().raw();
            assert!(
                (0x2000..0x2080).contains(&addr),
                "effect addr 0x{addr:x} outside aligned block [0x2000, 0x2080)",
            );
        }
    }
}

#[test]
fn srad_shift_ge_64_collapses_to_sign_broadcast() {
    // shift >= 64: RA = 64 copies of the sign bit, CA = sign bit.
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.gpr[4] = 64;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF);
    assert!(s.xer_ca());

    // shift > 64 with positive RS: all zeros, CA clear.
    s.gpr[3] = 0x1;
    s.gpr[4] = 100;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert!(!s.xer_ca());
}
