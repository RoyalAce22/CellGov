//! Golden tests for canonical assembly rendering: raw word in,
//! exact line out. Encoders mirror the form layouts the decoder
//! consumes; a handful of firmware-observed literals anchor the
//! encodings independently.

use crate::decode::decode;
use crate::instruction::{AsmText, PpuInstruction};

/// Decode `raw` and render at `addr`.
fn fmt_at(raw: u32, addr: u64) -> String {
    let insn = decode(raw).unwrap_or_else(|e| panic!("decode 0x{raw:08x} failed: {e}"));
    AsmText {
        insn: &insn,
        addr,
        symbols: None,
    }
    .to_string()
}

/// Decode `raw` and render at address 0.
fn fmt(raw: u32) -> String {
    fmt_at(raw, 0)
}

/// Render an instruction built directly (for variants `decode()`
/// never produces: quickened forms, superinstructions).
fn fmt_insn(insn: PpuInstruction) -> String {
    AsmText {
        insn: &insn,
        addr: 0x1_0000,
        symbols: None,
    }
    .to_string()
}

// -- Encoders (transparent mirrors of the decode forms) --

fn enc_d(primary: u32, rt: u32, ra: u32, imm: u16) -> u32 {
    (primary << 26) | (rt << 21) | (ra << 16) | imm as u32
}

fn enc_x(primary: u32, rt: u32, ra: u32, rb: u32, xo: u32, rc: bool) -> u32 {
    (primary << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (xo << 1) | rc as u32
}

fn enc_xo(rt: u32, ra: u32, rb: u32, oe: bool, xo: u32, rc: bool) -> u32 {
    (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | ((oe as u32) << 10) | (xo << 1) | rc as u32
}

fn enc_m(primary: u32, rs: u32, ra: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (primary << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | rc as u32
}

#[test]
fn fmt_d_form_loads_and_stores() {
    // lwz r3, -16(r1)
    assert_eq!(fmt(enc_d(32, 3, 1, 0xFFF0)), "lwz        r3, -16(r1)");
    // stw r31, 8(r1)
    assert_eq!(fmt(enc_d(36, 31, 1, 8)), "stw        r31, 8(r1)");
    // lbz r0, 0(r9)
    assert_eq!(fmt(enc_d(34, 0, 9, 0)), "lbz        r0, 0(r9)");
    // lmw r29, -12(r1)
    assert_eq!(fmt(enc_d(46, 29, 1, 0xFFF4)), "lmw        r29, -12(r1)");
}

#[test]
fn fmt_ds_form_byte_offsets() {
    // ld r2, 40(r1): DS-form imm is a byte offset, sub=0.
    assert_eq!(fmt(enc_d(58, 2, 1, 40)), "ld         r2, 40(r1)");
    // std r0, 16(r1)
    assert_eq!(fmt(enc_d(62, 0, 1, 16)), "std        r0, 16(r1)");
    // stdu r1, -112(r1): sub=1.
    assert_eq!(
        fmt(enc_d(62, 1, 1, (-112i16 as u16) | 1)),
        "stdu       r1, -112(r1)"
    );
    // lwa r4, 4(r3): sub=2.
    assert_eq!(fmt(enc_d(58, 4, 3, 4 | 2)), "lwa        r4, 4(r3)");
}

#[test]
fn fmt_arith_immediates_signed_decimal() {
    assert_eq!(fmt(enc_d(14, 3, 4, 0xFFFF)), "addi       r3, r4, -1");
    assert_eq!(fmt(enc_d(15, 5, 6, 2)), "addis      r5, r6, 2");
    assert_eq!(fmt(enc_d(7, 7, 8, 100)), "mulli      r7, r8, 100");
    assert_eq!(fmt(enc_d(8, 9, 10, 0xFFFE)), "subfic     r9, r10, -2");
    assert_eq!(fmt(enc_d(13, 3, 3, 1)), "addic.     r3, r3, 1");
}

#[test]
fn fmt_logical_immediates_hex() {
    // RS rides in the D-form rt slot; rendering is `op ra, rs, ui`.
    assert_eq!(fmt(enc_d(24, 3, 4, 0x8000)), "ori        r4, r3, 0x8000");
    assert_eq!(fmt(enc_d(25, 3, 4, 0xF)), "oris       r4, r3, 0xf");
    assert_eq!(fmt(enc_d(26, 3, 4, 0xFF)), "xori       r4, r3, 0xff");
    assert_eq!(fmt(enc_d(28, 3, 4, 0x7)), "andi.      r4, r3, 0x7");
}

#[test]
fn fmt_xo_form_oe_rc_suffixes() {
    assert_eq!(
        fmt(enc_xo(3, 4, 5, false, 266, false)),
        "add        r3, r4, r5"
    );
    assert_eq!(
        fmt(enc_xo(3, 4, 5, false, 266, true)),
        "add.       r3, r4, r5"
    );
    assert_eq!(
        fmt(enc_xo(3, 4, 5, true, 266, false)),
        "addo       r3, r4, r5"
    );
    assert_eq!(
        fmt(enc_xo(3, 4, 5, true, 266, true)),
        "addo.      r3, r4, r5"
    );
    assert_eq!(
        fmt(enc_xo(3, 4, 5, false, 40, false)),
        "subf       r3, r4, r5"
    );
    assert_eq!(fmt(enc_xo(6, 7, 0, false, 104, false)), "neg        r6, r7");
    assert_eq!(
        fmt(enc_xo(3, 4, 5, false, 491, true)),
        "divw.      r3, r4, r5"
    );
}

#[test]
fn fmt_x_form_logical_rs_first_in_ra_slot() {
    // X-form logical ops render `op ra, rs, rb` -- rs travels in the
    // rt slot of the encoding.
    assert_eq!(fmt(enc_x(31, 5, 3, 7, 444, false)), "or         r3, r5, r7");
    assert_eq!(fmt(enc_x(31, 5, 3, 7, 28, true)), "and.       r3, r5, r7");
    assert_eq!(fmt(enc_x(31, 5, 3, 7, 316, false)), "xor        r3, r5, r7");
}

#[test]
fn fmt_shift_immediates_decimal() {
    assert_eq!(fmt(enc_x(31, 5, 3, 4, 824, false)), "srawi      r3, r5, 4");
}

#[test]
fn fmt_sradi_six_bit_shift() {
    // sradi r3, r5, 34: XS-form; sh=34 splits across rb slot (2) and
    // raw bit 1 (sh_hi=1), making the 10-bit xo read 827.
    let raw = (31 << 26) | (5 << 21) | (3 << 16) | (2 << 11) | (413 << 2) | (1 << 1);
    assert_eq!(fmt(raw), "sradi      r3, r5, 34");
}

#[test]
fn fmt_rotates() {
    // mb=1 dodges every extended-mnemonic gate.
    assert_eq!(
        fmt(enc_m(21, 5, 3, 4, 1, 27, false)),
        "rlwinm     r3, r5, 4, 1, 27"
    );
    assert_eq!(
        fmt(enc_m(20, 5, 3, 4, 8, 27, true)),
        "rlwimi.    r3, r5, 4, 8, 27"
    );
    // rldicl r3, r5, 2, 0 (MD-form xo=0).
    let raw = (30 << 26) | (5 << 21) | (3 << 16) | (2 << 11);
    assert_eq!(fmt(raw), "rldicl     r3, r5, 2, 0");
}

#[test]
fn fmt_compares_omit_cr0() {
    // cmpwi r3, 0 (bf=0 omitted).
    assert_eq!(fmt(enc_d(11, 0, 3, 0)), "cmpwi      r3, 0");
    // cmpwi cr7, r3, -1.
    assert_eq!(fmt(enc_d(11, 7 << 2, 3, 0xFFFF)), "cmpwi      cr7, r3, -1");
    // cmpdi r4, 5 (L=1).
    assert_eq!(fmt(enc_d(11, 1, 4, 5)), "cmpdi      r4, 5");
    // cmplwi cr6, r9, 0xff: unsigned compares render hex.
    assert_eq!(fmt(enc_d(10, 6 << 2, 9, 0xFF)), "cmplwi     cr6, r9, 0xff");
    // cmpw cr1, r3, r4 (X-form xo=0).
    assert_eq!(
        fmt(enc_x(31, 1 << 2, 3, 4, 0, false)),
        "cmpw       cr1, r3, r4"
    );
    // cmpld r5, r6 (X-form xo=32, L=1, bf=0).
    assert_eq!(fmt(enc_x(31, 1, 5, 6, 32, false)), "cmpld      r5, r6");
}

#[test]
fn fmt_branch_targets_resolve_absolute() {
    // b 0x10080 from 0x10000: offset +0x80.
    let raw = (18 << 26) | 0x80;
    assert_eq!(fmt_at(raw, 0x10000), "b          0x10080");
    // bl 0xff80 from 0x10000: offset -0x80, link.
    let back = (18 << 26) | (0x03FF_FFFC & (-0x80i32 as u32)) | 1;
    assert_eq!(fmt_at(back, 0x10000), "bl         0xff80");
    // ba 0x1234 absolute.
    let abs = (18 << 26) | 0x1234 | 2;
    assert_eq!(fmt_at(abs, 0xDEAD_0000), "ba         0x1234");
    // bc 20, 0, 0x10010: branch-always via bc has no extended
    // mnemonic (Table 3 row is blank), so the canonical form stays.
    let bc = (16 << 26) | (20 << 21) | 0x10;
    assert_eq!(fmt_at(bc, 0x10000), "bc         20, 0, 0x10010");
}

#[test]
fn fmt_branch_simplifications() {
    // blr / blrl / bctr / bctrl (BO=20 branch always).
    assert_eq!(fmt((19 << 26) | (20 << 21) | (16 << 1)), "blr");
    assert_eq!(fmt((19 << 26) | (20 << 21) | (16 << 1) | 1), "blrl");
    assert_eq!(fmt((19 << 26) | (20 << 21) | (528 << 1)), "bctr");
    assert_eq!(fmt((19 << 26) | (20 << 21) | (528 << 1) | 1), "bctrl");
}

#[test]
fn fmt_conditional_branch_mnemonics() {
    let bc = |bo: u32, bi: u32, bd: u32| (16u32 << 26) | (bo << 21) | (bi << 16) | bd;
    // [PPC-Book1 p:153 s:B.2.3] bne target = bc 4,2,target.
    assert_eq!(fmt_at(bc(4, 2, 8), 0x100), "bne        0x108");
    // [PPC-Book1 p:154 s:B.2.3] bne cr3 = bc 4,14.
    assert_eq!(fmt_at(bc(4, 14, 8), 0x100), "bne        cr3, 0x108");
    // blt = bc 12,0.
    assert_eq!(fmt_at(bc(12, 0, 8), 0x100), "blt        0x108");
    // bso cr7 = bc 12,31; bns = bc 4,3.
    assert_eq!(fmt_at(bc(12, 31, 8), 0x100), "bso        cr7, 0x108");
    assert_eq!(fmt_at(bc(4, 3, 8), 0x100), "bns        0x108");
    // [PPC-Book1 p:153 s:B.2.2] bdnz target = bc 16,0,target.
    assert_eq!(fmt_at(bc(16, 0, 8), 0x100), "bdnz       0x108");
    assert_eq!(fmt_at(bc(18, 0, 8), 0x100), "bdz        0x108");
    // [PPC-Book1 p:153 s:B.2.2] bdnzt eq,target = bc 8,2,target.
    assert_eq!(fmt_at(bc(8, 2, 8), 0x100), "bdnzt      eq, 0x108");
    // bdnzt 4*cr5+eq,target = bc 8,22,target.
    assert_eq!(fmt_at(bc(8, 22, 8), 0x100), "bdnzt      4*cr5+eq, 0x108");
    // Hint bits are dropped: bc 15,0 renders like bc 12,0.
    assert_eq!(fmt_at(bc(15, 0, 8), 0x100), "blt        0x108");
}

#[test]
fn fmt_conditional_returns_and_ctr_branches() {
    let bclr = |bo: u32, bi: u32, lk: u32| (19u32 << 26) | (bo << 21) | (bi << 16) | (16 << 1) | lk;
    let bcctr =
        |bo: u32, bi: u32, lk: u32| (19u32 << 26) | (bo << 21) | (bi << 16) | (528 << 1) | lk;
    assert_eq!(fmt(bclr(4, 2, 0)), "bnelr");
    assert_eq!(fmt(bclr(12, 30, 0)), "beqlr      cr7");
    assert_eq!(fmt(bclr(16, 0, 0)), "bdnzlr");
    assert_eq!(fmt(bclr(8, 2, 0)), "bdnztlr    eq");
    assert_eq!(fmt(bclr(12, 0, 1)), "bltlrl");
    // [PPC-Book1 p:154 s:B.2.3] bgtctrl cr4 = bcctrl 12,17.
    assert_eq!(fmt(bcctr(12, 17, 1)), "bgtctrl    cr4");
    // CTR-decrement bcctr forms have no mnemonic: canonical.
    assert_eq!(fmt(bcctr(16, 0, 0)), "bcctr      16, 0");
}

#[test]
fn fmt_branch_nonzero_bi_on_ignored_field_stays_canonical() {
    // BO=20 ignores BI; a nonstandard BI != 0 must not be guessed
    // into `blr`.
    let raw = (19 << 26) | (20 << 21) | (5 << 16) | (16 << 1);
    assert_eq!(fmt(raw), "bclr       20, 5");
}

#[test]
fn fmt_nop_li_lis() {
    // [PPC-Book1 p:162 s:B.9] nop = ori 0,0,0.
    assert_eq!(fmt(0x6000_0000), "nop");
    // ori with any nonzero field is NOT nop.
    assert_eq!(fmt(enc_d(24, 0, 0, 1)), "ori        r0, r0, 0x1");
    // li r3, -1 = addi r3, 0, -1.
    assert_eq!(fmt(enc_d(14, 3, 0, 0xFFFF)), "li         r3, -1");
    // lis r4, 0x8001 = addis r4, 0, 0x8001 -- hex, not -32767.
    assert_eq!(fmt(enc_d(15, 4, 0, 0x8001)), "lis        r4, 0x8001");
}

#[test]
fn fmt_mr_and_not() {
    // mr r4, r5 = or r4, r5, r5.
    assert_eq!(fmt(enc_x(31, 5, 4, 5, 444, false)), "mr         r4, r5");
    assert_eq!(fmt(enc_x(31, 5, 4, 5, 444, true)), "mr.        r4, r5");
    // not r4, r5 = nor r4, r5, r5.
    assert_eq!(fmt(enc_x(31, 5, 4, 5, 124, false)), "not        r4, r5");
    // or with rs != rb stays canonical.
    assert_eq!(fmt(enc_x(31, 5, 4, 6, 444, false)), "or         r4, r5, r6");
}

#[test]
fn fmt_shift_clear_simplifications() {
    // slwi r3, r5, 4 = rlwinm r3, r5, 4, 0, 27.
    assert_eq!(
        fmt(enc_m(21, 5, 3, 4, 0, 27, false)),
        "slwi       r3, r5, 4"
    );
    // srwi r3, r5, 4 = rlwinm r3, r5, 28, 4, 31.
    assert_eq!(
        fmt(enc_m(21, 5, 3, 28, 4, 31, false)),
        "srwi       r3, r5, 4"
    );
    // clrlwi r3, r5, 16 = rlwinm r3, r5, 0, 16, 31.
    assert_eq!(
        fmt(enc_m(21, 5, 3, 0, 16, 31, true)),
        "clrlwi.    r3, r5, 16"
    );
    // Non-gated rlwinm stays canonical.
    assert_eq!(
        fmt(enc_m(21, 5, 3, 4, 8, 27, false)),
        "rlwinm     r3, r5, 4, 8, 27"
    );
    // sldi r3, r5, 8 = rldicr r3, r5, 8, 55 (MD-form xo=1; me=55
    // splits into lo=23 at bits 6..10 and hi bit at raw bit 5).
    let sldi = (30 << 26) | (5 << 21) | (3 << 16) | (8 << 11) | (23 << 6) | (1 << 5) | (1 << 2);
    assert_eq!(fmt(sldi), "sldi       r3, r5, 8");
    // srdi r3, r5, 8 = rldicl r3, r5, 56, 8 (sh=56 splits: lo=24,
    // hi bit raw[1]=1).
    let srdi = (30 << 26) | (5 << 21) | (3 << 16) | (24 << 11) | (8 << 6) | (1 << 1);
    assert_eq!(fmt(srdi), "srdi       r3, r5, 8");
    // clrldi r3, r5, 32 = rldicl r3, r5, 0, 32 (mb=32: lo=0, hi
    // bit raw[5]=1).
    let clrldi = (30 << 26) | (5 << 21) | (3 << 16) | (1 << 5);
    assert_eq!(fmt(clrldi), "clrldi     r3, r5, 32");
}

#[test]
fn fmt_cr_logical_bit_names() {
    // crxor 4*cr1+eq, 4*cr2+lt, so  (bt=6, ba=8, bb=3).
    let raw = (19 << 26) | (6 << 21) | (8 << 16) | (3 << 11) | (193 << 1);
    assert_eq!(fmt(raw), "crxor      4*cr1+eq, 4*cr2+lt, so");
    // creqv lt, lt, lt (all cr0 -> bare condition names).
    let raw = (19 << 26) | (289 << 1);
    assert_eq!(fmt(raw), "creqv      lt, lt, lt");
}

#[test]
fn fmt_mcrf_and_mcrxr() {
    let mcrf = (19 << 26) | (7 << 23) | (2 << 18);
    assert_eq!(fmt(mcrf), "mcrf       cr7, cr2");
    let mcrxr = enc_x(31, 1 << 2, 0, 0, 512, false);
    assert_eq!(fmt(mcrxr), "mcrxr      cr1");
}

#[test]
fn fmt_indexed_and_atomic() {
    assert_eq!(fmt(enc_x(31, 3, 4, 5, 23, false)), "lwzx       r3, r4, r5");
    assert_eq!(fmt(enc_x(31, 3, 4, 5, 84, false)), "ldarx      r3, r4, r5");
    // stwcx. always carries the dot.
    assert_eq!(fmt(enc_x(31, 3, 4, 5, 150, true)), "stwcx.     r3, r4, r5");
}

#[test]
fn fmt_string_moves_nb_decimal() {
    // lswi r5, r4, 8.
    assert_eq!(fmt(enc_x(31, 5, 4, 8, 597, false)), "lswi       r5, r4, 8");
}

#[test]
fn fmt_spr_moves() {
    // Firmware-observed literals: mfvrsave r0 / mtvrsave r0.
    assert_eq!(fmt(0x7c0042a6), "mfvrsave   r0");
    assert_eq!(fmt(0x7c0043a6), "mtvrsave   r0");
    // mflr r0 = 0x7c0802a6 (ubiquitous prologue word).
    assert_eq!(fmt(0x7c0802a6), "mflr       r0");
    // mtlr r0 = 0x7c0803a6.
    assert_eq!(fmt(0x7c0803a6), "mtlr       r0");
    // mtctr r12 = 0x7d8903a6 (import-stub tail).
    assert_eq!(fmt(0x7d8903a6), "mtctr      r12");
}

#[test]
fn fmt_cr_field_moves_crm_hex() {
    // mtcrf 0xff, r0.
    let raw = (31 << 26) | (0xFF << 12) | (144 << 1);
    assert_eq!(fmt(raw), "mtcrf      0xff, r0");
    // mfocrf r3, 0x80.
    let raw = (31 << 26) | (3 << 21) | (1 << 20) | (0x80 << 12) | (19 << 1);
    assert_eq!(fmt(raw), "mfocrf     r3, 0x80");
}

#[test]
fn fmt_fp_loads_use_f_registers() {
    assert_eq!(fmt(enc_d(48, 1, 3, 8)), "lfs        f1, 8(r3)");
    assert_eq!(fmt(enc_d(54, 2, 1, 0xFFF8)), "stfd       f2, -8(r1)");
    assert_eq!(fmt(enc_x(31, 1, 3, 4, 535, false)), "lfsx       f1, r3, r4");
    assert_eq!(fmt(enc_x(31, 1, 3, 4, 983, false)), "stfiwx     f1, r3, r4");
}

#[test]
fn fmt_vector_memory_uses_v_registers() {
    assert_eq!(fmt(enc_x(31, 2, 0, 9, 103, false)), "lvx        v2, r0, r9");
    assert_eq!(fmt(enc_x(31, 2, 0, 9, 231, false)), "stvx       v2, r0, r9");
    assert_eq!(fmt(enc_x(31, 2, 3, 9, 775, false)), "stvlx      v2, r3, r9");
    assert_eq!(fmt(enc_x(31, 2, 3, 9, 6, false)), "lvsl       v2, r3, r9");
}

#[test]
fn fmt_vxor_and_vsldoi() {
    // vxor v0, v1, v2.
    let raw = (4 << 26) | (1 << 16) | (2 << 11) | 0x4c4;
    assert_eq!(fmt(raw), "vxor       v0, v1, v2");
    // vsldoi v3, v4, v5, 8.
    let raw = (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (8 << 6) | 0x2c;
    assert_eq!(fmt(raw), "vsldoi     v3, v4, v5, 8");
}

#[test]
fn fmt_trap_and_system() {
    assert_eq!(fmt(enc_x(31, 31, 3, 4, 4, false)), "tw         31, r3, r4");
    // sc (lev=0) renders bare; the form marker is raw bit 1.
    assert_eq!(fmt((17 << 26) | 2), "sc");
    assert_eq!(fmt((17 << 26) | (1 << 5) | 2), "sc         1");
    assert_eq!(fmt(enc_x(31, 0, 3, 4, 1014, false)), "dcbz       r3, r4");
}

#[test]
fn fmt_byte_reverse() {
    assert_eq!(fmt(enc_x(31, 3, 4, 5, 534, false)), "lwbrx      r3, r4, r5");
    assert_eq!(fmt(enc_x(31, 3, 4, 5, 660, false)), "sdbrx      r3, r4, r5");
}

#[test]
fn fmt_quickened_forms_render_extended_mnemonics() {
    assert_eq!(
        fmt_insn(PpuInstruction::Li { rt: 3, imm: -1 }),
        "li         r3, -1"
    );
    assert_eq!(
        fmt_insn(PpuInstruction::Mr { ra: 4, rs: 5 }),
        "mr         r4, r5"
    );
    assert_eq!(fmt_insn(PpuInstruction::Nop), "nop");
    assert_eq!(
        fmt_insn(PpuInstruction::CmpwZero { bf: 7, ra: 3 }),
        "cmpwi      cr7, r3, 0"
    );
    assert_eq!(
        fmt_insn(PpuInstruction::Sldi { ra: 3, rs: 4, n: 2 }),
        "sldi       r3, r4, 2"
    );
}

#[test]
fn fmt_superinstructions_render_fused_halves() {
    assert_eq!(
        fmt_insn(PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 8,
            bf: 7,
            cmp_imm: 0,
        }),
        "lwz        r3, 8(r1); cmpwi cr7, r3, 0"
    );
    assert_eq!(
        fmt_insn(PpuInstruction::MflrStd {
            rt: 0,
            ra_store: 1,
            store_offset: 16,
        }),
        "mflr       r0; std r0, 16(r1)"
    );
    assert_eq!(
        fmt_insn(PpuInstruction::StdStd {
            rs1: 30,
            rs2: 31,
            ra: 1,
            offset1: -16,
        }),
        "std        r30, -16(r1); std r31, -8(r1)"
    );
    // The fused bc resolves relative to addr+4 (fmt_insn renders at
    // 0x1_0000, so the bc half sits at 0x1_0004).
    assert_eq!(
        fmt_insn(PpuInstruction::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 0,
            bo: 12,
            bi: 2,
            target_offset: 0x20,
        }),
        "cmpwi      r3, 0; bc 12, 2, 0x10024"
    );
    assert_eq!(fmt_insn(PpuInstruction::Consumed), ".consumed");
}

#[test]
fn fmt_vx_family_named_ops() {
    // vaddubm v3, v4, v5 (VX xo=0).
    let raw = (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11);
    assert_eq!(fmt(raw), "vaddubm    v3, v4, v5");
    // vcmpequw. v0, v1, v2: VXR Rc=1 form 1024|134 appends the dot.
    let raw = (4 << 26) | (1 << 16) | (2 << 11) | (1024 | 134);
    assert_eq!(fmt(raw), "vcmpequw.  v0, v1, v2");
    // vspltw v2, v3, 1: UIMM rides the va slot.
    let raw = (4 << 26) | (2 << 21) | (1 << 16) | (3 << 11) | 652;
    assert_eq!(fmt(raw), "vspltw     v2, v3, 1");
    // vspltisb v4, -1: 5-bit SIMM sign-extends.
    let raw = (4 << 26) | (4 << 21) | (0x1F << 16) | 780;
    assert_eq!(fmt(raw), "vspltisb   v4, -1");
    // vupkhsb v1, v2: unary, va slot reserved.
    let raw = (4 << 26) | (1 << 21) | (2 << 11) | 526;
    assert_eq!(fmt(raw), "vupkhsb    v1, v2");
}

#[test]
fn fmt_va_family_named_ops() {
    // vperm v0, v1, v2, v3 (VA xo=43).
    let raw = (4 << 26) | (1 << 16) | (2 << 11) | (3 << 6) | 43;
    assert_eq!(fmt(raw), "vperm      v0, v1, v2, v3");
    // vmaddfp v4, v5, v6, v7: assembly order vD,vA,vC,vB.
    let raw = (4 << 26) | (4 << 21) | (5 << 16) | (7 << 11) | (6 << 6) | 46;
    assert_eq!(fmt(raw), "vmaddfp    v4, v5, v6, v7");
}

#[test]
fn fmt_fp_family_named_ops() {
    // fmadd f1, f2, f3, f4: A-form, assembly order FRT,FRA,FRC,FRB.
    let raw = (63 << 26) | (1 << 21) | (2 << 16) | (4 << 11) | (3 << 6) | (29 << 1);
    assert_eq!(fmt(raw), "fmadd      f1, f2, f3, f4");
    // fadds. f1, f2, f3 (primary 59, Rc).
    let raw = (59 << 26) | (1 << 21) | (2 << 16) | (3 << 11) | (21 << 1) | 1;
    assert_eq!(fmt(raw), "fadds.     f1, f2, f3");
    // fmul f1, f2, f3: FRC operand, FRB reserved.
    let raw = (63 << 26) | (1 << 21) | (2 << 16) | (3 << 6) | (25 << 1);
    assert_eq!(fmt(raw), "fmul       f1, f2, f3");
    // fmr f0, f5.
    let raw = (63 << 26) | (5 << 11) | (72 << 1);
    assert_eq!(fmt(raw), "fmr        f0, f5");
    // fcmpu cr7, f1, f2.
    let raw = (63 << 26) | (28 << 21) | (1 << 16) | (2 << 11);
    assert_eq!(fmt(raw), "fcmpu      cr7, f1, f2");
    // mffs f0.
    let raw = (63 << 26) | (583 << 1);
    assert_eq!(fmt(raw), "mffs       f0");
    // mtfsf 0xff, f3 (XFL-form: FLM spans the frt/fra slots).
    let raw = (63 << 26) | (0xFF << 17) | (3 << 11) | (711 << 1);
    assert_eq!(fmt(raw), "mtfsf      0xff, f3");
}

/// Anchor words: hand-assembled per the PEM / Book I field layouts,
/// independent of the op-enum discriminants, locked as
/// `decode + format == expected text`.
#[test]
fn fmt_anchor_words() {
    // vaddubm v1, v2, v3 = 0x10221800.
    assert_eq!(fmt(0x1022_1800), "vaddubm    v1, v2, v3");
    // vcmpequw. v0, v1, v2 = 0x10011486 (xo 1158 = 0x486).
    assert_eq!(fmt(0x1001_1486), "vcmpequw.  v0, v1, v2");
    // vperm v0, v1, v2, v3 = 0x100110eb.
    assert_eq!(fmt(0x1001_10EB), "vperm      v0, v1, v2, v3");
    // vspltisw v0, -1 = 0x101f038c (xo 908 = 0x38c).
    assert_eq!(fmt(0x101F_038C), "vspltisw   v0, -1");
    // fmr f1, f2 = 0xfc201090 (xo 72 = 0x48 -> 0x090 at <<1).
    assert_eq!(fmt(0xFC20_1090), "fmr        f1, f2");
    // fadds f1, f2, f3 = 0xec22182a (xo 21 -> 0x2a at <<1).
    assert_eq!(fmt(0xEC22_182A), "fadds      f1, f2, f3");
    // fmadd f1, f2, f3, f4 = 0xfc2220fa (frc=3 at <<6, xo5 29).
    assert_eq!(fmt(0xFC22_20FA), "fmadd      f1, f2, f3, f4");
    // mffs f0 = 0xfc00048e.
    assert_eq!(fmt(0xFC00_048E), "mffs       f0");
}

/// Undocumented primary-59/63 XOs now reject at decode instead of
/// fabricating a stub the executor silently retires.
#[test]
fn fmt_undocumented_fp_xo_rejects() {
    // Primary 63 with xo=1 (no documented op, low5=1 not A-form).
    let raw = (63u32 << 26) | (1 << 1);
    assert!(decode(raw).is_err(), "xo=1 under primary 63 must reject");
    // Primary 59 with xo5=23 (no documented op).
    let raw = (59u32 << 26) | (23 << 1);
    assert!(decode(raw).is_err(), "xo5=23 under primary 59 must reject");
}

#[test]
fn fmt_branch_targets_symbolize_with_function_map() {
    use crate::funcmap::{FunctionMap, FunctionName, FunctionOrigin, FunctionSpan};
    let map = FunctionMap {
        functions: vec![FunctionSpan {
            start: 0x10080,
            end: 0x100C0,
            name: FunctionName::Known("entry"),
            origin: FunctionOrigin::EntryOpd,
        }],
        truncated: false,
    };
    let sym = |raw: u32, addr: u64| {
        let insn = decode(raw).unwrap();
        AsmText {
            insn: &insn,
            addr,
            symbols: Some(&map),
        }
        .to_string()
    };
    // bl onto the span start: name, no +0x0.
    assert_eq!(
        sym((18 << 26) | 0x80 | 1, 0x10000),
        "bl         0x10080 <entry>"
    );
    // Conditional branch into the middle: +delta suffix.
    let bc = (16 << 26) | (12 << 21) | (2 << 16) | 0x90;
    assert_eq!(sym(bc, 0x10000), "beq        0x10090 <entry+0x10>");
    // Target outside any span: bare hex.
    assert_eq!(sym((18 << 26) | 0x200, 0x10000), "b          0x10200");
}

// -- Totality sweep --

/// Every word the decoder accepts must render without panicking and
/// without `Debug`-derive leakage.
#[test]
fn totality_sweep_no_debug_leakage() {
    // Sweep a broad sample: every primary opcode with a spread of
    // field patterns. Decode rejections are fine; accepted words
    // must format cleanly.
    let mut checked = 0usize;
    for primary in 0u32..64 {
        for pattern in [
            0x0000_0000u32,
            0x03FF_FFFE,
            0x0064_1234,
            0x007F_0851,
            0x03E0_F001,
            0x0123_4567,
        ] {
            let raw = (primary << 26) | (pattern & 0x03FF_FFFF);
            if let Ok(insn) = decode(raw) {
                let text = AsmText {
                    insn: &insn,
                    addr: 0x10000,
                    symbols: None,
                }
                .to_string();
                assert!(
                    !text.contains('{') && !text.contains('}'),
                    "Debug leakage for 0x{raw:08x}: {text}"
                );
                assert!(text.is_ascii(), "non-ASCII output for 0x{raw:08x}: {text}");
                assert!(!text.is_empty(), "empty render for 0x{raw:08x}");
                checked += 1;
            }
        }
    }
    assert!(checked > 50, "sweep decoded only {checked} words");
}

/// No rendered line carries trailing whitespace (operand-less
/// mnemonics skip the column padding).
#[test]
fn no_trailing_whitespace() {
    for insn in [
        PpuInstruction::Nop,
        PpuInstruction::Consumed,
        PpuInstruction::Sc { lev: 0 },
    ] {
        let text = fmt_insn(insn);
        assert_eq!(text, text.trim_end(), "trailing space in {text:?}");
    }
}
