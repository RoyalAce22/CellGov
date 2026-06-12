//! Individual scalar / vector encodings decoded one instruction at a time.

use super::*;

#[test]
fn lvsr_decodes_to_named_variant_post_40e() {
    // lvsr v12, r0, r9 = 0x7d80484c (primary 31, XO 38). Stage
    // 40E graduated the AltiVec-memory family into the decoder;
    // the previous Phase-39-terminal "missing lvsr" reject is
    // now a successful decode.
    let raw = 0x7d80_484c;
    let inst = decode(raw).expect("lvsr must decode after 40E");
    assert_eq!(
        inst,
        PpuInstruction::Lvsr {
            vt: 12,
            ra: 0,
            rb: 9
        }
    );
}

#[test]
fn li_decodes_as_addi_ra0() {
    // li r3, 42 -> addi r3, r0, 42 -> 0x3860002A
    let raw = 0x3860_002A;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Addi {
            rt: 3,
            ra: 0,
            imm: 42
        }
    );
}

#[test]
fn oris_decodes() {
    // oris r2, r2, 3 -> 0x64420003
    let insn = decode(0x6442_0003).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Oris {
            ra: 2,
            rs: 2,
            imm: 3
        }
    );
}

#[test]
fn stwu_decodes() {
    // stwu r1, -128(r1) -> 0x9421FF80
    let insn = decode(0x9421_FF80).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stwu {
            rs: 1,
            ra: 1,
            imm: -128,
        }
    );
}

#[test]
fn stdu_decodes() {
    // stdu r1, -112(r1) -> 0xF821FF91 (sub-opcode 1)
    let insn = decode(0xF821_FF91).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stdu {
            rs: 1,
            ra: 1,
            imm: -112,
        }
    );
}

#[test]
fn rldicl_clrldi_decodes() {
    // clrldi r9, r3, 61 -> rldicl r9, r3, 0, 61 -> 0x78690760
    let insn = decode(0x7869_0760).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rldicl {
            ra: 9,
            rs: 3,
            sh: 0,
            mb: 61,
            rc: false,
        }
    );
}

#[test]
fn rldicr_sldi_decodes() {
    // sldi r9, r3, 4 -> rldicr r9, r3, 4, 59 -> 0x786926E4
    let insn = decode(0x7869_26E4).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rldicr {
            ra: 9,
            rs: 3,
            sh: 4,
            me: 59,
            rc: false,
        }
    );
}

#[test]
fn sth_decodes() {
    // sth r6, -24(r1) -> 0xb0c1ffe8
    let insn = decode(0xb0c1_ffe8).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Sth {
            rs: 6,
            ra: 1,
            imm: -24,
        }
    );
}

#[test]
fn vxor_clears_vector_register() {
    // vxor v0, v0, v0 -> 0x100004C4
    let insn = decode(0x1000_04C4).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Vxor {
            vt: 0,
            va: 0,
            vb: 0,
        }
    );
}

#[test]
fn ldu_decodes_with_negative_ds_offset() {
    // ldu r7, -8(r4): DS=-2 sign-extended through the shift-left-2.
    let insn = decode(0xE8E4_FFF9).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Ldu {
            rt: 7,
            ra: 4,
            imm: -8,
        }
    );
}

#[test]
fn ld_still_decodes_with_sub_zero() {
    // ld r3, 0(r4): primary-58 sub=0 must map to Ld, not Ldu.
    let insn = decode(0xE864_0000).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Ld {
            rt: 3,
            ra: 4,
            imm: 0
        }
    );
}

#[test]
fn lwa_decodes_at_primary_58_sub_2() {
    // lwa r3, 8(r4): primary=58, RT=3, RA=4, DS=2 (byte offset 8),
    // sub=2. Word: (58<<26) | (3<<21) | (4<<16) | 0x0008 | 2.
    let raw = (58u32 << 26) | (3u32 << 21) | (4u32 << 16) | 0x000A;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Lwa {
            rt: 3,
            ra: 4,
            imm: 8,
        }
    );
}

#[test]
fn addic_dot_decodes_at_primary_13() {
    // addic. r3, r4, -1: primary=13, RT=3, RA=4, SIMM=0xFFFF.
    let raw = (13u32 << 26) | (3u32 << 21) | (4u32 << 16) | 0xFFFF;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::AddicDot {
            rt: 3,
            ra: 4,
            imm: -1,
        }
    );
}

#[test]
fn andis_dot_decodes_at_primary_29() {
    // andis. r3, r4, 0x00FF: primary=29, RA=3, RS=4, UI=0xFF.
    let raw = (29u32 << 26) | (4u32 << 21) | (3u32 << 16) | 0x00FF;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::AndisDot {
            ra: 3,
            rs: 4,
            imm: 0x00FF,
        }
    );
}

#[test]
fn rlwnm_decodes() {
    // rlwnm r0, r0, r8, 0, 31 -> 0x5C00_403E.
    let insn = decode(0x5C00_403E).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rlwnm {
            ra: 0,
            rs: 0,
            rb: 8,
            mb: 0,
            me: 31,
            rc: false,
        }
    );
}

#[test]
fn adde_decodes() {
    // adde r3, r0, r29 -> XO(9)=138 -> 0x7C60_E914.
    let insn = decode(0x7C60_E914).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Adde {
            rt: 3,
            ra: 0,
            rb: 29,
            oe: false,
            rc: false,
        }
    );
}

#[test]
fn mulhdu_decodes() {
    // mulhdu r0, r0, r11: opcode 31, RT=0, RA=0, RB=11, XO=9 -> 0x7C005812
    let insn = decode(0x7C00_5812).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Mulhdu {
            rt: 0,
            ra: 0,
            rb: 11,
            rc: false,
        }
    );
}

#[test]
fn lbzu_decodes() {
    // lbzu r0, 1(r9) -> opcode 35, RT=0, RA=9, D=1 -> 0x8C090001
    let insn = decode(0x8C09_0001).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Lbzu {
            rt: 0,
            ra: 9,
            imm: 1,
        }
    );
}

#[test]
fn mr_decodes_as_or_rb_eq_rs() {
    // mr r31, r3 -> or r31, r3, r3 -> 0x7C7F1B78
    let insn = decode(0x7C7F_1B78).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Or {
            ra: 31,
            rs: 3,
            rb: 3,
            rc: false,
        }
    );
}

#[test]
fn orc_decodes() {
    // orc r0, r11, r28 -> XO(10)=412 -> 0x7D60_E338.
    let insn = decode(0x7D60_E338).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Orc {
            ra: 0,
            rs: 11,
            rb: 28,
            rc: false,
        }
    );
}

#[test]
fn addze_decodes() {
    // addze r0, r0 -> XO(9)=202 -> 0x7C00_0194.
    let insn = decode(0x7C00_0194).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Addze {
            rt: 0,
            ra: 0,
            oe: false,
            rc: false,
        }
    );
}

#[test]
fn cntlzd_decodes() {
    // cntlzd r0, r11 -> XO(10)=58 -> 0x7D60_0074.
    let insn = decode(0x7D60_0074).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Cntlzd {
            ra: 0,
            rs: 11,
            rc: false,
        }
    );
}

#[test]
fn stfsu_decodes() {
    // stfsu f13, 8(r8) -> primary 53 -> 0xD5A8_0008.
    let insn = decode(0xD5A8_0008).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stfsu {
            frs: 13,
            ra: 8,
            imm: 8,
        }
    );
}

#[test]
fn stfdu_decodes() {
    // stfdu f1, -8(r1) -> primary 55, FRS=1, RA=1, D=-8 -> 0xDC21_FFF8
    let insn = decode(0xDC21_FFF8).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stfdu {
            frs: 1,
            ra: 1,
            imm: -8,
        }
    );
}

#[test]
fn mulhw_decodes() {
    // mulhw r0, r0, r9 -> XO(9)=75 -> 0x7C00_4896.
    let insn = decode(0x7C00_4896).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Mulhw {
            rt: 0,
            ra: 0,
            rb: 9,
            rc: false,
        }
    );
}

#[test]
fn stfiwx_decodes() {
    // stfiwx f13, r0, r9 -> XO(10)=983 -> 0x7DA0_4FAE.
    let insn = decode(0x7DA0_4FAE).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stfiwx {
            frs: 13,
            ra: 0,
            rb: 9,
        }
    );
}

#[test]
fn lfsx_decodes() {
    // lfsx fr13, r3, r0.
    // Encoding: OP=31 | FRT=13 | RA=3 | RB=0 | XO(10)=535 | 0
    let insn = decode(0x7DA3_042E).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Lfsx {
            frt: 13,
            ra: 3,
            rb: 0,
        }
    );
}

#[test]
fn cmpi_l_bit_selects_cmpdi() {
    // cmpdi cr0, r3, 0 -> primary 11, BF=0, L=1, RA=3, imm=0 -> 0x2C23_0000
    let insn = decode(0x2C23_0000).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Cmpdi {
            bf: 0,
            ra: 3,
            imm: 0,
        }
    );
}

#[test]
fn cmpi_l_bit_zero_is_cmpwi() {
    // cmpwi cr0, r3, 0 -> primary 11, L=0 -> 0x2C03_0000
    let insn = decode(0x2C03_0000).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 0,
        }
    );
}

#[test]
fn cmpli_l_bit_selects_cmpldi() {
    // cmpldi cr0, r3, 0 -> primary 10, L=1 -> 0x2823_0000
    let insn = decode(0x2823_0000).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Cmpldi {
            bf: 0,
            ra: 3,
            imm: 0,
        }
    );
}

#[test]
fn stbu_decodes_with_update() {
    // stbu r6, -4(r1) -> primary 39, RS=6, RA=1, D=-4 -> 0x9CC1_FFFC
    let insn = decode(0x9CC1_FFFC).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Stbu {
            rs: 6,
            ra: 1,
            imm: -4,
        }
    );
}

#[test]
fn sthu_decodes_with_update() {
    // sthu r5, -8(r1) -> primary 45, RS=5, RA=1, D=-8 -> 0xB4A1_FFF8
    let insn = decode(0xB4A1_FFF8).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Sthu {
            rs: 5,
            ra: 1,
            imm: -8,
        }
    );
}

#[test]
fn rldic_decodes_from_xo_2() {
    // Build rldic r5, r4, SH=4, MB=32 manually.
    // primary=30, RS=4, RA=5, sh_lo=4 (SH&0x1F), mb_lo=32&0x1F=0,
    // mb_hi=(32>>5)&1 = 1, xo=2, sh_hi=0, Rc=0.
    let raw: u32 =
        (30 << 26) | (4u32 << 21) | (5u32 << 16) | (4u32 << 11) | (1u32 << 5) | (2u32 << 2);
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rldic {
            ra: 5,
            rs: 4,
            sh: 4,
            mb: 32,
            rc: false,
        }
    );
}

#[test]
fn rldimi_decodes_from_xo_3() {
    // rldimi r5, r4, SH=16, MB=0.
    // primary=30, RS=4, RA=5, sh_lo=16, mb_lo=0, mb_hi=0, xo=3.
    let raw: u32 = (30 << 26) | (4u32 << 21) | (5u32 << 16) | (16u32 << 11) | (3u32 << 2);
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rldimi {
            ra: 5,
            rs: 4,
            sh: 16,
            mb: 0,
            rc: false,
        }
    );
}

#[test]
fn xo_794_decodes_as_srad_not_sraw() {
    // primary=31, RT=5, RA=6, RB=7, XO(10)=794, Rc=0.
    let raw: u32 = (31u32 << 26) | (5u32 << 21) | (6u32 << 16) | (7u32 << 11) | (794u32 << 1);
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Srad {
            ra: 6,
            rs: 5,
            rb: 7,
            rc: false,
        }
    );
}

#[test]
fn add_dot_decodes_with_rc_set() {
    // add. r3, r4, r5 -> primary 31, XO(9)=266, Rc=1.
    let raw: u32 = (31u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (266u32 << 1) | 1;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Add {
            rt: 3,
            ra: 4,
            rb: 5,
            oe: false,
            rc: true,
        }
    );
}

#[test]
fn addo_decodes_with_oe_set() {
    // addo r3, r4, r5 -> primary 31, RT=3, RA=4, RB=5, OE=1, XO(9)=266, Rc=0.
    let raw: u32 =
        (31u32 << 26) | (3u32 << 21) | (4u32 << 16) | (5u32 << 11) | (1u32 << 10) | (266u32 << 1);
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Add {
            rt: 3,
            ra: 4,
            rb: 5,
            oe: true,
            rc: false,
        }
    );
}

#[test]
fn or_dot_decodes_with_rc_set() {
    // or. r3, r4, r5 -> primary 31, XO(10)=444, Rc=1.
    let raw: u32 = (31u32 << 26) | (4u32 << 21) | (3u32 << 16) | (5u32 << 11) | (444u32 << 1) | 1;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Or {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: true,
        }
    );
}

#[test]
fn rldicl_dot_decodes_with_rc_set() {
    // rldicl. r5, r4, sh=0, mb=61, Rc=1.
    let raw: u32 = (30u32 << 26) | (4u32 << 21) | (5u32 << 16) | (29u32 << 6) | 1;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Rldicl {
            ra: 5,
            rs: 4,
            sh: 0,
            mb: 29,
            rc: true,
        }
    );
}

#[test]
fn dcbz_decodes_with_real_variant() {
    // dcbz r6, r7 -> primary 31, RA=6, RB=7, XO=1014.
    let raw: u32 = (31u32 << 26) | (6u32 << 16) | (7u32 << 11) | (1014u32 << 1);
    let insn = decode(raw).unwrap();
    assert_eq!(insn, PpuInstruction::Dcbz { ra: 6, rb: 7 });
}

#[test]
fn vsldoi_decodes_with_shb_field() {
    // vsldoi v3, v1, v2, 4 -> primary 4, VT=3, VA=1, VB=2,
    // vc field holds SHB=4 in its low nibble, xo_6=0x2C.
    // Layout: primary=4, VT=3, VA=1, VB=2, shb=4 in bits 21..25
    // (vc slot), xo_6 in bits 0..5.
    let raw: u32 = (4u32 << 26) | (3u32 << 21) | (1u32 << 16) | (2u32 << 11) | (4u32 << 6) | 0x2C;
    let insn = decode(raw).unwrap();
    assert_eq!(
        insn,
        PpuInstruction::Vsldoi {
            vt: 3,
            va: 1,
            vb: 2,
            shb: 4,
        }
    );
}
