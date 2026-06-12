//! Fixed-point loads: D/DS/X-form, update forms, sign extension, lmw, and fault paths.

use super::*;

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
    assert!(matches!(
        result,
        ExecuteVerdict::MemFault(cellgov_mem::MemError::Unmapped(ctx)) if ctx.addr == 0x1008
    ));
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
fn lwa_sign_extends_word_into_64_bits() {
    // 0xFFFF_FFFE = -2 as i32; lwa must sign-extend to the full
    // 64-bit GPR. Reading this as lwz (zero-extend) would give
    // 0x0000_0000_FFFF_FFFE instead.
    let mut mem = vec![0u8; 0x2000];
    mem[0x1004..0x1008].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lwa {
            rt: 3,
            ra: 1,
            imm: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFE);
    assert_eq!(s.gpr[3] as i64, -2);
}

#[test]
fn lwa_sign_extends_through_store_buffer_forward() {
    // stw then lwa from same EA: forwards through StoreBuffer,
    // exercises size-aware sign extension (sub-8-byte forwards
    // leave high u64 bits zero, so naive i64 cast would mis-sign).
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[5] = 0xFFFF_FFFE;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let region_views: [(u64, &[u8]); 1] = [(0, &[0u8; 0x2000])];
    let v_stw = execute(
        &PpuInstruction::Stw {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        UnitId::new(0),
        &region_views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v_stw, ExecuteVerdict::Continue);
    let v_lwa = execute(
        &PpuInstruction::Lwa {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        UnitId::new(0),
        &region_views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v_lwa, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFE);
    assert_eq!(s.gpr[3] as i64, -2);
}

#[test]
fn lha_sign_extends_through_store_buffer_forward() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[5] = 0xFF80;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let region_views: [(u64, &[u8]); 1] = [(0, &[0u8; 0x2000])];
    let v_sth = execute(
        &PpuInstruction::Sth {
            rs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        UnitId::new(0),
        &region_views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v_sth, ExecuteVerdict::Continue);
    let v_lha = execute(
        &PpuInstruction::Lha {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        UnitId::new(0),
        &region_views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v_lha, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3] as i64, -128);
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

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "lwzu invalid form")]
fn lwzu_with_ra_zero_panics_in_debug() {
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwzu {
            rt: 3,
            ra: 0, // invalid: RA=0 has no base register to update
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "lwzu invalid form")]
fn lwzu_with_ra_eq_rt_panics_in_debug() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwzu {
            rt: 3,
            ra: 3, // invalid: EA-write to RA would clobber RT
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
}

// -----------------------------------------------------------------
// Integer D-form loads
// -----------------------------------------------------------------

#[test]
fn lbz_loads_byte_zero_extended() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20] = 0xA5;
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lbz {
            rt: 3,
            ra: 1,
            imm: 0x10,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xA5);
}

#[test]
fn lhz_loads_halfword_zero_extended_big_endian() {
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x12].copy_from_slice(&[0xBE, 0xEF]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhz {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xBEEF);
}

#[test]
fn ld_loads_doubleword_big_endian() {
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x18].copy_from_slice(&0x0123_4567_89AB_CDEFu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ld {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x0123_4567_89AB_CDEF);
}

#[test]
fn lhau_sign_extends_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x12..0x14].copy_from_slice(&0x8000u16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhau {
            rt: 3,
            ra: 1,
            imm: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3] as i64, i16::MIN as i64);
    assert_eq!(s.gpr[1], 0x12);
}

#[test]
fn lwzu_loads_word_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x14..0x18].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwzu {
            rt: 3,
            ra: 1,
            imm: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    assert_eq!(s.gpr[1], 0x14);
}

#[test]
fn lbzu_loads_byte_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x11] = 0x7E;
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lbzu {
            rt: 3,
            ra: 1,
            imm: 1,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x7E);
    assert_eq!(s.gpr[1], 0x11);
}

#[test]
fn lmw_loads_consecutive_words_until_r31() {
    let mut mem = vec![0u8; 0x100];
    for r in 0..3u32 {
        let off = 0x10 + (r as usize) * 4;
        mem[off..off + 4].copy_from_slice(&(0xAABB_0000u32 + r).to_be_bytes());
    }
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    // RT=29 -> loads r29, r30, r31 (three words).
    let v = exec_with_mem(
        &PpuInstruction::Lmw {
            rt: 29,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[29], 0xAABB_0000);
    assert_eq!(s.gpr[30], 0xAABB_0001);
    assert_eq!(s.gpr[31], 0xAABB_0002);
}

#[test]
fn lfault_lhau_does_not_update_ra() {
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000_0000;
    let original = s.gpr[1];
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lhau {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(matches!(v, ExecuteVerdict::MemFault(_)));
    assert_eq!(s.gpr[1], original);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "lhau invalid form")]
fn lhau_with_ra_zero_panics_in_debug() {
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhau {
            rt: 3,
            ra: 0,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
}

// -----------------------------------------------------------------
// Integer X-form loads
// -----------------------------------------------------------------

#[test]
fn lwzx_loads_word_at_ra_plus_rb() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x24].copy_from_slice(&0xCAFE_BABEu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwzx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xCAFE_BABE);
}

#[test]
fn lbzx_loads_byte_zero_extended() {
    let mut mem = vec![0u8; 0x100];
    mem[0x30] = 0x42;
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x20;
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lbzx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x42);
}

#[test]
fn lhzx_loads_halfword_zero_extended() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x22].copy_from_slice(&0xABCDu16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhzx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xABCD);
}

#[test]
fn ldx_loads_doubleword_with_ra_zero_ignored() {
    // ea_x_form with RA=0 uses literal 0, not gpr[0].
    let mut mem = vec![0u8; 0x100];
    mem[0x18..0x20].copy_from_slice(&0xCAFE_F00D_DEAD_BEEFu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[0] = 0xDEAD; // must be ignored
    s.gpr[2] = 0x18;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldx {
            rt: 3,
            ra: 0,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xCAFE_F00D_DEAD_BEEF);
}

#[test]
fn lhax_sign_extends_halfword() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x22].copy_from_slice(&0xFF80u16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhax {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3] as i64, -128);
}

#[test]
fn lwax_sign_extends_word() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x24].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwax {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3] as i64, -2);
}

#[test]
fn lwzux_loads_and_writes_back_ra_only_on_success() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x24].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwzux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
    assert_eq!(s.gpr[4], 0x20);
}

#[test]
fn lwzux_fault_leaves_ra_unchanged() {
    let mem = vec![0u8; 0x40];
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000_0000;
    s.gpr[5] = 0;
    let original = s.gpr[4];
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lwzux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(matches!(v, ExecuteVerdict::MemFault(_)));
    assert_eq!(s.gpr[4], original);
}

#[test]
fn lbzux_loads_byte_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x21] = 0x99;
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x11;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lbzux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x99);
    assert_eq!(s.gpr[4], 0x21);
}

#[test]
fn lhzux_loads_halfword_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x22].copy_from_slice(&0xC0DEu16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhzux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xC0DE);
    assert_eq!(s.gpr[4], 0x20);
}

#[test]
fn ldux_loads_doubleword_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x18..0x20].copy_from_slice(&0xDEAD_BEEF_CAFE_BABEu64.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x8;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
    assert_eq!(s.gpr[4], 0x18);
}

#[test]
fn lhaux_sign_extends_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x22].copy_from_slice(&0xFFFFu16.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhaux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3] as i64, -1);
    assert_eq!(s.gpr[4], 0x20);
}

#[test]
fn lwaux_sign_extends_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x24].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x10;
    s.gpr[5] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwaux {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_8000_0000);
    assert_eq!(s.gpr[4], 0x20);
}
