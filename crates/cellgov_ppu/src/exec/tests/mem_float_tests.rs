//! Float loads/stores: single/double conversion, NaN payload preservation, and write-back gating.

use super::*;

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
fn lfsx_loads_single_and_round_converts_to_double() {
    // 1.5f as float bits is 0x3FC00000; verify the FPR holds the
    // double bit pattern of 1.5 (0x3FF8000000000000).
    let mut mem = vec![0u8; 0x100];
    mem[0x40..0x44].copy_from_slice(&0x3FC0_0000u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[4] = 0x40;
    s.gpr[5] = 0;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfsx {
            frt: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.fpr[7], 0x3FF8_0000_0000_0000);
}

#[test]
fn lfsux_writes_back_ea_to_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x44..0x48].copy_from_slice(&0x4040_0000u32.to_be_bytes()); // 3.0f
    let mut s = PpuState::new();
    s.gpr[4] = 0x40;
    s.gpr[5] = 4;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfsux {
            frt: 8,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0x44);
    // 3.0 as double: 0x4008000000000000
    assert_eq!(s.fpr[8], 0x4008_0000_0000_0000);
}

#[test]
fn lfdx_loads_64_bit_double() {
    let mut mem = vec![0u8; 0x100];
    let bits = 0x4080_1122_3344_5566u64;
    mem[0x10..0x18].copy_from_slice(&bits.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[2] = 0x10;
    s.gpr[3] = 0;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfdx {
            frt: 9,
            ra: 2,
            rb: 3,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.fpr[9], bits);
}

#[test]
fn lfdux_writes_back_ea_to_ra() {
    let mut mem = vec![0u8; 0x100];
    let bits = 0x4090_AAAA_BBBB_CCCCu64;
    mem[0x20..0x28].copy_from_slice(&bits.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[2] = 0x10;
    s.gpr[3] = 0x10;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfdux {
            frt: 10,
            ra: 2,
            rb: 3,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[2], 0x20);
    assert_eq!(s.fpr[10], bits);
}

#[test]
fn stfsx_stores_round_converted_single() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x100;
    s.gpr[5] = 0x4;
    // 1.5 as double; round-convert to single bit pattern is 0x3FC00000.
    s.fpr[6] = 0x3FF8_0000_0000_0000;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfsx {
            frs: 6,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x104);
            assert_eq!(bytes.bytes(), &0x3FC0_0000u32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stfsux_writes_back_ea_only_on_success() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x40;
    s.gpr[5] = 0x4;
    s.fpr[3] = 0x4040_0000_0000_0000; // 32.0 as double
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfsux {
            frs: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0x44);
    assert_eq!(effects.len(), 1);
}

#[test]
fn stfdx_stores_64_bit_double_verbatim() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x80;
    s.gpr[5] = 0x10;
    let bits = 0xC020_FFFF_0000_1111u64;
    s.fpr[2] = bits;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfdx {
            frs: 2,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x90);
            assert_eq!(bytes.bytes(), &bits.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn lfsx_preserves_nan_payload_bit_for_bit() {
    // SNaN single (high frac bit clear, payload non-zero):
    // 0x7F801234. Spec says lfsx delivers WORD0:1 + WORD2:31||0^29
    // into FRT, leaving the SNaN/QNaN distinction untouched.
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x14].copy_from_slice(&0x7F80_1234u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[3] = 0x10;
    s.gpr[4] = 0;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfsx {
            frt: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    // Expected: sign=0, exp=0x7FF, frac52 = 0x001234 << 29.
    let expected = (0x7FFu64 << 52) | (0x001234u64 << 29);
    assert_eq!(s.fpr[5], expected);
}

#[test]
fn stfsx_preserves_nan_payload_bit_for_bit() {
    // FRS = double-encoded NaN with sign=1, exp=0x7FF,
    // frac52 = 0xABCDE_DEADBEEF (low 29 bits will be discarded
    // by the spec's WORD2:31 <- FRS5:34 selection). Expect WORD
    // = sign=1, exp=0xFF, frac23 = top 23 bits of frac52.
    let mut s = PpuState::new();
    s.gpr[4] = 0x80;
    s.gpr[5] = 0;
    let frac52: u64 = 0x000A_BCDE_DEAD_BEEF;
    let nan_d = (1u64 << 63) | (0x7FFu64 << 52) | frac52;
    s.fpr[6] = nan_d;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfsx {
            frs: 6,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    let frac23 = ((frac52 >> 29) & 0x007F_FFFF) as u32;
    let expected = (1u32 << 31) | (0xFFu32 << 23) | frac23;
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x80);
            assert_eq!(bytes.bytes(), &expected.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stfsx_then_lfsx_round_trips_nan_payload() {
    // Round-trip pin: a NaN whose 23 high fraction bits are
    // distinct survives stfsx -> lfsx with the same single bit
    // pattern, after re-expansion in lfsx into double form.
    let frac23: u32 = 0x004A_5A5A;
    let single_nan = (1u32 << 31) | (0xFFu32 << 23) | frac23;
    // Set up FPR with the canonical lfsx-of-this-single result.
    let canonical_fpr = (1u64 << 63) | (0x7FFu64 << 52) | ((frac23 as u64) << 29);

    // stfsx round.
    let mut s = PpuState::new();
    s.gpr[4] = 0x80;
    s.gpr[5] = 0;
    s.fpr[7] = canonical_fpr;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfsx {
            frs: 7,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    let stored = match &effects[0] {
        Effect::SharedWriteIntent { bytes, .. } => {
            u32::from_be_bytes(bytes.bytes().try_into().unwrap())
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    };
    assert_eq!(stored, single_nan, "stfsx must preserve NaN bit pattern");

    // lfsx round.
    let mut mem = vec![0u8; 0x100];
    mem[0x40..0x44].copy_from_slice(&single_nan.to_be_bytes());
    let mut s2 = PpuState::new();
    s2.gpr[3] = 0x40;
    s2.gpr[4] = 0;
    let mut effects2 = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfsx {
            frt: 8,
            ra: 3,
            rb: 4,
        },
        &mut s2,
        0,
        &mem,
        &mut effects2,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(
        s2.fpr[8], canonical_fpr,
        "lfsx-of-NaN must rebuild the spec FPR pattern bit-for-bit"
    );
}

#[test]
fn lfsux_load_fault_does_not_write_ra() {
    // EA out of mapped region: load_ze returns Err(ea), the
    // handler emits MemFault, and RA must stay at its prior
    // value. A naive implementation that writes RA before
    // checking the load result would break the on-success-only
    // discipline.
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[4] = 0x1000_0000; // far outside the 0x100-byte region
    s.gpr[5] = 0;
    let original_ra = s.gpr[4];
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfsux {
            frt: 9,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(matches!(out, ExecuteVerdict::MemFault(_)));
    assert_eq!(s.gpr[4], original_ra);
}

#[test]
fn stfsux_buffer_full_does_not_write_ra() {
    // Pre-fill the store buffer to capacity, then dispatch an
    // stfsux. buffer_store should return BufferFull; RA must
    // remain at its prior value so the retry-after-flush sees
    // the same architectural state.
    use crate::store_buffer::StoreBuffer;
    let mut store_buf = StoreBuffer::new();
    for i in 0..64 {
        assert!(store_buf.insert(0x1000 + i * 4, 4, i as u128));
    }
    assert!(store_buf.is_full());
    let mut s = PpuState::new();
    s.gpr[4] = 0x80;
    s.gpr[5] = 0x10;
    s.fpr[3] = 0x4040_0000_0000_0000;
    let original_ra = s.gpr[4];
    let mem = [0u8; 0x200];
    let views: [(u64, &[u8]); 1] = [(0, &mem)];
    let mut effects = Vec::new();
    let out = crate::exec::execute(
        &PpuInstruction::Stfsux {
            frs: 3,
            ra: 4,
            rb: 5,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(out, ExecuteVerdict::BufferFull);
    assert_eq!(s.gpr[4], original_ra);
}

#[test]
fn stfdux_buffer_full_does_not_write_ra() {
    use crate::store_buffer::StoreBuffer;
    let mut store_buf = StoreBuffer::new();
    for i in 0..64 {
        assert!(store_buf.insert(0x2000 + i * 4, 4, i as u128));
    }
    let mut s = PpuState::new();
    s.gpr[4] = 0x60;
    s.gpr[5] = 0x8;
    s.fpr[2] = 0xDEAD_BEEF_CAFE_BABE;
    let original_ra = s.gpr[4];
    let mem = [0u8; 0x200];
    let views: [(u64, &[u8]); 1] = [(0, &mem)];
    let mut effects = Vec::new();
    let out = crate::exec::execute(
        &PpuInstruction::Stfdux {
            frs: 2,
            ra: 4,
            rb: 5,
        },
        &mut s,
        uid(),
        &views,
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(out, ExecuteVerdict::BufferFull);
    assert_eq!(s.gpr[4], original_ra);
}

#[test]
fn lfdux_load_fault_does_not_write_ra() {
    let mem = vec![0u8; 0x100];
    let mut s = PpuState::new();
    s.gpr[2] = 0x2000_0000;
    s.gpr[3] = 0;
    let original_ra = s.gpr[2];
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Lfdux {
            frt: 10,
            ra: 2,
            rb: 3,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert!(matches!(out, ExecuteVerdict::MemFault(_)));
    assert_eq!(s.gpr[2], original_ra);
}

#[test]
fn stfdux_writes_back_ea_to_ra() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x60;
    s.gpr[5] = 0x8;
    s.fpr[2] = 0xDEAD_BEEF_CAFE_BABE;
    let mut effects = Vec::new();
    let out = exec_with_mem(
        &PpuInstruction::Stfdux {
            frs: 2,
            ra: 4,
            rb: 5,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(out, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[4], 0x68);
    assert_eq!(effects.len(), 1);
}

#[test]
fn stfsu_buffer_full_does_not_update_ra() {
    // Mirrors the integer Stwu contract: when buffer_store returns
    // BufferFull the update of RA must be skipped, so the caller can
    // retry after flushing without double-advancing the base pointer.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.fpr[5] = (1.5f32 as f64).to_bits();
    let mut store_buf = StoreBuffer::new();
    // Fill the buffer to capacity. CAPACITY is private; saturate by
    // inserting until insert reports `is_full`.
    while !store_buf.is_full() {
        assert!(store_buf.insert(0, 1, 0));
    }
    let mut effects = Vec::new();
    let v = execute(
        &PpuInstruction::Stfsu {
            frs: 5,
            ra: 1,
            imm: 0x40,
        },
        &mut s,
        uid(),
        &[],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        s.gpr[1], 0x1000,
        "RA must be unchanged when buffer_store returns BufferFull"
    );
}

#[test]
fn stfdu_buffer_full_does_not_update_ra() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x2000;
    s.fpr[6] = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut store_buf = StoreBuffer::new();
    while !store_buf.is_full() {
        assert!(store_buf.insert(0, 1, 0));
    }
    let mut effects = Vec::new();
    let v = execute(
        &PpuInstruction::Stfdu {
            frs: 6,
            ra: 1,
            imm: 0x80,
        },
        &mut s,
        uid(),
        &[],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        s.gpr[1], 0x2000,
        "RA must be unchanged when buffer_store returns BufferFull"
    );
}

// -----------------------------------------------------------------
// Floating-point D-form loads / stores
// -----------------------------------------------------------------

#[test]
fn lfs_loads_single_and_converts_to_double() {
    // 2.0f single = 0x40000000 -> 2.0 double = 0x4000_0000_0000_0000.
    let mut mem = vec![0u8; 0x100];
    mem[0x10..0x14].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfs {
            frt: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.fpr[5], 0x4000_0000_0000_0000);
}

#[test]
fn lfsu_loads_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    mem[0x14..0x18].copy_from_slice(&0x4040_0000u32.to_be_bytes()); // 3.0f
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfsu {
            frt: 6,
            ra: 1,
            imm: 4,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.fpr[6], 0x4008_0000_0000_0000);
    assert_eq!(s.gpr[1], 0x14);
}

#[test]
fn lfd_loads_8_byte_double_big_endian() {
    let mut mem = vec![0u8; 0x100];
    let bits = 0x4010_2030_4050_6070u64;
    mem[0x10..0x18].copy_from_slice(&bits.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfd {
            frt: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.fpr[5], bits);
}

#[test]
fn lfdu_loads_double_and_writes_back_ra() {
    let mut mem = vec![0u8; 0x100];
    let bits = 0xDEAD_BEEF_CAFE_BABEu64;
    mem[0x18..0x20].copy_from_slice(&bits.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfdu {
            frt: 5,
            ra: 1,
            imm: 8,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.fpr[5], bits);
    assert_eq!(s.gpr[1], 0x18);
}

#[test]
fn stfs_round_converts_double_to_single() {
    // 1.5 double = 0x3FF8_0000_0000_0000 -> 1.5f single = 0x3FC00000.
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.fpr[5] = 0x3FF8_0000_0000_0000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stfs {
            frs: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(bytes.bytes(), &0x3FC0_0000u32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn lfd_preserves_nan_payload_8_bytes_verbatim() {
    // lfd is byte-for-byte; SNaN double pattern survives intact.
    let mut mem = vec![0u8; 0x100];
    let snan = 0x7FF0_0000_0000_0001u64;
    mem[0x10..0x18].copy_from_slice(&snan.to_be_bytes());
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfd {
            frt: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.fpr[5], snan);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "lfsu invalid form")]
fn lfsu_with_ra_zero_panics_in_debug() {
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfsu {
            frt: 5,
            ra: 0,
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
#[should_panic(expected = "lfdu invalid form")]
fn lfdu_with_ra_zero_panics_in_debug() {
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lfdu {
            frt: 5,
            ra: 0,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x100],
        &mut effects,
    );
}
