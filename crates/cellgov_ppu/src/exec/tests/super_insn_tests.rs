//! Quickened and fused super-instruction execution, including fault and effect paths.

use super::*;
use crate::exec::test_support::{exec_no_mem, exec_with_mem};
use cellgov_effects::Effect;

#[test]
fn li_negative_sign_extends() {
    let mut s = PpuState::new();
    exec_no_mem(&PpuInstruction::Li { rt: 5, imm: -1 }, &mut s);
    assert_eq!(s.gpr[5], u64::MAX);
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
fn nop_returns_continue() {
    let mut s = PpuState::new();
    let result = exec_no_mem(&PpuInstruction::Nop, &mut s);
    assert_eq!(result, ExecuteVerdict::Continue);
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
fn li_stw_emits_store_effect() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::LiStw {
            rt: 5,
            imm: 99,
            ra_store: 1,
            store_offset: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.gpr[5], 99);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SharedWriteIntent { .. })));
}

#[test]
fn li_stw_stores_low_32_bits_only() {
    // LiStw with imm=-1 sign-extends to 0xFFFF_FFFF_FFFF_FFFF in
    // RT, but stw must store only the low 32 bits. Locks the
    // implicit `buffer_store(_, _, _, 4, val)` low-bytes
    // contract so a future change cannot silently store the
    // sign-extended high half.
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
    let stored = effects
        .iter()
        .find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1000 => {
                Some(bytes.bytes().to_vec())
            }
            _ => None,
        })
        .expect("LiStw must emit a store at 0x1000");
    assert_eq!(stored, vec![0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(stored.len(), 4, "stw stores 4 bytes, not 8");
}

#[test]
fn cmpw_zero_positive() {
    let mut s = PpuState::new();
    s.gpr[3] = 5;
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
    assert_eq!(s.cr_field(0), 0b0100); // GT
}

#[test]
fn cmpw_zero_propagates_xer_so() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ov(true); // set sticky SO
    exec_no_mem(&PpuInstruction::CmpwZero { bf: 0, ra: 3 }, &mut s);
    // EQ + SO = 0b0010 | 0b0001 = 0b0011.
    assert_eq!(s.cr_field(0), 0b0011);
}

#[test]
fn lwz_cmpwi_propagates_xer_so() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x100..0x104].copy_from_slice(&5u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.set_xer_ov(true);
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
    // LT + SO.
    assert_eq!(s.cr_field(2), 0b1001);
}

#[test]
fn lwz_cmpwi_loaded_value_negative_as_i32() {
    // lwz zero-extends in the GPR but cmpwi treats the low 32
    // bits as signed. A loaded 0xFFFF_FFFF must compare as -1,
    // not 4294967295.
    let mut mem = vec![0u8; 0x2000];
    mem[0x100..0x104].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 0,
            bf: 0,
            cmp_imm: 1,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xFFFF_FFFF, "lwz zero-extends in the GPR");
    assert_eq!(s.cr_field(0), 0b1000, "cmpwi sees -1, less than 1");
}

#[test]
fn cmpwi_bc_propagates_xer_so_when_not_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.gpr[3] = 5;
    s.set_xer_ov(true);
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 10,
            bo: 0x0C,
            bi: 1, // GT bit -- compare result is LT, so not taken
            target_offset: 16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.cr_field(0), 0b1001, "LT + SO");
}

#[test]
fn cmpwi_bc_negative_target_offset_branches_backward() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.gpr[3] = 0;
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 0,
            bo: 0x0C,
            bi: 2, // EQ
            target_offset: -16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Branch);
    // Target is (super + 4) + offset = 0x2004 + (-16) = 0x1FF4.
    assert_eq!(s.pc, 0x1FF4);
}

#[test]
fn mflr_std_writes_lr_then_stores_64_bits() {
    let mut s = PpuState::new();
    s.lr = 0xDEAD_BEEF_CAFE_BABE;
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::MflrStd {
            rt: 3,
            ra_store: 1,
            store_offset: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
    let stored = effects
        .iter()
        .find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1000 => {
                Some((range.length(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .expect("MflrStd must emit a store");
    assert_eq!(stored.0, 8, "std stores 8 bytes");
    assert_eq!(
        stored.1,
        vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE]
    );
}

#[test]
fn ld_mtlr_loads_into_rt_and_lr() {
    let mut mem = vec![0u8; 0x2000];
    mem[0x100..0x108].copy_from_slice(&0xCAFE_BABE_DEAD_BEEFu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::LdMtlr {
            rt: 3,
            ra_load: 1,
            offset: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xCAFE_BABE_DEAD_BEEF);
    assert_eq!(s.lr, 0xCAFE_BABE_DEAD_BEEF);
}

#[test]
fn li_loads_sign_extended_immediate() {
    // li rt, SI is addi rt, 0, SI -- RT <- EXTS(SI), no other state.
    let mut s = PpuState::new();
    s.gpr[5] = 0xDEAD_BEEF_CAFE_BABE; // pre-existing value clobbered
    exec_no_mem(&PpuInstruction::Li { rt: 5, imm: 0x1234 }, &mut s);
    assert_eq!(s.gpr[5], 0x1234);
}

#[test]
fn mr_copies_full_64_bits() {
    // mr ra, rs is or ra, rs, rs -- RA <- (RS), full 64 bits.
    let mut s = PpuState::new();
    s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE;
    s.gpr[3] = 0;
    exec_no_mem(&PpuInstruction::Mr { ra: 3, rs: 4 }, &mut s);
    assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
    assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_BABE, "source unchanged");
}

#[test]
fn mr_same_register_is_identity() {
    // or rs, rs, rs with RA == RS is a self-copy; semantics equal
    // the no-op transform but the executor must still leave the
    // value unchanged rather than zeroing it.
    let mut s = PpuState::new();
    s.gpr[7] = 0x1234_5678_9ABC_DEF0;
    exec_no_mem(&PpuInstruction::Mr { ra: 7, rs: 7 }, &mut s);
    assert_eq!(s.gpr[7], 0x1234_5678_9ABC_DEF0);
}

#[test]
fn nop_advances_nia_no_state_change() {
    // ori 0, 0, 0 -- no architected state change. Verifies Continue
    // verdict and that no GPR / CR / XER / LR moved.
    let mut s = PpuState::new();
    for i in 0..32 {
        s.gpr[i] = i as u64 * 0x0101_0101_0101_0101;
    }
    s.lr = 0xAAAA_BBBB_CCCC_DDDD;
    s.set_cr_field(0, 0b0100);
    s.set_xer_ov(true);
    let snapshot_gpr = s.gpr;
    let snapshot_lr = s.lr;
    let snapshot_cr0 = s.cr_field(0);
    let snapshot_so = s.xer_so();
    let v = exec_no_mem(&PpuInstruction::Nop, &mut s);
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr, snapshot_gpr);
    assert_eq!(s.lr, snapshot_lr);
    assert_eq!(s.cr_field(0), snapshot_cr0);
    assert_eq!(s.xer_so(), snapshot_so);
}

#[test]
fn slwi_n1_shifts_low_32_then_zero_extends() {
    // slwi ra, rs, 1 == rlwinm ra, rs, 1, 0, 30: the low 32 bits
    // shift left by 1 and the result zero-extends into RA.
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_8000_0001;
    exec_no_mem(&PpuInstruction::Slwi { ra: 3, rs: 4, n: 1 }, &mut s);
    // Low 32 bits 0x8000_0001 << 1 = 0x0000_0002 (high bit dropped
    // off the 32-bit edge), zero-extended.
    assert_eq!(s.gpr[3], 0x0000_0002);
}

#[test]
fn slwi_n31_keeps_only_low_bit() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Slwi {
            ra: 3,
            rs: 4,
            n: 31,
        },
        &mut s,
    );
    // Low 32 bits all-ones << 31 = 0x8000_0000.
    assert_eq!(s.gpr[3], 0x8000_0000);
}

#[test]
fn srwi_n1_shifts_low_32_unsigned() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_8000_0001;
    exec_no_mem(&PpuInstruction::Srwi { ra: 3, rs: 4, n: 1 }, &mut s);
    // Low 32 bits 0x8000_0001 >> 1 = 0x4000_0000 (unsigned, the
    // high 32 bits of RS are ignored).
    assert_eq!(s.gpr[3], 0x4000_0000);
}

#[test]
fn srwi_n31_isolates_top_bit_of_low_word() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_8000_0000;
    exec_no_mem(
        &PpuInstruction::Srwi {
            ra: 3,
            rs: 4,
            n: 31,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x1);
}

#[test]
fn clrlwi_n0_zero_extends_low_32() {
    // clrlwi ra, rs, 0 == rlwinm ra, rs, 0, 0, 31: keep the low
    // 32 bits, zero-extend; the high 32 bits of RS are discarded.
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_DEAD_BEEF;
    exec_no_mem(&PpuInstruction::Clrlwi { ra: 3, rs: 4, n: 0 }, &mut s);
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn clrlwi_n31_keeps_only_low_bit() {
    // clrlwi ra, rs, 31: only bit 31 of the low word survives.
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Clrlwi {
            ra: 3,
            rs: 4,
            n: 31,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x1);
}

#[test]
fn sldi_n1_doubles_full_64_bits() {
    // sldi ra, rs, 1 == rldicr ra, rs, 1, 62: shift left by 1
    // across the full 64-bit word.
    let mut s = PpuState::new();
    s.gpr[4] = 0x4000_0000_0000_0001;
    exec_no_mem(&PpuInstruction::Sldi { ra: 3, rs: 4, n: 1 }, &mut s);
    assert_eq!(s.gpr[3], 0x8000_0000_0000_0002);
}

#[test]
fn srdi_n1_halves_full_64_bits_unsigned() {
    // srdi ra, rs, 1 == rldicl ra, rs, 63, 1: logical shift right
    // by 1 across all 64 bits.
    let mut s = PpuState::new();
    s.gpr[4] = 0x8000_0000_0000_0002;
    exec_no_mem(&PpuInstruction::Srdi { ra: 3, rs: 4, n: 1 }, &mut s);
    assert_eq!(s.gpr[3], 0x4000_0000_0000_0001);
}

#[test]
fn mflr_stw_writes_lr_then_stores_low_32_bits() {
    // mflr rt then stw rt, off(ra): rt gets full LR (64 bits), but
    // stw only commits the low 32 bits at EA = (ra) + offset.
    let mut s = PpuState::new();
    s.lr = 0xDEAD_BEEF_CAFE_BABE;
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::MflrStw {
            rt: 3,
            ra_store: 1,
            store_offset: 4,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
    let stored = effects
        .iter()
        .find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1004 => {
                Some((range.length(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .expect("MflrStw must emit a store at 0x1004");
    assert_eq!(stored.0, 4, "stw stores 4 bytes");
    // Low 32 bits of 0xDEAD_BEEF_CAFE_BABE = 0xCAFE_BABE in big-endian.
    assert_eq!(stored.1, vec![0xCA, 0xFE, 0xBA, 0xBE]);
}

#[test]
fn stdstd_writes_both_doublewords_at_consecutive_eas() {
    // EA1 = ra + offset1; EA2 = EA1 + 8 by fuser construction.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[5] = 0x1122_3344_5566_7788;
    s.gpr[6] = 0x99AA_BBCC_DDEE_FF00;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::StdStd {
            rs1: 5,
            rs2: 6,
            ra: 1,
            offset1: 0x10,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    let s1 = effects
        .iter()
        .find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1010 => {
                Some((range.length(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .expect("StdStd must emit a store at EA1 = 0x1010");
    let s2 = effects
        .iter()
        .find_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } if range.start().raw() == 0x1018 => {
                Some((range.length(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .expect("StdStd must emit a store at EA2 = EA1 + 8 = 0x1018");
    assert_eq!(s1.0, 8, "std stores 8 bytes");
    assert_eq!(s2.0, 8, "std stores 8 bytes");
    assert_eq!(
        s1.1,
        vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        "RS1 stored big-endian at EA1"
    );
    assert_eq!(
        s2.1,
        vec![0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00],
        "RS2 stored big-endian at EA2"
    );
}

#[test]
fn stdstd_negative_offset1_still_uses_ea2_eq_ea1_plus_8() {
    // Verifies the EA2 = EA1 + 8 relationship survives a negative
    // offset1: EA1 is signed-extended, EA2 is the literal +8.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1100;
    s.gpr[5] = 0xAAAA_AAAA_AAAA_AAAA;
    s.gpr[6] = 0xBBBB_BBBB_BBBB_BBBB;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::StdStd {
            rs1: 5,
            rs2: 6,
            ra: 1,
            offset1: -0x10,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    // EA1 = 0x1100 + (-0x10) = 0x10F0; EA2 = 0x10F8.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SharedWriteIntent { range, .. } if range.start().raw() == 0x10F0
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SharedWriteIntent { range, .. } if range.start().raw() == 0x10F8
    )));
}

#[test]
fn lwz_cmpwi_preserves_xer_and_lr() {
    // Fusion of lwz (memory load, no CR/XER/LR write) + cmpwi (CR
    // write only). The XER bits and LR must be untouched; the
    // targeted CR field is expected to change.
    let mut mem = vec![0u8; 0x2000];
    mem[0x100..0x104].copy_from_slice(&5u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.lr = 0xDEAD_BEEF_CAFE_BABE;
    let xer_before = s.xer;
    let lr_before = s.lr;
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
    assert_eq!(s.xer, xer_before, "XER must not change");
    assert_eq!(s.lr, lr_before, "LR must not change");
    assert_eq!(s.cr_field(2), 0b1000, "targeted CR field updates");
}

#[test]
fn mflr_stw_does_not_touch_cr_or_xer() {
    // mflr+stw: neither composing instruction writes CR or XER.
    let mut s = PpuState::new();
    s.lr = 0x1234_5678_9ABC_DEF0;
    s.gpr[1] = 0x1000;
    s.set_cr_field(0, 0b0101);
    s.set_cr_field(3, 0b1010);
    s.set_xer_ov(true);
    let cr_before = s.cr;
    let xer_before = s.xer;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::MflrStw {
            rt: 3,
            ra_store: 1,
            store_offset: 0,
        },
        &mut s,
        0,
        &[0u8; 0x2000],
        &mut effects,
    );
    assert_eq!(s.cr, cr_before, "CR must not change");
    assert_eq!(s.xer, xer_before, "XER must not change");
}

#[test]
fn cmpwi_bc_not_taken_does_not_touch_lr_or_ctr() {
    // The fuser only emits CmpwiBc for LK=0 bc with non-decrementing
    // BO. With the condition false the bc skips: LR and CTR stay
    // put, only the CR field updates.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.gpr[3] = 5;
    s.lr = 0xDEAD;
    s.ctr = 0x4242;
    let lr_before = s.lr;
    let ctr_before = s.ctr;
    let v = exec_no_mem(
        &PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 10,
            bo: 0x0C, // CR test, no CTR decrement
            bi: 1,    // GT, but compare is LT -> not taken
            target_offset: 16,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.lr, lr_before, "LR untouched (no LK on fused bc)");
    assert_eq!(s.ctr, ctr_before, "CTR untouched (BO_2=1, no decrement)");
    assert_eq!(s.cr_field(0), 0b1000, "CR field updated by cmpwi");
}

#[test]
fn cmpw_bc_taken_with_ctr_decrement_does_decrement_ctr() {
    // BO=0x10 (skip-CR, decrement-CTR, branch-if-CTR!=0). Forced
    // taken via initial CTR=2 -> 1, non-zero. PC advances to the
    // bc target and CTR is decremented.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.gpr[3] = 7;
    s.gpr[4] = 7;
    s.ctr = 2;
    let v = exec_no_mem(
        &PpuInstruction::CmpwBc {
            bf: 0,
            ra: 3,
            rb: 4,
            bo: 0x10, // BO_0=1 skip CR test; BO_2=0 decrement CTR; BO_3=0 branch on CTR!=0
            bi: 0,
            target_offset: 32,
        },
        &mut s,
    );
    assert_eq!(v, ExecuteVerdict::Branch);
    assert_eq!(s.ctr, 1, "CTR decremented by 1");
    // Target = (pc + 4) + target_offset = 0x2004 + 0x20 = 0x2024.
    assert_eq!(s.pc, 0x2024, "PC advanced to bc target");
}

#[test]
fn stdstd_no_partial_commit_on_buffer_full() {
    // When the buffer is one slot short of capacity, the first
    // std lands but the second hits BufferFull. The first store
    // is staged in the buffer (architectural; the retry path
    // replays the whole super-insn from EA1) but no second store
    // is staged, and the verdict is BufferFull.
    use crate::exec::test_support::uid;
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[5] = 0x1111_1111_1111_1111;
    s.gpr[6] = 0x2222_2222_2222_2222;
    // Pre-fill the buffer to capacity - 1.
    let mut store_buf = StoreBuffer::new();
    for i in 0..63 {
        assert!(store_buf.insert(0x4000 + (i as u64) * 16, 1, 0));
    }
    let mut effects = Vec::new();
    let v = crate::exec::execute(
        &PpuInstruction::StdStd {
            rs1: 5,
            rs2: 6,
            ra: 1,
            offset1: 0,
        },
        &mut s,
        uid(),
        &[(0u64, &[0u8; 0x2000][..])],
        &mut effects,
        &mut store_buf,
    );
    // First store fills the last slot, second store cannot stage.
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        store_buf.len(),
        64,
        "first store staged, second hit full buffer"
    );
}
