//! Cross-module exec tests: equivalence between predecoded shadow
//! variants (quickenings and super-pairs) and the ISA-native sequences
//! they replace, plus the lone Sc dispatch that lives in exec.rs
//! itself. Per-module unit tests live next to their implementation in
//! `exec/<module>.rs::tests`.

use super::*;
use crate::exec::test_support::{exec_no_mem, exec_with_mem, uid};

#[test]
fn sc_returns_syscall() {
    let mut s = PpuState::new();
    let result = exec_no_mem(&PpuInstruction::Sc { lev: 0 }, &mut s);
    assert!(matches!(result, ExecuteVerdict::Syscall));
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
            aa: false,
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
            aa: false,
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
fn ld_stitches_eight_byte_stbs_in_store_buffer() {
    // Regression for PSL1GHT printf bug: _Litob writes the 8 ASCII
    // hex digits to a stack buffer with 8 individual `stb`s, then
    // `bl memcpy` reads them back with `ld`. A `forward()` that
    // requires single-entry full coverage misses the load and falls
    // through to pre-block memory (zeros) -- corrupting every printf
    // that produces an 8-digit integer.
    use crate::store_buffer::StoreBuffer;
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mem = vec![0u8; 0x100];
    let views: [(u64, &[u8]); 1] = [(0x1000, &mem)];
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();

    let bytes = [b'1', b'0', b'0', b'0', b'0', b'0', b'0', b'0'];
    for (i, b) in bytes.iter().enumerate() {
        s.gpr[3] = *b as u64;
        let v = execute(
            &PpuInstruction::Stb {
                rs: 3,
                ra: 1,
                imm: i as i16,
            },
            &mut s,
            uid(),
            &views,
            &mut effects,
            &mut store_buf,
        );
        assert_eq!(v, ExecuteVerdict::Continue, "stb #{i}");
    }
    assert_eq!(store_buf.len(), 8);

    let v = execute(
        &PpuInstruction::Ld {
            rt: 4,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0x3130303030303030);
}

#[test]
fn ld_overlays_partial_store_onto_pre_block_memory() {
    // A buffered store covers bytes [4, 8) of an 8-byte load; the
    // other 4 bytes come from committed memory. Stitching must mix
    // the two sources, not pick one.
    use crate::store_buffer::StoreBuffer;
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    let mut mem = vec![0u8; 0x100];
    mem[0..8].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]);
    let views: [(u64, &[u8]); 1] = [(0x1000, &mem)];
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();

    s.gpr[3] = 0xDEAD_BEEF;
    let v = execute(
        &PpuInstruction::Stw {
            rs: 3,
            ra: 1,
            imm: 4,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);

    let v = execute(
        &PpuInstruction::Ld {
            rt: 4,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0xAABB_CCDD_DEAD_BEEF);
}
