//! The eight cache-block instructions under primary 31: each decodes to
//! its own variant whatever its reserved bits, encodes to its canonical
//! word, and renders by its mnemonic.

use cellgov_ps3_abi::hw::ppc_isa::PPC_CACHE_BLOCK_XOS;

use super::{encode, reserved_bits};
use crate::decode::decode;
use crate::instruction::{AsmText, PpuInstruction as I};

/// The MSB-0 bit field `first..=last` of `raw`.
fn bits(raw: u32, first: u32, last: u32) -> u8 {
    ((raw >> (31 - last)) & ((1 << (last - first + 1)) - 1)) as u8
}

/// The instruction the book's field layout gives for `raw`.
///
/// [PPC-Book2 p:21 s:3.2.2] dcbst 54, dcbf 86; [PPC-Book2 p:19 s:3.2.2] dcbt 278, dcbtst 246.
/// [PPC-Book2 p:35 s:5.2.1.1] TH in bits 7:10, bit 6 reserved.
/// [PPC-Book2 p:41 s:5.2.1.1] dcbf L in bit 10, bits 6:9 reserved.
/// [PPC-Book2 p:18 s:3.2.1] icbi 982.
/// [AltiVec-PEM p:6-10 s:6.2] dst 342 and [AltiVec-PEM p:6-12 s:6.2] dstst 374: T in bit 6, STRM in bits 9:10.
/// [AltiVec-PEM p:6-9 s:6.2] dss 822: A in bit 6, STRM in bits 9:10.
fn expected(xo: u32, raw: u32) -> I {
    let (ra, rb) = (bits(raw, 11, 15), bits(raw, 16, 20));
    let (flag, strm, th) = (bits(raw, 6, 6) == 1, bits(raw, 9, 10), bits(raw, 7, 10));
    match xo {
        54 => I::Dcbst { ra, rb },
        86 => I::Dcbf {
            ra,
            rb,
            l: bits(raw, 10, 10),
        },
        982 => I::Icbi { ra, rb },
        278 => I::Dcbt { ra, rb, th },
        246 => I::Dcbtst { ra, rb, th },
        342 => I::Dst {
            ra,
            rb,
            strm,
            t: flag,
        },
        374 => I::Dstst {
            ra,
            rb,
            strm,
            t: flag,
        },
        822 => I::Dss { strm, all: flag },
        _ => unreachable!("xo {xo} is not a cache-block instruction"),
    }
}

#[test]
fn each_cache_block_word_decodes_to_its_own_instruction_whatever_its_other_bits() {
    let mut checked = 0usize;
    for xo in PPC_CACHE_BLOCK_XOS {
        for slot_6_10 in 0..32u32 {
            for (ra, rb) in [(0u32, 0u32), (3, 4), (31, 17)] {
                for rc in 0..2u32 {
                    let raw =
                        (31 << 26) | (slot_6_10 << 21) | (ra << 16) | (rb << 11) | (xo << 1) | rc;
                    let insn = decode(raw).unwrap_or_else(|e| panic!("{raw:#010x}: {e}"));
                    assert_eq!(insn, expected(xo, raw), "{raw:#010x}");
                    assert_ne!(
                        insn,
                        I::Ori {
                            ra: 0,
                            rs: 0,
                            imm: 0
                        },
                        "{raw:#010x}"
                    );
                    assert_eq!(
                        encode(&insn),
                        Ok(raw & !reserved_bits(&insn)),
                        "{raw:#010x}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, PPC_CACHE_BLOCK_XOS.len() * 32 * 3 * 2);
}

#[test]
fn each_cache_block_instruction_encodes_to_its_canonical_word() {
    for (insn, word) in [
        (I::Dcbst { ra: 3, rb: 4 }, 0x7c03_206c),
        (I::Dcbf { ra: 3, rb: 4, l: 0 }, 0x7c03_20ac),
        (I::Dcbf { ra: 3, rb: 4, l: 1 }, 0x7c23_20ac),
        (I::Icbi { ra: 3, rb: 4 }, 0x7c03_27ac),
        (
            I::Dcbt {
                ra: 3,
                rb: 4,
                th: 0,
            },
            0x7c03_222c,
        ),
        (
            I::Dcbtst {
                ra: 3,
                rb: 4,
                th: 8,
            },
            0x7d03_21ec,
        ),
        (
            I::Dst {
                ra: 3,
                rb: 4,
                strm: 1,
                t: true,
            },
            0x7e23_22ac,
        ),
        (
            I::Dstst {
                ra: 3,
                rb: 4,
                strm: 2,
                t: false,
            },
            0x7c43_22ec,
        ),
        (I::Dss { strm: 0, all: true }, 0x7e00_066c),
        (
            I::Dss {
                strm: 3,
                all: false,
            },
            0x7c60_066c,
        ),
    ] {
        assert_eq!(encode(&insn), Ok(word), "{insn:?}");
    }
}

fn render(insn: I) -> Vec<String> {
    AsmText {
        insn: &insn,
        addr: 0x1_0000,
        symbols: None,
    }
    .to_string()
    .split_whitespace()
    .map(str::to_owned)
    .collect()
}

#[test]
fn each_cache_block_instruction_renders_by_its_mnemonic() {
    for (insn, text) in [
        (I::Dcbst { ra: 3, rb: 4 }, "dcbst r3, r4"),
        (I::Dcbf { ra: 0, rb: 9, l: 0 }, "dcbf r0, r9"),
        (I::Dcbf { ra: 0, rb: 9, l: 1 }, "dcbfl r0, r9"),
        (I::Icbi { ra: 3, rb: 4 }, "icbi r3, r4"),
        (
            I::Dcbt {
                ra: 3,
                rb: 4,
                th: 0,
            },
            "dcbt r3, r4",
        ),
        (
            I::Dcbtst {
                ra: 3,
                rb: 4,
                th: 8,
            },
            "dcbtst r3, r4, 8",
        ),
        (
            I::Dst {
                ra: 3,
                rb: 4,
                strm: 1,
                t: true,
            },
            "dstt r3, r4, 1",
        ),
        (
            I::Dstst {
                ra: 3,
                rb: 4,
                strm: 2,
                t: false,
            },
            "dstst r3, r4, 2",
        ),
        (I::Dss { strm: 2, all: true }, "dssall"),
        (
            I::Dss {
                strm: 3,
                all: false,
            },
            "dss 3",
        ),
    ] {
        assert_eq!(
            render(insn),
            text.split_whitespace().collect::<Vec<_>>(),
            "{insn:?}"
        );
    }
}
