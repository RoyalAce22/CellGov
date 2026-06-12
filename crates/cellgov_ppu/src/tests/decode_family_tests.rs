//! Family sweeps: one canonical encoding per member of each decoder arm family.

use super::*;

#[test]
fn altivec_memory_family_decodes_canonical_encodings() {
    // Each row: (raw, expected variant). The raw words use a
    // fixed RT/VT/VS=0, RA=1, RB=2 with the XO from the
    // AltiVec-PEM Ch. 6 instruction table.
    let p31 = 31u32 << 26;
    let regs = (0u32 << 21) | (1u32 << 16) | (2u32 << 11);
    let mk = |xo: u32| p31 | regs | (xo << 1);
    let cases: &[(u32, PpuInstruction)] = &[
        (
            mk(6),
            PpuInstruction::Lvsl {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(7),
            PpuInstruction::Lvebx {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(38),
            PpuInstruction::Lvsr {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(39),
            PpuInstruction::Lvehx {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(71),
            PpuInstruction::Lvewx {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(103),
            PpuInstruction::Lvx {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(135),
            PpuInstruction::Stvebx {
                vs: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(167),
            PpuInstruction::Stvehx {
                vs: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(199),
            PpuInstruction::Stvewx {
                vs: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(359),
            PpuInstruction::Lvxl {
                vt: 0,
                ra: 1,
                rb: 2,
            },
        ),
        (
            mk(487),
            PpuInstruction::Stvxl {
                vs: 0,
                ra: 1,
                rb: 2,
            },
        ),
    ];
    for &(raw, ref expected) in cases {
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}"));
        assert_eq!(&inst, expected, "raw 0x{raw:08x}");
    }
}

#[test]
fn indexed_update_family_decodes_canonical_encodings() {
    // X-form primary 31; RT/RS=3, RA=4, RB=5.
    let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
    let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
    let cases: &[(u32, PpuInstruction)] = &[
        (
            mk(55),
            PpuInstruction::Lwzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(119),
            PpuInstruction::Lbzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(311),
            PpuInstruction::Lhzux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(53),
            PpuInstruction::Ldux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(343),
            PpuInstruction::Lhax {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(375),
            PpuInstruction::Lhaux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(341),
            PpuInstruction::Lwax {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(373),
            PpuInstruction::Lwaux {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(407),
            PpuInstruction::Sthx {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(439),
            PpuInstruction::Sthux {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(183),
            PpuInstruction::Stwux {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(247),
            PpuInstruction::Stbux {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
    ];
    for &(raw, ref expected) in cases {
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}"));
        assert_eq!(&inst, expected, "raw 0x{raw:08x}");
    }
}

#[test]
fn load_store_string_family_decode_to_named_variants() {
    // X-form primary 31; for lswi/stswi the rb-slot encodes NB.
    let p31 = 31u32 << 26;
    let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
    let mk = |xo: u32| p31 | regs | (xo << 1);
    assert_eq!(
        decode(mk(533)).unwrap(),
        PpuInstruction::Lswx {
            rt: 3,
            ra: 4,
            rb: 5
        }
    );
    assert_eq!(
        decode(mk(597)).unwrap(),
        PpuInstruction::Lswi {
            rt: 3,
            ra: 4,
            nb: 5
        }
    );
    assert_eq!(
        decode(mk(661)).unwrap(),
        PpuInstruction::Stswx {
            rs: 3,
            ra: 4,
            rb: 5
        }
    );
    assert_eq!(
        decode(mk(725)).unwrap(),
        PpuInstruction::Stswi {
            rs: 3,
            ra: 4,
            nb: 5
        }
    );
}

#[test]
fn primary31_x_form_residue_decode_to_named_variants() {
    // Stage 40C.9: tw / td / popcntb / mcrxr.
    // tw/td: TO rides in the rt slot; bit 31 reserved (= 0).
    // popcntb: standard X-form; no Rc.
    // mcrxr: BF occupies bits 6..8 (rt high 3 bits); bits 9..10 reserved.
    let p31 = 31u32 << 26;
    // tw 12, r4, r5
    let raw_tw = p31 | (12u32 << 21) | (4u32 << 16) | (5u32 << 11) | (4u32 << 1);
    assert_eq!(
        decode(raw_tw).unwrap(),
        PpuInstruction::Tw {
            to: 12,
            ra: 4,
            rb: 5
        }
    );
    // td 24, r6, r7
    let raw_td = p31 | (24u32 << 21) | (6u32 << 16) | (7u32 << 11) | (68u32 << 1);
    assert_eq!(
        decode(raw_td).unwrap(),
        PpuInstruction::Td {
            to: 24,
            ra: 6,
            rb: 7
        }
    );
    // popcntb r3, r4
    let raw_popcntb = p31 | (4u32 << 21) | (3u32 << 16) | (122u32 << 1);
    assert_eq!(
        decode(raw_popcntb).unwrap(),
        PpuInstruction::Popcntb { ra: 3, rs: 4 }
    );
    // mcrxr cr3: BF=3 occupies bits 6..8; rt slot value = 3 << 2 = 12.
    let raw_mcrxr = p31 | (12u32 << 21) | (512u32 << 1);
    assert_eq!(decode(raw_mcrxr).unwrap(), PpuInstruction::Mcrxr { bf: 3 });
}

#[test]
fn xo_arith_and_logical_family_decode_to_named_variants() {
    // Stage 40C.7: XO-form arith (subfze/subfme/addme) and
    // 2-op logical (eqv/nand). XO-form uses 9-bit XO; logical
    // uses 10-bit XO. Test the bit-exact encoding shape.
    let p31 = 31u32 << 26;
    let regs_xo = (3u32 << 21) | (4u32 << 16);
    let mk_xo9 = |xo: u32| p31 | regs_xo | (xo << 1);
    assert_eq!(
        decode(mk_xo9(200)).unwrap(),
        PpuInstruction::Subfze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        }
    );
    assert_eq!(
        decode(mk_xo9(232)).unwrap(),
        PpuInstruction::Subfme {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        }
    );
    assert_eq!(
        decode(mk_xo9(234)).unwrap(),
        PpuInstruction::Addme {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        }
    );

    let regs_x = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
    let mk_x10 = |xo: u32| p31 | regs_x | (xo << 1);
    assert_eq!(
        decode(mk_x10(284)).unwrap(),
        PpuInstruction::Eqv {
            ra: 4,
            rs: 3,
            rb: 5,
            rc: false,
        }
    );
    assert_eq!(
        decode(mk_x10(476)).unwrap(),
        PpuInstruction::Nand {
            ra: 4,
            rs: 3,
            rb: 5,
            rc: false,
        }
    );
}

#[test]
fn cache_hint_family_collapses_to_nop() {
    // The cache-hint and data-stream-touch ops at primary 31 /
    // XOs 246 (dcbtst), 342 (dst), 374 (dstst), 822 (dss),
    // 982 (icbi) collapse to an Ori-nop under CellGov's deterministic
    // single-unit no-cache model. The collapse is by XO only;
    // operand bits are irrelevant. (`dst` / `dstst` / `dss` use
    // the AltiVec T(6) || STRM(7..8) || RA(11..15) || RB(16..20)
    // layout, not standard X-form `(rt, ra, rb)` -- but those
    // fields are ignored by the nop, and `dss` shares XO 822
    // with `dssall`, distinguished by the A bit also ignored.)
    // The XO bits in this fixture are the only thing the arm
    // discriminates on.
    let nop = PpuInstruction::Ori {
        ra: 0,
        rs: 0,
        imm: 0,
    };
    let p31 = 31u32 << 26;
    for xo in [246u32, 342, 374, 822, 982] {
        let raw = p31 | (xo << 1);
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} xo={xo}: {e:?}"));
        assert_eq!(inst, nop, "xo {xo} did not collapse to nop");
    }
}

#[test]
fn d_form_scalar_gaps_decode_to_named_variants() {
    // Each row: (primary, raw, expected). RT/RS/FRT/FRS=0,
    // RA=1, imm=4 (low bits free since none of these are
    // DS-form). Stage 40C.1 promoted these 5 ops out of the
    // OPCODE_GAPS top-level fall-through.
    let mk = |primary: u32| (primary << 26) | (0u32 << 21) | (1u32 << 16) | 4u32;
    let cases: &[(u32, PpuInstruction)] = &[
        (
            mk(43),
            PpuInstruction::Lhau {
                rt: 0,
                ra: 1,
                imm: 4,
            },
        ),
        (
            mk(46),
            PpuInstruction::Lmw {
                rt: 0,
                ra: 1,
                imm: 4,
            },
        ),
        (
            mk(47),
            PpuInstruction::Stmw {
                rs: 0,
                ra: 1,
                imm: 4,
            },
        ),
        (
            mk(49),
            PpuInstruction::Lfsu {
                frt: 0,
                ra: 1,
                imm: 4,
            },
        ),
        (
            mk(51),
            PpuInstruction::Lfdu {
                frt: 0,
                ra: 1,
                imm: 4,
            },
        ),
    ];
    for &(raw, ref expected) in cases {
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}"));
        assert_eq!(&inst, expected, "raw 0x{raw:08x}");
    }
}

#[test]
fn cbe_unaligned_vxu_family_decodes_canonical_encodings() {
    // X-form primary 31; VT/VS=2, RA=3, RB=4.
    let regs = (2u32 << 21) | (3u32 << 16) | (4u32 << 11);
    let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
    let cases: &[(u32, PpuInstruction)] = &[
        (
            mk(647),
            PpuInstruction::Lvlxl {
                vt: 2,
                ra: 3,
                rb: 4,
            },
        ),
        (
            mk(711),
            PpuInstruction::Lvrxl {
                vt: 2,
                ra: 3,
                rb: 4,
            },
        ),
        (
            mk(775),
            PpuInstruction::Stvlx {
                vs: 2,
                ra: 3,
                rb: 4,
            },
        ),
        (
            mk(839),
            PpuInstruction::Stvrx {
                vs: 2,
                ra: 3,
                rb: 4,
            },
        ),
        (
            mk(903),
            PpuInstruction::Stvlxl {
                vs: 2,
                ra: 3,
                rb: 4,
            },
        ),
        (
            mk(967),
            PpuInstruction::Stvrxl {
                vs: 2,
                ra: 3,
                rb: 4,
            },
        ),
    ];
    for &(raw, ref expected) in cases {
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}"));
        assert_eq!(&inst, expected, "raw 0x{raw:08x}");
    }
}

#[test]
fn byte_reverse_family_decodes_canonical_encodings() {
    // X-form primary 31; RT/RS=3, RA=4, RB=5.
    let regs = (3u32 << 21) | (4u32 << 16) | (5u32 << 11);
    let mk = |xo: u32| (31u32 << 26) | regs | (xo << 1);
    let cases: &[(u32, PpuInstruction)] = &[
        (
            mk(532),
            PpuInstruction::Ldbrx {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(534),
            PpuInstruction::Lwbrx {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(660),
            PpuInstruction::Sdbrx {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(662),
            PpuInstruction::Stwbrx {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(790),
            PpuInstruction::Lhbrx {
                rt: 3,
                ra: 4,
                rb: 5,
            },
        ),
        (
            mk(918),
            PpuInstruction::Sthbrx {
                rs: 3,
                ra: 4,
                rb: 5,
            },
        ),
    ];
    for &(raw, ref expected) in cases {
        let inst = decode(raw)
            .unwrap_or_else(|e| panic!("decode failed for raw 0x{raw:08x} ({expected:?}): {e:?}"));
        assert_eq!(&inst, expected, "raw 0x{raw:08x}");
    }
}

#[test]
fn mds_form_rldcl_rldcr_decode_to_named_variants() {
    // MDS-form: primary 30, RS=5, RA=6, RB=7, mask_lo=2,
    // mask_hi=1 -> mb/me = 0x22 = 34, Rc=0.
    // XO 8 (rldcl) and XO 9 (rldcr) live in PPC bits 27..30
    // (LSB-0 bits 1..4). The MD-form sub-XO bits 2..4 hit
    // XO=4 for both, which is reserved -- the MDS XO is the
    // distinguishing key.
    let common = (30u32 << 26)         // primary
        | (5u32 << 21)                  // rs = 5
        | (6u32 << 16)                  // ra = 6
        | (7u32 << 11)                  // rb = 7
        | (2u32 << 6)                   // mask_lo = 2
        | (1u32 << 5); // mask_hi = 1 -> mb/me = 34

    let rldcl_raw = common | (8u32 << 1); // MDS XO=8
    let rldcr_raw = common | (9u32 << 1); // MDS XO=9

    assert_eq!(
        decode(rldcl_raw).unwrap(),
        PpuInstruction::Rldcl {
            ra: 6,
            rs: 5,
            rb: 7,
            mb: 34,
            rc: false,
        }
    );
    assert_eq!(
        decode(rldcr_raw).unwrap(),
        PpuInstruction::Rldcr {
            ra: 6,
            rs: 5,
            rb: 7,
            me: 34,
            rc: false,
        }
    );
}

#[test]
fn cr_logical_family_decodes() {
    // BT=8, BA=9, BB=10 across each XO. The 5-bit fields lie at
    // raw bits (21..26), (16..21), (11..16) respectively.
    let mk = |xo: u32| (19u32 << 26) | (8u32 << 21) | (9u32 << 16) | (10u32 << 11) | (xo << 1);

    let cases: &[(u32, PpuInstruction)] = &[
        (
            33,
            PpuInstruction::Crnor {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            129,
            PpuInstruction::Crandc {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            193,
            PpuInstruction::Crxor {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            225,
            PpuInstruction::Crnand {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            257,
            PpuInstruction::Crand {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            289,
            PpuInstruction::Creqv {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            417,
            PpuInstruction::Crorc {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
        (
            449,
            PpuInstruction::Cror {
                bt: 8,
                ba: 9,
                bb: 10,
            },
        ),
    ];
    for (xo, expected) in cases {
        let raw = mk(*xo);
        assert_eq!(decode(raw).unwrap(), *expected, "xo={xo}");
    }
}

#[test]
fn x_form_fp_load_store_family_decodes() {
    // FRT/FRS=11, RA=4, RB=5 across each XO. The 5-bit fields lie
    // at raw bits (21..26), (16..21), (11..16) respectively;
    // X-form XO at bits 21..30 puts XO << 1 in the low half.
    let mk = |xo: u32| (31u32 << 26) | (11u32 << 21) | (4u32 << 16) | (5u32 << 11) | (xo << 1);

    let cases: &[(u32, PpuInstruction)] = &[
        (
            535,
            PpuInstruction::Lfsx {
                frt: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            567,
            PpuInstruction::Lfsux {
                frt: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            599,
            PpuInstruction::Lfdx {
                frt: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            631,
            PpuInstruction::Lfdux {
                frt: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            663,
            PpuInstruction::Stfsx {
                frs: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            695,
            PpuInstruction::Stfsux {
                frs: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            727,
            PpuInstruction::Stfdx {
                frs: 11,
                ra: 4,
                rb: 5,
            },
        ),
        (
            759,
            PpuInstruction::Stfdux {
                frs: 11,
                ra: 4,
                rb: 5,
            },
        ),
    ];
    for (xo, expected) in cases {
        let raw = mk(*xo);
        assert_eq!(decode(raw).unwrap(), *expected, "xo={xo}");
    }
}
