use super::*;
use cellgov_event::UnitId;

fn uid() -> UnitId {
    UnitId::new(0)
}

/// Shorthand: execute with no memory regions. Good for ALU /
/// branch / SPR tests that never touch memory.
fn exec_no_mem(insn: &PpuInstruction, s: &mut PpuState) -> ExecuteVerdict {
    let mut effects = Vec::new();
    let mut store_buf = crate::store_buffer::StoreBuffer::new();
    execute(insn, s, uid(), &[], &mut effects, &mut store_buf)
}

/// Execute with a single flat memory region starting at `base`.
/// After execution, flushes the store buffer into `effects`.
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
    // BO=0x14 = always taken (don't test CR, don't decr CTR)
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
    // slwi r3, r5, 16 = rlwinm r3, r5, 16, 0, 15
    exec_no_mem(
        &PpuInstruction::Rlwinm {
            ra: 3,
            rs: 5,
            sh: 16,
            mb: 0,
            me: 15,
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
    // Wrapped: bits [0..3] and [28..31]
    assert_eq!(rlwinm_mask(28, 3), 0xF000000F);
}

#[test]
fn ldu_writes_ea_back_to_ra() {
    // ldu r7, -8(r4): read 8 bytes at r4-8, set r4 := r4-8.
    // Place 8 bytes of data at address 0x1018.
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
    // Update form: RA holds the effective address after the instruction.
    assert_eq!(s.gpr[4], 0x1018);
}

#[test]
fn rlwnm_rotates_by_rb_low_5_bits() {
    // rlwnm r0, r0, r8, 0, 31: full-word rotate left by r8 mod 32.
    let mut s = PpuState::new();
    s.gpr[0] = 0x0000_0000_1234_5678;
    s.gpr[8] = 8; // rotate by 8
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 0,
            rs: 0,
            rb: 8,
            mb: 0,
            me: 31,
        },
        &mut s,
    );
    // 0x12345678 rotated left by 8 = 0x34567812
    assert_eq!(s.gpr[0], 0x3456_7812);
}

#[test]
fn rlwnm_ignores_high_bits_of_rb() {
    // Only low 5 bits of RB are used. 0x20 == 32 -> 0 rotation.
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
    // Should have emitted one SharedWriteIntent at aligned EA 0x1010.
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x1010);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn extsw_sign_extends_low_32_bits() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_8000_0000; // bit 31 set in low word
    exec_no_mem(&PpuInstruction::Extsw { ra: 4, rs: 3 }, &mut s);
    assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
}

#[test]
fn sc_returns_syscall() {
    let mut s = PpuState::new();
    let result = exec_no_mem(&PpuInstruction::Sc, &mut s);
    assert!(matches!(result, ExecuteVerdict::Syscall));
}

#[test]
fn lwz_loads_from_memory() {
    // Place 0xDEADBEEF at address 0x1008.
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
    // Place 0xFF80 (-128 as i16) at address 0x1002.
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
    s.set_cr_field(0, 0b0010); // EQ set
                               // beq cr0, +8: BO=0x0C (test CR, don't decr CTR), BI=2 (EQ bit of cr0)
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
    s.set_cr_field(0, 0b0100); // GT set, not EQ
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
    assert_eq!(s.pc, 0x1000); // unchanged
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
        },
        &mut s,
    );
    // -1 * 2 = -2 (wrapping) = 0xFFFF_FFFF_FFFF_FFFE
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFE);
}

#[test]
fn adde_adds_with_carry_in_and_sets_carry_out() {
    let mut s = PpuState::new();
    // First: adde with CA=1 on overflowing low word produces carry-out.
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    // 0xFFFF... + 0 + 1 = 0, with carry.
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
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 8);
    assert!(!s.xer_ca());
}

#[test]
fn mulhdu_takes_high_64_bits_of_u128_product() {
    // 0xFFFF_FFFF_FFFF_FFFF * 2 = 0x1_FFFF_FFFF_FFFF_FFFE,
    // high 64 bits = 1.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 1);
}

#[test]
fn mulhdu_small_product_is_zero() {
    // 7 * 8 = 56; fits in 64 bits, so high 64 bits = 0.
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
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
fn stdcx_always_succeeds_in_single_threaded() {
    let mut s = PpuState::new();
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
    // CR0 EQ must be set to indicate success.
    assert_eq!(s.cr_field(0), 0b0010);
    // Should have emitted one store effect.
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x1008);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

// stfiwx / stfsu / stfdu / mulhw / cntlzd / addze / orc: one
// exec test per variant pinning the semantics.

#[test]
fn mulhw_signed_high_32_bits() {
    // -2 * 3 = -6; high 32 bits sign-extended == 0xFFFFFFFF.
    let mut s = PpuState::new();
    s.gpr[4] = (-2i32) as u32 as u64;
    s.gpr[5] = 3;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 3,
            ra: 4,
            rb: 5,
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
        },
        &mut s,
    );
    // 0x10000 * 0x10000 = 0x1_0000_0000; high 32 = 1.
    assert_eq!(s.gpr[3], 1);
}

#[test]
fn cntlzd_counts_64_for_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 0;
    exec_no_mem(&PpuInstruction::Cntlzd { ra: 3, rs: 5 }, &mut s);
    assert_eq!(s.gpr[3], 64);
}

#[test]
fn cntlzd_high_bit_set_returns_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 1u64 << 63;
    exec_no_mem(&PpuInstruction::Cntlzd { ra: 3, rs: 5 }, &mut s);
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn addze_with_ca_zero_copies_ra() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(false);
    exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
    assert_eq!(s.gpr[3], 42);
    assert!(!s.xer_ca());
}

#[test]
fn addze_with_ca_set_adds_one() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(true);
    exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
    assert_eq!(s.gpr[3], 43);
    assert!(!s.xer_ca());
}

#[test]
fn addze_overflow_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[4] = u64::MAX;
    s.set_xer_ca(true);
    exec_no_mem(&PpuInstruction::Addze { rt: 3, ra: 4 }, &mut s);
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
        },
        &mut s,
    );
    // 0x00FF_0000 | !0x0000_00FF == 0xFFFF_FF00 sign-extended to u64
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
    assert_eq!(s.gpr[8], 0x2008, "ra is updated to ea");
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
    // Unlike Stfs (single-precision round-convert), stfiwx writes
    // the low 32 bits of the FPR bit pattern verbatim as a u32.
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
            assert_eq!(
                bytes.bytes(),
                &0x1234_5678u32.to_be_bytes(),
                "low 32 bits verbatim"
            );
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

// -- Quickened instruction tests --

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
    // clrlwi clears the top 16 bits of the 32-bit value
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

// =================================================================
// Superinstruction tests
// =================================================================

#[test]
fn lwz_cmpwi_matches_separate_execution() {
    // Execute lwz + cmpwi separately
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
    assert_eq!(s2.cr_field(0), 0b0010); // EQ
}

#[test]
fn lwz_cmpwi_lt_and_gt() {
    let mut mem = vec![0u8; 0x2000];
    // Store value 5 (less than 10)
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

    // Store value 20 (greater than 10)
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
    // Execute li + stw separately
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

    // Execute fused LiStw
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
    // Both should produce a store effect at address 0x1000
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
    // Execute mflr + stw separately
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

    // Execute fused MflrStw
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
    // Execute lwz + mtlr separately
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

    // Execute fused LwzMtlr
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

// -- Nop --

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

// -- CmpwZero --

#[test]
fn cmpw_zero_matches_cmpwi_zero() {
    // Positive value
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

// -- Clrldi --

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

// -- Sldi --

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

// -- Srdi --

#[test]
fn srdi_matches_rldicl() {
    let mut s1 = PpuState::new();
    s1.gpr[4] = 0xFF00_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Rldicl {
            ra: 3,
            rs: 4,
            sh: 56, // 64 - 8
            mb: 8,
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

// -- CmpwiBc --

#[test]
fn cmpwi_bc_taken_matches_separate() {
    // cmpwi cr0, r3, 10; beq cr0, +16
    // Separate execution:
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
    s1.pc = 0x1004; // advance for bc
    let v1 = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 16,
            link: false,
        },
        &mut s1,
    );

    // Fused execution: super is at the cmpwi slot (0x1000).
    // The bc offset (16) is relative to the bc's original PC
    // (super + 4 = 0x1004), so target = 0x1004 + 16 = 0x1014.
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
    assert_eq!(s2.pc, 0x1014); // 0x1004 + 16
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
            bo: 0x0C, // test CR, don't decr CTR
            bi: 2,    // EQ bit of cr0
            target_offset: 16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b1000); // LT
                                       // PC not modified by the super on not-taken; outer loop
                                       // advances by 4, then Consumed advances by 4, total +8.
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn cmpwi_bc_gt_taken() {
    // cmpwi cr0, r3, 5; bgt cr0, +20
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.gpr[3] = 10;
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 5,
            bo: 0x0C, // test CR, don't decr CTR
            bi: 1,    // GT bit of cr0
            target_offset: 20,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Branch);
    assert_eq!(s.cr_field(0), 0b0100); // GT
                                       // target = (super_pc + 4) + 20 = 0x2004 + 20 = 0x2018
    assert_eq!(s.pc, 0x2018);
}

#[test]
fn cmpw_bc_taken_matches_separate() {
    // cmpw cr0, r3, r4; beq cr0, +16
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

// -- Phase 16 additions --

#[test]
fn subfc_computes_rb_minus_ra_and_sets_ca_on_no_borrow() {
    let mut s = PpuState::new();
    // rb(10) - ra(3) = 7; no borrow -> CA=1
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 7);
    assert!(s.xer_ca());
}

#[test]
fn subfc_borrow_clears_ca() {
    let mut s = PpuState::new();
    // rb(3) - ra(10) = wrapping; borrow -> CA=0
    s.gpr[3] = 10;
    s.gpr[4] = 3;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 3u64.wrapping_sub(10));
    assert!(!s.xer_ca());
}

#[test]
fn subfe_uses_carry_in() {
    // rt = ~ra + rb + CA. With CA=1 this is rb - ra; with CA=0
    // this is rb - ra - 1.
    let mut s = PpuState::new();
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
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
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 6);
}

#[test]
fn sraw_preserves_sign_and_caps_at_31() {
    let mut s = PpuState::new();
    // Sign-propagating right shift on the low 32 bits.
    s.gpr[3] = 0xFFFF_FFFF_8000_0000; // low32 = -2147483648
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
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
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_F00D);
    assert!(!s.xer_ca());
}

#[test]
fn mulhd_signed_high_doubleword() {
    let mut s = PpuState::new();
    // -1 * -1 = 1 as i128 = 0x0000_0000_0000_0001, high 64 bits = 0
    s.gpr[3] = u64::MAX; // -1 as i64
    s.gpr[4] = u64::MAX;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);

    // -1 * 2 = -2 as i128, high 64 bits = -1 = 0xFFFF_FFFF_FFFF_FFFF
    s.gpr[3] = u64::MAX;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], u64::MAX);
}

#[test]
fn lhzu_loads_halfword_and_updates_base() {
    // Place 0xBEEF at address 0x1010.
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
    assert_eq!(s.gpr[4], 0x1010, "base register updated with EA");
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
    assert_eq!(s.gpr[4], 0x1040, "base updated to EA = ra + rb");
    // One SharedWriteIntent for the 8-byte store.
    assert!(!effects.is_empty());
}

#[test]
fn lvlx_aligned_address_matches_lvx() {
    // With EA already 16-aligned, lvlx == lvx (no shift).
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
    // EA has low 4 bits = 3. lvlx shifts the loaded 16 bytes
    // left by 3*8 = 24 bits: the top 13 bytes of the aligned
    // block become the top 13 bytes of the result, and the
    // bottom 3 bytes are zero.
    let mut mem = vec![0u8; 0x2000];
    let pattern = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x10,
    ];
    mem[0x1000..0x1010].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1003; // EA & 15 = 3
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
    // EA has low 4 bits = 3. lvrx shifts right by (16-3)*8 = 104
    // bits: only the high 3 bytes of the aligned block survive,
    // landing in the low 3 bytes of the result.
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
    // EA aligned (low 4 bits = 0): lvrx result is all zero.
    let mut mem = vec![0u8; 0x2000];
    mem[0x1000..0x1010].copy_from_slice(&[0xFF; 16]);
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000;
    s.gpr[5] = 0;
    s.vr[7] = u128::MAX; // pre-fill to verify it's overwritten
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
