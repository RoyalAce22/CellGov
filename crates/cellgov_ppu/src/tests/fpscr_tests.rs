//! The FPSCR move instructions, run from their encoded words: the fields
//! they write, the bits they keep, and the summary bits they recompute.

use crate::exec::{execute, ExecuteVerdict};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

fn run(state: &mut PpuState, raw: u32) {
    let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
    let page = [0u8; 0x100];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let verdict = execute(
        &insn,
        state,
        cellgov_event::UnitId::new(0),
        &views,
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Continue, "0x{raw:08x}");
}

/// [PPC-Book1 p:120 s:4.6.8] mffs is 63/583, X-form.
fn mffs(frt: u32, rc: u32) -> u32 {
    (63 << 26) | (frt << 21) | (583 << 1) | rc
}
/// [PPC-Book1 p:120 s:4.6.8] mcrfs is 63/64 with BF at bits 6:8 and BFA at 11:13.
fn mcrfs(bf: u32, bfa: u32) -> u32 {
    (63 << 26) | (bf << 23) | (bfa << 18) | (64 << 1)
}
/// [PPC-Book1 p:121 s:4.6.8] mtfsfi is 63/134 with BF at bits 6:8 and U at 16:19.
fn mtfsfi(bf: u32, u: u32, rc: u32) -> u32 {
    (63 << 26) | (bf << 23) | (u << 12) | (134 << 1) | rc
}
/// [PPC-Book1 p:121 s:4.6.8] mtfsf is 63/711, XFL-form, with FLM at bits 7:14.
fn mtfsf(flm: u32, frb: u32, rc: u32) -> u32 {
    (63 << 26) | (flm << 17) | (frb << 11) | (711 << 1) | rc
}
/// [PPC-Book1 p:122 s:4.6.8] mtfsb0 is 63/70 with BT at bits 6:10.
fn mtfsb0(bt: u32, rc: u32) -> u32 {
    (63 << 26) | (bt << 21) | (70 << 1) | rc
}
/// [PPC-Book1 p:122 s:4.6.8] mtfsb1 is 63/38 with BT at bits 6:10.
fn mtfsb1(bt: u32, rc: u32) -> u32 {
    (63 << 26) | (bt << 21) | (38 << 1) | rc
}

/// A state whose FPSCR is `v`, with VX and FEX consistent with it.
fn with_fpscr(v: u32) -> PpuState {
    let mut s = PpuState::new();
    s.set_fpscr(super::with_summaries(v));
    s
}

#[test]
fn mffs_places_the_fpscr_in_the_low_word() {
    let mut s = with_fpscr(0x8200_4001);
    s.set_fpr(7, 0x1234_5678_9ABC_DEF0);
    s.set_fpr(8, 0x5555_5555_5555_5555);
    run(&mut s, mffs(7, 0));
    assert_eq!(s.fpr[7], 0xFFF8_0000_8200_4001);
    assert_eq!(s.fpr[8], 0x5555_5555_5555_5555);
    assert_eq!(s.fpscr(), 0x8200_4001);
}

#[test]
fn mtfsfi_writes_one_field_and_keeps_the_others() {
    for bf in 1..8u32 {
        let mut s = with_fpscr(0);
        run(&mut s, mtfsfi(bf, 0b1010, 0));
        let want = super::with_summaries(0b1010 << (28 - 4 * bf));
        assert_eq!(s.fpscr(), want, "field {bf}");
    }
    let mut s = with_fpscr(0x0000_00FC);
    run(&mut s, mtfsfi(7, 0b0011, 0));
    assert_eq!(
        s.fpscr(),
        0x0000_00F3,
        "field 7 replaced whole, field 6 kept"
    );
}

#[test]
fn mtfsfi_to_field_0_takes_fx_and_ox_from_u_and_derives_fex_and_vx() {
    // U = 1111: FX and OX come from U; FEX and VX stay 0 because no
    // invalid bit and no enable is set.
    let mut s = with_fpscr(0);
    run(&mut s, mtfsfi(0, 0b1111, 0));
    assert_eq!(s.fpscr(), 0x9000_0000);
    // U = 0000 clears FX even though VXSNAN keeps VX at 1.
    let mut s = with_fpscr(0x8100_0000);
    run(&mut s, mtfsfi(0, 0, 0));
    assert_eq!(s.fpscr(), 0x2100_0000);
}

#[test]
fn mtfsf_writes_the_fields_flm_selects_from_the_low_word_of_frb() {
    let mut s = with_fpscr(0x0000_0000);
    s.set_fpr(4, 0xFFFF_FFFF_0000_0F01);
    run(&mut s, mtfsf(0b0000_0011, 4, 0));
    assert_eq!(s.fpscr(), 0x0000_0001, "field 6 from FRB, field 7 from FRB");
    let mut s = with_fpscr(0x0000_0003);
    s.set_fpr(4, 0x0000_0000_8000_0000);
    run(&mut s, mtfsf(0b1000_0000, 4, 0));
    assert_eq!(
        s.fpscr(),
        0x8000_0003,
        "field 0 only; FRB's high word ignored"
    );
    let mut s = with_fpscr(0x0000_0000);
    s.set_fpr(4, 0x0000_0000_6000_0000);
    run(&mut s, mtfsf(0xFF, 4, 0));
    assert_eq!(s.fpscr(), 0x0000_0000, "FEX and VX are not taken from FRB");
}

#[test]
fn mtfsb1_sets_fx_only_when_an_exception_bit_turns_on() {
    // OX (bit 3) turns on: FX follows.
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(3, 0));
    assert_eq!(s.fpscr(), 0x9000_0000);
    // VXCVI (bit 23) turns on: FX and VX follow.
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(23, 0));
    assert_eq!(s.fpscr(), 0xA000_0100);
    // FR (bit 13) is a status bit: FX stays 0.
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(13, 0));
    assert_eq!(s.fpscr(), 0x0004_0000);
    // OX already on: FX stays as it was.
    let mut s = with_fpscr(0x1000_0000);
    run(&mut s, mtfsb1(3, 0));
    assert_eq!(s.fpscr(), 0x1000_0000);
}

#[test]
fn mtfsb_cannot_set_or_reset_fex_and_vx() {
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(1, 0));
    run(&mut s, mtfsb1(2, 0));
    assert_eq!(s.fpscr(), 0);
    let mut s = with_fpscr(0x0100_0000);
    assert_eq!(s.fpscr(), 0x2100_0000, "VXSNAN sets VX");
    run(&mut s, mtfsb0(2, 0));
    assert_eq!(s.fpscr(), 0x2100_0000);
    run(&mut s, mtfsb0(7, 0));
    assert_eq!(s.fpscr(), 0, "clearing the last invalid bit clears VX");
}

#[test]
fn an_enabled_exception_sets_fex() {
    // XE (bit 28) enabled, then XX (bit 6) turned on.
    let mut s = with_fpscr(0x0000_0008);
    run(&mut s, mtfsb1(6, 0));
    assert_eq!(s.fpscr(), 0xC200_0008);
    // VE (bit 24) enabled and an invalid bit on: VX, then FEX.
    let s = with_fpscr(0x0080_0080);
    assert_eq!(s.fpscr(), 0x6080_0080);
}

#[test]
fn mcrfs_copies_a_field_and_clears_only_its_exception_bits() {
    // Field 0: FX and OX clear; FEX and VX are recomputed.
    let mut s = with_fpscr(0x9100_0000);
    run(&mut s, mcrfs(5, 0));
    assert_eq!(s.cr_field(5), 0b1011);
    assert_eq!(s.fpscr(), 0x2100_0000, "VXSNAN keeps VX");
    // Field 3 holds VXVC, FR, FI, C: only VXVC is an exception bit.
    let mut s = with_fpscr(0x000F_0000);
    run(&mut s, mcrfs(2, 3));
    assert_eq!(s.cr_field(2), 0b1111);
    assert_eq!(s.fpscr(), 0x0007_0000);
    // Field 7 holds control bits: nothing clears.
    let mut s = with_fpscr(0x0000_000B);
    run(&mut s, mcrfs(0, 7));
    assert_eq!(s.cr_field(0), 0b1011);
    assert_eq!(s.fpscr(), 0x0000_000B);
}

#[test]
fn a_record_form_copies_fx_fex_vx_ox_to_cr1() {
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(3, 1));
    assert_eq!(s.cr_field(1), 0b1001);
    let mut s = with_fpscr(0);
    run(&mut s, mtfsb1(3, 0));
    assert_eq!(s.cr_field(1), 0, "Rc=0 leaves CR1");
    for raw in [
        mffs(1, 1),
        mtfsfi(7, 1, 1),
        mtfsf(0x01, 2, 1),
        mtfsb0(31, 1),
    ] {
        let mut s = with_fpscr(0xB100_0000);
        run(&mut s, raw);
        assert_eq!(s.cr_field(1), super::cr1_nibble(s.fpscr()), "0x{raw:08x}");
        assert_ne!(s.cr_field(1), 0, "0x{raw:08x}");
    }
}

#[test]
fn an_arithmetic_record_form_copies_the_fpscr_to_cr1() {
    // [PPC-Book1 p:111 s:4.6.5.1] fadd is 63/21 and fadds 59/21, A-form.
    let fadd = |primary: u32, rc: u32| {
        (primary << 26) | (1 << 21) | (2 << 16) | (3 << 11) | (21 << 1) | rc
    };
    for primary in [63, 59] {
        let mut s = with_fpscr(0x9000_0000);
        s.set_fpr(2, 1.5f64.to_bits());
        s.set_fpr(3, 2.0f64.to_bits());
        run(&mut s, fadd(primary, 1));
        assert_eq!(s.cr_field(1), 0b1001, "primary {primary}");
        let mut s = with_fpscr(0x9000_0000);
        run(&mut s, fadd(primary, 0));
        assert_eq!(s.cr_field(1), 0, "primary {primary} Rc=0");
    }
}

#[test]
fn a_compare_or_mcrfs_leaves_cr1_whatever_bit_31_holds() {
    // [PPC-Book1 p:119 s:4.6.7] fcmpu is 63/0 with bit 31 reserved.
    let mut s = with_fpscr(0x9000_0000);
    s.set_fpr(2, 1.0f64.to_bits());
    s.set_fpr(3, 2.0f64.to_bits());
    run(&mut s, (63 << 26) | (6 << 23) | (2 << 16) | (3 << 11) | 1);
    assert_eq!(s.cr_field(6), 0b1000);
    assert_eq!(s.cr_field(1), 0);
    // [PPC-Book1 p:120 s:4.6.8] mcrfs bit 31 is reserved too; FPSCR bits 0:3 stay nonzero after it.
    let mut s = with_fpscr(0x9100_0000);
    run(&mut s, mcrfs(5, 7) | 1);
    assert_eq!(s.cr_field(1), 0, "mcrfs has no record form");
}
