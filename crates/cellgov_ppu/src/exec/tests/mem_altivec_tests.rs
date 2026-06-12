//! AltiVec loads/stores: alignment, lane placement, shift permutes, and LVLX/LVRX edges.

use super::*;

// Helper used by the lvsl test above.
fn exec_no_mem_or_load<F>(s: &mut PpuState, e: &mut Vec<Effect>, f: F)
where
    F: FnOnce(&mut PpuState, &mut Vec<Effect>) -> ExecuteVerdict,
{
    let v = f(s, e);
    assert_eq!(v, ExecuteVerdict::Continue);
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
    // stvx forces EA to 16-byte alignment: 0x1000+0x1F -> 0x1010, then
    // commits as two 8-byte halves so buffer_store's reservation
    // clear-sweep covers both.
    assert_eq!(effects.len(), 2);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x1010);
            assert_eq!(range.length(), 8);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    match &effects[1] {
        Effect::SharedWriteIntent { range, .. } => {
            assert_eq!(range.start().raw(), 0x1018);
            assert_eq!(range.length(), 8);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
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
fn stvx_pre_checks_capacity_for_both_halves() {
    // Pre-fill the buffer to 63 entries so only one slot remains
    // -- not enough for stvx's two halves. Without the
    // pre-check, the first half would commit and the retry
    // would duplicate it.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[2] = 0;
    s.vr[3] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    for i in 0..63 {
        assert!(store_buf.insert((i as u64) * 8, 8, 0));
    }
    let v = execute(
        &PpuInstruction::Stvx {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        UnitId::new(0),
        &[(0, &[0u8; 0x2000])],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        store_buf.len(),
        63,
        "no partial commit when capacity is insufficient"
    );
}

#[test]
fn lvlx_partial_overlap_merges_buffered_bytes_with_region() {
    // Pre-stage a 4-byte store at offset +4 within the line.
    // forward(aligned, 16) returns None (no full match). The
    // load reads the 16-byte line from regions and overlays the
    // 4 buffered bytes byte-by-byte instead of yielding.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    assert!(store_buf.insert(0x1004, 4, 0xDEAD_BEEFu128));
    let mut mem = vec![0u8; 0x2000];
    for i in 0..16 {
        mem[0x1000 + i] = 0x10 + i as u8;
    }
    let v = execute(
        &PpuInstruction::Lvlx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        UnitId::new(0),
        &[(0, &mem)],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    // Lvlx with aligned EA shifts by 0, so the result is the
    // raw 16 bytes. Bytes 4..8 should be patched to DEADBEEF.
    let expected = u128::from_be_bytes([
        0x10, 0x11, 0x12, 0x13, // unchanged
        0xDE, 0xAD, 0xBE, 0xEF, // patched
        0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
    ]);
    assert_eq!(s.vr[3], expected);
}

#[test]
fn lvlx_full_overlap_forwards_without_yielding() {
    // Sanity: when the buffer covers the full 16-byte line, the
    // load proceeds without yielding.
    let mut s = PpuState::new();
    s.gpr[1] = 0x1000;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    let val = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899u128;
    assert!(store_buf.insert(0x1000, 16, val));
    let v = execute(
        &PpuInstruction::Lvlx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        UnitId::new(0),
        &[(0, &[0u8; 0x2000])],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert_eq!(s.vr[3], val);
}

// -----------------------------------------------------------------
// AltiVec aligned vector loads
// -----------------------------------------------------------------

#[test]
fn lvx_aligns_ea_down_to_16_byte_boundary() {
    let mut mem = vec![0u8; 0x200];
    let pattern: [u8; 16] = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ];
    mem[0x100..0x110].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x0F; // EA = 0x10F -> aligned 0x100
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[3], u128::from_be_bytes(pattern));
}

#[test]
fn lvxl_matches_lvx_semantics() {
    // lvxl is lvx with an ignored cache hint; same bytes -> same VR.
    let mut mem = vec![0u8; 0x200];
    let pattern: [u8; 16] = [
        0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E,
        0x2F,
    ];
    mem[0x100..0x110].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvxl {
            vt: 4,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[4], u128::from_be_bytes(pattern));
}

#[test]
fn lvsl_sh_zero_returns_identity_vector() {
    // sh=0 -> VRT = [0, 1, 2, ..., 15].
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    exec_no_mem_or_load(&mut s, &mut effects, |s, e| {
        exec_with_mem(
            &PpuInstruction::Lvsl {
                vt: 3,
                ra: 1,
                rb: 2,
            },
            s,
            0,
            &[0u8; 0x10],
            e,
        )
    });
    let expected_bytes: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    assert_eq!(s.vr[3], u128::from_be_bytes(expected_bytes));
}

#[test]
fn lvsl_sh_nonzero_returns_shifted_identity() {
    // EA & 0xF = 3 -> VRT = [3, 4, 5, ..., 18].
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x3;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvsl {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x10],
        &mut effects,
    );
    let mut expected = [0u8; 16];
    for (i, b) in expected.iter_mut().enumerate() {
        *b = 3 + i as u8;
    }
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvsr_sh_zero_returns_descending_from_16() {
    // sh=0 -> VRT = [16, 17, ..., 31] (wraps low bits of u8).
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvsr {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x10],
        &mut effects,
    );
    let mut expected = [0u8; 16];
    for (i, b) in expected.iter_mut().enumerate() {
        *b = 16u8.wrapping_add(i as u8);
    }
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvsr_sh_three_returns_companion_to_lvsl() {
    // sh=3 -> VRT[i] = 16 + i - 3 = 13 + i, for i in 0..16.
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x3;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvsr {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x10],
        &mut effects,
    );
    let mut expected = [0u8; 16];
    for (i, b) in expected.iter_mut().enumerate() {
        *b = 13u8.wrapping_add(i as u8);
    }
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvebx_places_byte_in_be_lane_from_ea_low_nibble() {
    // EA & 0xF = 5 -> byte lands at byte[5] of the 16-byte BE view.
    let mut mem = vec![0u8; 0x100];
    mem[0x15] = 0x7E;
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x05;
    // Pre-seed VR with a sentinel so we can verify other lanes are
    // preserved (spec-undefined but our implementation preserves).
    s.vr[3] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAAu128;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvebx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    let mut expected = [0xAAu8; 16];
    expected[5] = 0x7E;
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvehx_places_halfword_in_aligned_be_lane() {
    // EA = 0x14 -> after &!1 still 0x14; lane = 0x4.
    let mut mem = vec![0u8; 0x100];
    mem[0x14..0x16].copy_from_slice(&[0xBE, 0xEF]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x05; // EA = 0x15 -> aligned to 0x14
    s.vr[3] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvehx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    let mut expected = [0u8; 16];
    expected[4] = 0xBE;
    expected[5] = 0xEF;
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvewx_places_word_in_aligned_be_lane() {
    // EA = 0x18 -> &!3 still 0x18; lane = 0x8.
    let mut mem = vec![0u8; 0x100];
    mem[0x18..0x1C].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x10;
    s.gpr[2] = 0x09; // EA = 0x19 -> aligned 0x18
    s.vr[3] = 0;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvewx {
            vt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    let mut expected = [0u8; 16];
    expected[8] = 0xDE;
    expected[9] = 0xAD;
    expected[10] = 0xBE;
    expected[11] = 0xEF;
    assert_eq!(s.vr[3], u128::from_be_bytes(expected));
}

#[test]
fn lvlxl_matches_lvlx_semantics() {
    let mut mem = vec![0u8; 0x200];
    let pattern: [u8; 16] = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        0x10,
    ];
    mem[0x100..0x110].copy_from_slice(&pattern);
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x3;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvlxl {
            vt: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[5], u128::from_be_bytes(pattern) << 24);
}

#[test]
fn lvrxl_aligned_ea_returns_zero() {
    // Mirrors lvrx aligned: shift by 128 produces zero. NOTE the
    // implementation still issues the underlying line read before
    // computing the zero result; this test pins the architectural
    // outcome, not the buffer-side memoization shape.
    let mut mem = vec![0u8; 0x200];
    mem[0x100..0x110].copy_from_slice(&[0xFF; 16]);
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    s.vr[5] = u128::MAX;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Lvrxl {
            vt: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &mem,
        &mut effects,
    );
    assert_eq!(s.vr[5], 0);
}

// -----------------------------------------------------------------
// AltiVec single-lane stores
// -----------------------------------------------------------------

#[test]
fn stvebx_emits_byte_from_be_lane_ea_low_nibble() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x5;
    // Lane 5 of the BE view holds the byte 0x55.
    let mut bytes = [0u8; 16];
    bytes[5] = 0x55;
    s.vr[3] = u128::from_be_bytes(bytes);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvebx {
            vs: 3,
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
            assert_eq!(range.start().raw(), 0x105);
            assert_eq!(bytes.bytes(), &[0x55]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stvehx_emits_halfword_from_aligned_be_lane() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x5; // EA = 0x105 -> aligned 0x104, lane 4
    let mut bytes = [0u8; 16];
    bytes[4] = 0xBE;
    bytes[5] = 0xEF;
    s.vr[3] = u128::from_be_bytes(bytes);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvehx {
            vs: 3,
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
            assert_eq!(range.start().raw(), 0x104);
            assert_eq!(bytes.bytes(), &[0xBE, 0xEF]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stvewx_emits_word_from_aligned_be_lane() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x9; // EA = 0x109 -> aligned 0x108, lane 8
    let mut bytes = [0u8; 16];
    bytes[8] = 0xDE;
    bytes[9] = 0xAD;
    bytes[10] = 0xBE;
    bytes[11] = 0xEF;
    s.vr[3] = u128::from_be_bytes(bytes);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvewx {
            vs: 3,
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
            assert_eq!(range.start().raw(), 0x108);
            assert_eq!(bytes.bytes(), &[0xDE, 0xAD, 0xBE, 0xEF]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

// -----------------------------------------------------------------
// AltiVec unaligned vector stores (stvlx/stvrx + l)
// -----------------------------------------------------------------

#[test]
fn stvlx_writes_high_bytes_starting_at_ea() {
    // EA & 0xF = 3 -> count = 16 - 3 = 13 bytes from VS[0..13]
    // stored to [EA..EA+13].
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x3;
    s.vr[3] = u128::from_be_bytes([
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ]);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvlx {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
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
    assert_eq!(writes.len(), 13);
    assert_eq!(writes[0], (0x103, 0x10));
    assert_eq!(writes[12], (0x10F, 0x1C));
}

#[test]
fn stvrx_writes_low_bytes_to_aligned_line_below_ea() {
    // EA & 0xF = 3 -> 3 bytes from VS[13..16] -> [aligned..aligned+3].
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x3;
    s.vr[3] = u128::from_be_bytes([
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ]);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvrx {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
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
    assert_eq!(writes, vec![(0x100, 0x1D), (0x101, 0x1E), (0x102, 0x1F)]);
}

#[test]
fn stvrx_aligned_ea_is_noop() {
    // EA & 0xF = 0 -> m=0; no stores emitted.
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    s.vr[3] = u128::MAX;
    let mut effects = Vec::new();
    let v = exec_with_mem(
        &PpuInstruction::Stvrx {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(v, ExecuteVerdict::Continue);
    assert!(effects.is_empty());
}

#[test]
fn stvlxl_matches_stvlx_semantics() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0xC; // 16 - 12 = 4 bytes
    s.vr[3] = u128::from_be_bytes([
        0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE,
        0xAF,
    ]);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvlxl {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
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
    assert_eq!(
        writes,
        vec![(0x10C, 0xA0), (0x10D, 0xA1), (0x10E, 0xA2), (0x10F, 0xA3),]
    );
}

#[test]
fn stvrxl_matches_stvrx_semantics() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x2; // 2 bytes from VS[14..16] -> [0x100, 0x101]
    s.vr[3] = u128::from_be_bytes([
        0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE,
        0xBF,
    ]);
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvrxl {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
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
    assert_eq!(writes, vec![(0x100, 0xBE), (0x101, 0xBF)]);
}

#[test]
fn stvxl_emits_16_byte_store_in_two_halves() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0;
    let val = 0x0011_2233_4455_6677_8899_AABB_CCDD_EEFFu128;
    s.vr[3] = val;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stvxl {
            vs: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(effects.len(), 2);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x100);
            assert_eq!(bytes.bytes(), &0x0011_2233_4455_6677u64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
    match &effects[1] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x108);
            assert_eq!(bytes.bytes(), &0x8899_AABB_CCDD_EEFFu64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}
