//! Encoder totality, canonical words, reserved bits, and alias spellings.

use super::*;
use crate::decode::decode;
use crate::instruction::fuzz::generation_descriptors;
use crate::instruction::{Fp63Op, PpuInstruction as I};

const NOP: I = I::Ori {
    ra: 0,
    rs: 0,
    imm: 0,
};

/// Floor on the witnessed kinds. The decoder produces well over this
/// many; a collapse means the descriptor scan stopped finding words.
const MIN_WITNESSED_KINDS: usize = 180;

fn witnesses() -> Vec<(u32, I)> {
    let descriptors = generation_descriptors();
    assert!(
        descriptors.len() >= MIN_WITNESSED_KINDS,
        "only {} witnessed kinds",
        descriptors.len()
    );
    descriptors
        .iter()
        .map(|d| {
            let raw = d.canonical_word;
            let insn = decode(raw)
                .unwrap_or_else(|e| panic!("witness {raw:#010x} for {:?} rejected: {e}", d.kind));
            (raw, insn)
        })
        .collect()
}

#[test]
fn every_witnessed_kind_re_encodes_to_a_word_that_decodes_to_itself() {
    for (raw, insn) in witnesses() {
        let encoded = encode(&insn).unwrap_or_else(|e| panic!("witness {raw:#010x} {insn:?}: {e}"));
        assert_eq!(decode(encoded), Ok(insn), "witness {raw:#010x}");
    }
}

#[test]
fn a_witness_re_encodes_to_itself_less_its_reserved_bits_or_is_an_alias() {
    for (raw, insn) in witnesses() {
        let encoded = encode(&insn).expect("witness encodes");
        match alias(raw) {
            None => assert_eq!(
                encoded,
                raw & !reserved_bits(&insn),
                "witness {raw:#010x} {insn:?}"
            ),
            Some(Alias::NopHint { .. }) => assert_eq!(insn, NOP, "witness {raw:#010x}"),
            Some(Alias::TimeBaseThroughMfspr { tbr }) => assert!(
                matches!((tbr, insn), (268, I::Mftb { .. }) | (269, I::Mftbu { .. })),
                "witness {raw:#010x} {insn:?} tbr {tbr}"
            ),
        }
    }
}

/// A flip inside `reserved_bits` keeps the instruction. A flip outside
/// it changes the decode, or lands on the other spelling of the same
/// instruction.
#[test]
fn reserved_bits_are_exactly_the_bits_the_decoder_does_not_read() {
    let mut reserved_flips = 0usize;
    for (raw, insn) in witnesses() {
        let reserved = reserved_bits(&insn);
        for bit in 0..u32::BITS {
            let flipped = raw ^ (1 << bit);
            let decoded = decode(flipped);
            if reserved & (1 << bit) != 0 {
                reserved_flips += 1;
                assert_eq!(
                    decoded,
                    Ok(insn),
                    "witness {raw:#010x} bit {bit} is reserved"
                );
            } else {
                assert!(
                    decoded != Ok(insn) || alias(flipped) != alias(raw),
                    "witness {raw:#010x} {insn:?}: the decoder ignores bit {bit}, \
                     which reserved_bits does not name"
                );
            }
        }
    }
    assert!(reserved_flips > 0, "no witness carried a reserved bit");
}

#[test]
fn every_nop_hint_decodes_to_the_preferred_nop_whatever_its_operand_bits() {
    let operand_patterns = [0u32, (5 << 21) | (6 << 16) | (7 << 11) | 1, 0x03FF_F801];
    let mut checked = 0;
    for pattern in operand_patterns {
        for xo in PPC_STORAGE_HINT_XOS {
            let raw = (31 << 26) | pattern | (xo << 1);
            assert_eq!(decode(raw), Ok(NOP), "hint {raw:#010x}");
            assert_eq!(
                alias(raw),
                Some(Alias::NopHint {
                    primary: 31,
                    xo: xo as u16
                })
            );
            assert_eq!(encode(&NOP), Ok(0x6000_0000));
            checked += 1;
        }
        let isync = (19 << 26) | pattern | (PPC_ISYNC_XO << 1);
        assert_eq!(decode(isync), Ok(NOP), "isync {isync:#010x}");
        assert_eq!(
            alias(isync),
            Some(Alias::NopHint {
                primary: 19,
                xo: PPC_ISYNC_XO as u16
            })
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        operand_patterns.len() * (PPC_STORAGE_HINT_XOS.len() + 1)
    );
}

#[test]
fn the_time_base_through_mfspr_is_an_alias_of_mftb() {
    // mfspr rT, 268 carries SPR 268 with its halves swapped: low 12 in RA, high 8 in RB.
    let mfspr_tb = (31 << 26) | (9 << 21) | (12 << 16) | (8 << 11) | (339 << 1);
    let mfspr_tbu = (31 << 26) | (9 << 21) | (13 << 16) | (8 << 11) | (339 << 1);
    assert_eq!(decode(mfspr_tb), Ok(I::Mftb { rt: 9 }));
    assert_eq!(decode(mfspr_tbu), Ok(I::Mftbu { rt: 9 }));
    assert_eq!(
        alias(mfspr_tb),
        Some(Alias::TimeBaseThroughMfspr { tbr: 268 })
    );
    assert_eq!(
        alias(mfspr_tbu),
        Some(Alias::TimeBaseThroughMfspr { tbr: 269 })
    );
    let mftb = encode(&I::Mftb { rt: 9 }).expect("mftb encodes");
    assert_eq!(mftb, mfspr_tb & !(339 << 1) | (371 << 1));
    assert_eq!(alias(mftb), None);
    assert_eq!(decode(mftb), Ok(I::Mftb { rt: 9 }));
}

#[test]
fn a_reserved_bit_difference_is_not_an_alias() {
    let mflr_with_rc = (31 << 26) | (9 << 21) | (8 << 16) | (339 << 1) | 1;
    assert_eq!(decode(mflr_with_rc), Ok(I::Mflr { rt: 9 }));
    assert_eq!(alias(mflr_with_rc), None);
    assert_eq!(encode(&I::Mflr { rt: 9 }), Ok(mflr_with_rc & !1));
}

#[test]
fn quickened_forms_encode_as_the_base_instruction_their_mnemonic_names() {
    let cases = [
        (
            I::Li { rt: 3, imm: -5 },
            I::Addi {
                rt: 3,
                ra: 0,
                imm: -5,
            },
        ),
        (
            I::Mr { ra: 4, rs: 9 },
            I::Or {
                ra: 4,
                rs: 9,
                rb: 9,
                rc: false,
            },
        ),
        (
            I::Slwi { ra: 4, rs: 9, n: 5 },
            I::Rlwinm {
                ra: 4,
                rs: 9,
                sh: 5,
                mb: 0,
                me: 26,
                rc: false,
            },
        ),
        (
            I::Srwi { ra: 4, rs: 9, n: 5 },
            I::Rlwinm {
                ra: 4,
                rs: 9,
                sh: 27,
                mb: 5,
                me: 31,
                rc: false,
            },
        ),
        (
            I::Clrlwi { ra: 4, rs: 9, n: 5 },
            I::Rlwinm {
                ra: 4,
                rs: 9,
                sh: 0,
                mb: 5,
                me: 31,
                rc: false,
            },
        ),
        (
            I::Clrldi { ra: 4, rs: 9, n: 5 },
            I::Rldicl {
                ra: 4,
                rs: 9,
                sh: 0,
                mb: 5,
                rc: false,
            },
        ),
        (
            I::Sldi { ra: 4, rs: 9, n: 5 },
            I::Rldicr {
                ra: 4,
                rs: 9,
                sh: 5,
                me: 58,
                rc: false,
            },
        ),
        (
            I::Srdi { ra: 4, rs: 9, n: 5 },
            I::Rldicl {
                ra: 4,
                rs: 9,
                sh: 59,
                mb: 5,
                rc: false,
            },
        ),
        (I::Nop, NOP),
        (
            I::CmpwZero { bf: 2, ra: 7 },
            I::Cmpwi {
                bf: 2,
                ra: 7,
                imm: 0,
            },
        ),
    ];
    for (quick, base) in cases {
        let word = encode(&quick).unwrap_or_else(|e| panic!("{quick:?}: {e}"));
        assert_eq!(Ok(word), encode(&base), "{quick:?}");
        assert_eq!(decode(word), Ok(base), "{quick:?}");
    }
}

#[test]
fn a_super_pair_and_its_consumed_slot_are_refused_by_kind() {
    let pair = I::LwzCmpwi {
        rt: 3,
        ra_load: 1,
        offset: 8,
        bf: 0,
        cmp_imm: 0,
    };
    assert_eq!(
        encode(&pair),
        Err(EncodeError::NoStandaloneEncoding {
            kind: PpuInstructionKind::LwzCmpwi
        })
    );
    assert_eq!(
        encode(&I::Consumed),
        Err(EncodeError::NoStandaloneEncoding {
            kind: PpuInstructionKind::Consumed
        })
    );
}

#[test]
fn an_x_form_float_op_takes_its_frc_slot_from_the_opcode() {
    let fmr = I::Fp63 {
        op: Fp63Op::Fmr,
        frt: 1,
        fra: 0,
        frb: 2,
        frc: 0,
        rc: false,
    };
    let word = encode(&fmr).expect("fmr encodes");
    assert_eq!(word, (63 << 26) | (1 << 21) | (2 << 11) | (72 << 1));
    let decoded = decode(word).expect("fmr decodes");
    assert!(
        matches!(
            decoded,
            I::Fp63 {
                op: Fp63Op::Fmr,
                frt: 1,
                frb: 2,
                frc: 2,
                ..
            }
        ),
        "{decoded:?}"
    );
    assert_eq!(encode(&decoded), Ok(word));
}
