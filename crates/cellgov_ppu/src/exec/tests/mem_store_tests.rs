//! Fixed-point stores: D/DS/X-form, update forms, stmw, and dcbz block zeroing.

use super::*;

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
        if let Effect::SharedWriteIntent { range, bytes, .. } = eff {
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
        if let Effect::SharedWriteIntent { range, .. } = eff {
            let addr = range.start().raw();
            assert!(
                (0x2000..0x2080).contains(&addr),
                "effect addr 0x{addr:x} outside aligned block [0x2000, 0x2080)",
            );
        }
    }
}

#[test]
fn dcbz_increments_counter_on_each_execution() {
    // C-4 audit witness: dcbz_executed increments per Dcbz
    // arm entry. The MMIO-window debug_assert is evaluated
    // before the counter resolves; this proves silence is
    // non-vacuous when the counter is > 0.
    let base = 0x1000u64;
    let mem = vec![0u8; 384];
    let mut s = PpuState::new();
    assert_eq!(s.dcbz_executed, 0);
    s.gpr[3] = 0;
    s.gpr[4] = 0x1000;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        base,
        &mem,
        &mut effects,
    );
    assert_eq!(s.dcbz_executed, 1);
    exec_with_mem(
        &PpuInstruction::Dcbz { ra: 3, rb: 4 },
        &mut s,
        base,
        &mem,
        &mut effects,
    );
    assert_eq!(s.dcbz_executed, 2);
}

#[test]
fn dcbz_pre_checks_capacity_for_full_block() {
    // Buffer with 50 entries leaves 14 slots -- not enough for
    // dcbz's 16 doubleword stores.
    let mut s = PpuState::new();
    s.gpr[1] = 0x2000;
    s.gpr[2] = 0;
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    for i in 0..50 {
        assert!(store_buf.insert((i as u64) * 8, 8, 0));
    }
    let v = execute(
        &PpuInstruction::Dcbz { ra: 1, rb: 2 },
        &mut s,
        UnitId::new(0),
        &[(0, &[0u8; 0x4000])],
        &mut effects,
        &mut store_buf,
    );
    assert_eq!(v, ExecuteVerdict::BufferFull);
    assert_eq!(
        store_buf.len(),
        50,
        "dcbz must not stage any stores when capacity is insufficient"
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "stwu invalid form")]
fn stwu_with_ra_zero_panics_in_debug() {
    let mut s = PpuState::new();
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwu {
            rs: 3,
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
// Integer stores
// -----------------------------------------------------------------

#[test]
fn std_emits_8_byte_store_big_endian() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[5] = 0x0123_4567_89AB_CDEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Std {
            rs: 5,
            ra: 1,
            imm: 0x10,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), 0x110);
            assert_eq!(range.length(), 8);
            assert_eq!(bytes.bytes(), &0x0123_4567_89AB_CDEFu64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stdu_updates_ra_and_emits_8_byte_store() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[5] = 0xCAFE_F00D_DEAD_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdu {
            rs: 5,
            ra: 1,
            imm: -8,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(s.gpr[1], 0xF8);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => assert_eq!(range.start().raw(), 0xF8),
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stmw_emits_words_starting_at_rs_through_r31() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[29] = 0x1111_2222;
    s.gpr[30] = 0x3333_4444;
    s.gpr[31] = 0x5555_6666;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stmw {
            rs: 29,
            ra: 1,
            imm: 0,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    let writes: Vec<(u64, [u8; 4])> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                let b: [u8; 4] = bytes.bytes().try_into().ok()?;
                Some((range.start().raw(), b))
            }
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 3);
    assert_eq!(writes[0], (0x100, 0x1111_2222u32.to_be_bytes()));
    assert_eq!(writes[1], (0x104, 0x3333_4444u32.to_be_bytes()));
    assert_eq!(writes[2], (0x108, 0x5555_6666u32.to_be_bytes()));
}

#[test]
fn stwx_emits_word_store_at_ra_plus_rb() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x10;
    s.gpr[5] = 0xDEAD_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwx {
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
            assert_eq!(range.start().raw(), 0x110);
            assert_eq!(bytes.bytes(), &0xDEAD_BEEFu32.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stdx_emits_8_byte_store_at_ra_plus_rb() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x8;
    s.gpr[5] = 0x0011_2233_4455_6677;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stdx {
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
            assert_eq!(range.start().raw(), 0x108);
            assert_eq!(bytes.bytes(), &0x0011_2233_4455_6677u64.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stbx_emits_low_byte_store() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x4;
    s.gpr[5] = 0xFFFF_FFFF_FFFF_FF42;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stbx {
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
            assert_eq!(range.start().raw(), 0x104);
            assert_eq!(bytes.bytes(), &[0x42]);
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn sthx_emits_low_halfword_store_big_endian() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x2;
    s.gpr[5] = 0xFFFF_FFFF_FFFF_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Sthx {
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
            assert_eq!(range.start().raw(), 0x102);
            assert_eq!(bytes.bytes(), &0xBEEFu16.to_be_bytes());
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn sthux_writes_back_ra_only_on_success() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x4;
    s.gpr[5] = 0xCAFE;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Sthux {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(s.gpr[1], 0x104);
    match &effects[0] {
        Effect::SharedWriteIntent { range, .. } => assert_eq!(range.start().raw(), 0x104),
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

#[test]
fn stwux_writes_back_ra_only_on_success() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x8;
    s.gpr[5] = 0xDEAD_BEEF;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stwux {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(s.gpr[1], 0x108);
}

#[test]
fn stbux_writes_back_ra_only_on_success() {
    let mut s = PpuState::new();
    s.gpr[1] = 0x100;
    s.gpr[2] = 0x1;
    s.gpr[5] = 0x33;
    let mut effects = Vec::new();
    exec_with_mem(
        &PpuInstruction::Stbux {
            rs: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        0,
        &[0u8; 0x200],
        &mut effects,
    );
    assert_eq!(s.gpr[1], 0x101);
}
