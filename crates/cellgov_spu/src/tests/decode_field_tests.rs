//! The fields the decoder once dropped: each one set alone arrives in its
//! variant.

use super::*;

const BI: u32 = 0x1A8 << 21;
const BISL: u32 = 0x1A9 << 21;
const BIZ: u32 = 0x128 << 21;
const BINZ: u32 = 0x129 << 21;
const BIHZ: u32 = 0x12A << 21;
const BIHNZ: u32 = 0x12B << 21;
const HBR: u32 = 0x1AC << 21;
const HBRA: u32 = 0x08 << 25;
const HBRR: u32 = 0x09 << 25;
const NOP: u32 = 0x201 << 21;

// [SPU-ISA p:178 s:7] D is instruction bit 12 and E is bit 13.
const D: u32 = 1 << (31 - 12);
const E: u32 = 1 << (31 - 13);

/// The (d, e) pair a branch-indirect variant carries.
fn interrupt_bits(raw: u32) -> (bool, bool) {
    match decode(raw).expect("decodes") {
        SpuInstruction::Bi { d, e, .. }
        | SpuInstruction::Bisl { d, e, .. }
        | SpuInstruction::Biz { d, e, .. }
        | SpuInstruction::Binz { d, e, .. }
        | SpuInstruction::Bihz { d, e, .. }
        | SpuInstruction::Bihnz { d, e, .. } => (d, e),
        other => panic!("{other:?} is not a branch-indirect form"),
    }
}

#[test]
fn every_branch_indirect_form_carries_d_and_e() {
    for opcode in [BI, BISL, BIZ, BINZ, BIHZ, BIHNZ] {
        assert_eq!(interrupt_bits(opcode), (false, false), "{opcode:#010x}");
        assert_eq!(interrupt_bits(opcode | D), (true, false), "{opcode:#010x}");
        assert_eq!(interrupt_bits(opcode | E), (false, true), "{opcode:#010x}");
        // [SPU-ISA p:178 s:7] D = E = 1 is reserved; the decode keeps the pair.
        assert_eq!(
            interrupt_bits(opcode | D | E),
            (true, true),
            "{opcode:#010x}"
        );
    }
}

// [SPU-ISA p:192 s:8] hbr: P at bit 11, ROH at bits 16 and 17, RA, ROL in the RT field.
#[test]
fn hbr_carries_p_ra_and_its_split_offset() {
    let hbr = |raw| decode(HBR | raw).expect("decodes");
    assert_eq!(
        hbr(1 << (31 - 11)),
        SpuInstruction::Hbr {
            p: true,
            ra: 0,
            ro: 0
        }
    );
    assert_eq!(
        hbr(9 << 7),
        SpuInstruction::Hbr {
            p: false,
            ra: 9,
            ro: 0
        }
    );
    // ROL alone is the low 7 bits of RO.
    assert_eq!(
        hbr(0x05),
        SpuInstruction::Hbr {
            p: false,
            ra: 0,
            ro: 5
        }
    );
    // ROH = 0b10 is RO's sign bit: RO = -256.
    assert_eq!(
        hbr(0b10 << 14),
        SpuInstruction::Hbr {
            p: false,
            ra: 0,
            ro: -256
        }
    );
}

// [SPU-ISA p:193 s:8] hbra: ROH at bits 7 and 8, I16, ROL in the RT field.
// [SPU-ISA p:194 s:8] hbrr: the same fields.
#[test]
fn hbra_and_hbrr_carry_their_split_offset_and_target() {
    assert_eq!(
        decode(HBRA | 0b01 << 23 | 0x7F),
        Ok(SpuInstruction::Hbra {
            ro: 0xFF,
            target: 0
        })
    );
    assert_eq!(
        decode(HBRA | 0xFFFF << 7),
        Ok(SpuInstruction::Hbra { ro: 0, target: -1 })
    );
    assert_eq!(
        decode(HBRR | 0b10 << 23 | 0x0040 << 7),
        Ok(SpuInstruction::Hbrr {
            ro: -256,
            offset: 0x40
        })
    );
}

// [SPU-ISA p:241 s:10] nop's RT is a false target the encoding still names.
#[test]
fn nop_carries_its_false_target() {
    assert_eq!(decode(NOP | 42), Ok(SpuInstruction::Nop { rt: 42 }));
}
