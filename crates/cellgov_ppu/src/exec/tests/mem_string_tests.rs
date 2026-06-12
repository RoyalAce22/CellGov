//! String loads/stores (lswi/lswx/stswi/stswx) and byte-reversed accesses.

use super::*;

#[test]
fn lswi_packs_bytes_four_per_register_msb_first() {
    // 5 bytes from offset 0x10 -> RT=3, fills r3 fully + r4 partially.
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x15].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lswi {
            rt: 3,
            ra: 1,
            nb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[3], 0xAABB_CCDDu64);
    assert_eq!(s.gpr[4], 0xEE00_0000u64);
}

#[test]
fn lswi_nb_zero_means_32_bytes_and_wraps_at_r31() {
    // RT=30 with NB=0 -> 32 bytes -> r30, r31, r0, r1 (wraps).
    let mut mem = vec![0u8; 0x100];
    for (i, slot) in mem.iter_mut().take(32).enumerate() {
        *slot = i as u8;
    }
    let mut s = PpuState::new();
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Lswi {
            rt: 30,
            ra: 5,
            nb: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    // Bytes 0..3 -> r30, 4..7 -> r31, 8..11 -> r0, 12..15 -> r1, ...
    assert_eq!(s.gpr[30], 0x00010203);
    assert_eq!(s.gpr[31], 0x04050607);
    assert_eq!(s.gpr[0], 0x08090A0B);
    assert_eq!(s.gpr[1], 0x0C0D0E0F);
}

#[test]
fn stswi_extracts_bytes_msb_first_from_consecutive_registers() {
    // 5 bytes from RS=3 (4 bytes from r3, 1 byte from r4 high).
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[1] = 0x20;
    s.gpr[3] = 0xAABB_CCDDu64;
    s.gpr[4] = 0xEE00_0000u64;
    let mut effects = Vec::new();
    let result = exec_with_mem(
        &PpuInstruction::Stswi {
            rs: 3,
            ra: 1,
            nb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(result, ExecuteVerdict::Continue);
    // Inspect the 5 SharedWriteIntent effects via their inline bytes.
    let writes: Vec<(u64, u8)> = effects
        .iter()
        .filter_map(|e| match e {
            cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } => {
                Some((range.start().raw(), bytes.bytes()[0]))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        writes,
        vec![
            (0x20, 0xAA),
            (0x21, 0xBB),
            (0x22, 0xCC),
            (0x23, 0xDD),
            (0x24, 0xEE),
        ]
    );
}

// -----------------------------------------------------------------
// String load / store
// -----------------------------------------------------------------

#[test]
fn lswx_uses_xer_tbc_byte_count() {
    let mut mem = vec![0u8; 0x100];
    mem[0x20..0x23].copy_from_slice(&[0x11, 0x22, 0x33]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    s.xer = 0x3; // TBC=3 bytes
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lswx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    // 3 bytes packed MSB-first into the low 32 bits of r3.
    assert_eq!(s.gpr[3], 0x1122_3300);
}

#[test]
fn lswx_zero_byte_count_is_noop() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    s.gpr[3] = 0xDEAD_BEEF;
    s.xer = 0;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Lswx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    // No write: r3 untouched.
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn stswx_uses_xer_tbc_byte_count() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x10;
    s.gpr[3] = 0xAABB_CCDDu64;
    s.xer = 0x2;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stswx {
            rs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
    let writes: Vec<(u64, u8)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                Some((range.start().raw(), bytes.bytes()[0]))
            }
            _ => None,
        })
        .collect();
    assert_eq!(writes, vec![(0x20, 0xAA), (0x21, 0xBB)]);
}

#[test]
fn stswi_nb_zero_means_32_bytes() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    for r in 0..8u32 {
        s.gpr[3 + r as usize] = (r as u64) * 0x0101_0101_0101_0101 + 0x1020_3040;
    }
    // To make the assertion simple, seed r3..r10 with known 32-bit words.
    for r in 0..8usize {
        s.gpr[3 + r] = (0xA0 + r as u64) << 24 | ((0xB0 + r as u64) << 16);
    }
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stswi {
            rs: 3,
            ra: 1,
            nb: 0,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
    // NB=0 -> 32 bytes -> exactly 32 byte-stores.
    let count = effects
        .iter()
        .filter(|e| matches!(e, Effect::SharedWriteIntent { .. }))
        .count();
    assert_eq!(count, 32);
}

#[test]
fn lswi_with_ra_zero_uses_literal_zero_base() {
    let mut mem = vec![0u8; 0x20];
    mem[0..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    let mut s = PpuState::new();
    s.gpr[0] = 0xDEAD; // must be ignored
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lswi {
            rt: 3,
            ra: 0,
            nb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x0102_0304);
}

// -----------------------------------------------------------------
// Byte-reverse loads / stores
// -----------------------------------------------------------------

#[test]
fn ldbrx_reverses_8_bytes() {
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x18].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Ldbrx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    // Reading BE -> 0x0102030405060708, swap_bytes -> 0x0807060504030201.
    assert_eq!(s.gpr[3], 0x0807_0605_0403_0201);
}

#[test]
fn lwbrx_reverses_low_4_bytes_and_zero_extends() {
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x14].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0;
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lwbrx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x0000_0000_DDCC_BBAA);
}

#[test]
fn lhbrx_reverses_halfword_and_zero_extends() {
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x12].copy_from_slice(&[0x12, 0x34]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0;
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lhbrx {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.gpr[3], 0x0000_0000_0000_3412);
}

#[test]
fn sdbrx_emits_byte_reversed_8_byte_store() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    s.gpr[5] = 0x0102_0304_0506_0708;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Sdbrx {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(bytes.bytes(), &0x0807_0605_0403_0201u64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stwbrx_emits_byte_reversed_low_4_bytes() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    s.gpr[5] = 0xFFFF_FFFF_AABB_CCDD;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwbrx {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(bytes.bytes(), &0xDDCC_BBAAu32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn sthbrx_emits_byte_reversed_low_halfword() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    s.gpr[5] = 0xFFFF_FFFF_FFFF_1234;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Sthbrx {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(bytes.bytes(), &0x3412u16.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}
