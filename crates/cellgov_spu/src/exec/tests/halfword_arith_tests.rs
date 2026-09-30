//! ah, ahi, sfh, sfhi and sfi: operand order, I10 sign extension at both
//! ends, and wrap in every slot.

use super::*;
use crate::state::SpuState;

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
// [SPU-ISA p:29 s:2.3] RI10: 8-bit opcode, I10, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

fn ri10(op: u32, rt: u32, ra: u32, i10: u32) -> u32 {
    op << 24 | (i10 & 0x3FF) << 14 | ra << 7 | rt
}

fn run(raw: u32, a: [u16; 8], b: [u16; 8]) -> [u16; 8] {
    let mut s = SpuState::new();
    s.regs[1] = std::array::from_fn(|i| a[i / 2].to_be_bytes()[i % 2]);
    s.regs[2] = std::array::from_fn(|i| b[i / 2].to_be_bytes()[i % 2]);
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    std::array::from_fn(|i| u16::from_be_bytes([s.regs[3][2 * i], s.regs[3][2 * i + 1]]))
}

const A: [u16; 8] = [1, 0x7FFF, 0xFFFF, 0x8000, 0x0010, 0x1234, 0, 0xFFF0];
const B: [u16; 8] = [2, 1, 1, 0x8000, 0x0001, 0x4321, 0, 0x0020];

#[test]
fn ah_adds_every_halfword_and_wraps() {
    assert_eq!(
        run(rr(0x0C8, 3, 1, 2), A, B),
        [3, 0x8000, 0, 0, 0x0011, 0x5555, 0, 0x0010]
    );
}

#[test]
fn sfh_subtracts_ra_from_rb_in_every_halfword() {
    assert_eq!(
        run(rr(0x048, 3, 1, 2), A, B),
        [1, 0x8002, 2, 0, 0xFFF1, 0x30ED, 0, 0x0030]
    );
}

#[test]
fn ahi_sign_extends_i10_at_both_ends() {
    // 0x1FF is +511; 0x200 is -512.
    assert_eq!(
        run(ri10(0x1D, 3, 1, 0x1FF), A, B),
        A.map(|h| h.wrapping_add(511))
    );
    assert_eq!(
        run(ri10(0x1D, 3, 1, 0x200), A, B),
        A.map(|h| h.wrapping_sub(512))
    );
}

#[test]
fn sfhi_subtracts_ra_from_the_immediate() {
    assert_eq!(
        run(ri10(0x0D, 3, 1, 0x1FF), A, B),
        A.map(|h| 511u16.wrapping_sub(h))
    );
    assert_eq!(
        run(ri10(0x0D, 3, 1, 0x200), A, B),
        A.map(|h| 0xFE00u16.wrapping_sub(h))
    );
}

#[test]
fn sfi_subtracts_each_word_from_the_word_immediate() {
    let mut s = SpuState::new();
    s.set_reg_word_slot(1, 0, 1);
    s.set_reg_word_slot(1, 1, 0xFFFF_FFFF);
    s.set_reg_word_slot(1, 2, 0x8000_0000);
    s.set_reg_word_slot(1, 3, 0x200);
    for (i10, t) in [(0x1FF_u32, 511u32), (0x200, 0xFFFF_FE00)] {
        let insn = crate::decode::decode(ri10(0x0C, 3, 1, i10)).expect("decodes");
        execute(&insn, &mut s, UnitId::new(0));
        assert_eq!(
            [0, 1, 2, 3].map(|slot| s.reg_word_slot(3, slot)),
            [1, 0xFFFF_FFFF, 0x8000_0000, 0x200].map(|w: u32| t.wrapping_sub(w)),
            "i10 {i10:#x}"
        );
    }
}

#[test]
fn the_five_decode_to_their_own_forms() {
    assert_eq!(
        crate::decode::decode(rr(0x0C8, 3, 1, 2)),
        Ok(SpuInstruction::Ah {
            rt: 3,
            ra: 1,
            rb: 2
        })
    );
    assert_eq!(
        crate::decode::decode(rr(0x048, 3, 1, 2)),
        Ok(SpuInstruction::Sfh {
            rt: 3,
            ra: 1,
            rb: 2
        })
    );
    assert_eq!(
        crate::decode::decode(ri10(0x1D, 3, 1, 0x200)),
        Ok(SpuInstruction::Ahi {
            rt: 3,
            ra: 1,
            imm: -512
        })
    );
    assert_eq!(
        crate::decode::decode(ri10(0x0D, 3, 1, 0x1FF)),
        Ok(SpuInstruction::Sfhi {
            rt: 3,
            ra: 1,
            imm: 511
        })
    );
    assert_eq!(
        crate::decode::decode(ri10(0x0C, 3, 1, 0x3FF)),
        Ok(SpuInstruction::Sfi {
            rt: 3,
            ra: 1,
            imm: -1
        })
    );
}
