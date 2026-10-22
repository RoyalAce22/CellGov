//! The instruction-form field builders: each places its operands at
//! the bit positions of its form.

pub(super) const fn p(primary: u32) -> u32 {
    primary << 26
}
pub(super) const fn rt(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 21
}
pub(super) const fn ra(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 16
}
pub(super) const fn rb(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 11
}
pub(super) const fn slot_6(v: u8) -> u32 {
    ((v as u32) & 0x1F) << 6
}
pub(super) const fn xo_10(xo: u32, rc: bool) -> u32 {
    (xo << 1) | rc as u32
}
const fn xo_9(xo: u32, oe: bool, rc: bool) -> u32 {
    ((oe as u32) << 10) | (xo << 1) | rc as u32
}

/// [PPC-Book1 p:8 s:1.7.4 D-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) D/SI(16:31).
pub(super) const fn d(primary: u32, t: u8, a: u8, imm: u16) -> u32 {
    p(primary) | rt(t) | ra(a) | imm as u32
}

/// [PPC-Book1 p:8 s:1.7.5 DS-Form] DS(16:29) || 0b00; XO(30:31) selects the op.
pub(super) const fn ds(primary: u32, t: u8, a: u8, imm: i16, sub: u32) -> u32 {
    p(primary) | rt(t) | ra(a) | ((imm as u16 as u32) & 0xFFFC) | sub
}

/// [PPC-Book1 p:9 s:1.7.6 X-Form] OPCD(0:5) RT/RS(6:10) RA(11:15) RB(16:20) XO(21:30) Rc(31).
pub(super) const fn x(t: u8, a: u8, b: u8, xo: u32, rc: bool) -> u32 {
    p(31) | rt(t) | ra(a) | rb(b) | xo_10(xo, rc)
}

/// [PPC-Book1 p:9 s:1.7.11 XO-Form] OPCD RT RA RB OE(21) XO(22:30) Rc(31).
pub(super) const fn xo(t: u8, a: u8, b: u8, xo: u32, oe: bool, rc: bool) -> u32 {
    p(31) | rt(t) | ra(a) | rb(b) | xo_9(xo, oe, rc)
}

/// [PPC-Book1 p:9 s:1.7.8 XFX-Form] spr(11:20) is the SPR number with its halves swapped: the low five bits ride in the RA slot, the high five in the RB slot.
pub(super) const fn xfx_spr(t: u8, spr: u16, xo: u32) -> u32 {
    let low = (spr as u32) & 0x1F;
    let high = ((spr as u32) >> 5) & 0x1F;
    p(31) | rt(t) | (low << 16) | (high << 11) | xo_10(xo, false)
}

/// [PPC-Book1 p:9 s:1.7.7 XL-Form] OPCD BT/BO BA/BI BB(16:20) XO(21:30) LK(31).
pub(super) const fn xl(t: u8, a: u8, b: u8, xo: u32, lk: bool) -> u32 {
    p(19) | rt(t) | ra(a) | rb(b) | xo_10(xo, lk)
}

/// [PPC-Book1 p:10 s:1.7.13 M-Form] OPCD RS RA RB/SH MB(21:25) ME(26:30) Rc.
pub(super) const fn m(primary: u32, s: u8, a: u8, third: u8, mb: u8, me: u8, rc: bool) -> u32 {
    p(primary) | rt(s) | ra(a) | rb(third) | slot_6(mb) | (((me as u32) & 0x1F) << 1) | rc as u32
}

/// [PPC-Book1 p:10 s:1.7.14 MD-Form] OPCD RS RA sh(16:20) mb(21:25,26) XO(27:29) sh(30) Rc.
pub(super) const fn md(s: u8, a: u8, sh: u8, mask: u8, xo: u32, rc: bool) -> u32 {
    let sh_hi = (((sh as u32) >> 5) & 1) << 1;
    let mask_hi = (((mask as u32) >> 5) & 1) << 5;
    p(30) | rt(s) | ra(a) | rb(sh) | slot_6(mask) | mask_hi | (xo << 2) | sh_hi | rc as u32
}

/// [PPC-Book1 p:75 s:3.3.12] MDS-form: RB(16:20) mb/me(21:26) XO(27:30) Rc.
pub(super) const fn mds(s: u8, a: u8, b: u8, mask: u8, xo: u32, rc: bool) -> u32 {
    let mask_hi = (((mask as u32) >> 5) & 1) << 5;
    p(30) | rt(s) | ra(a) | rb(b) | slot_6(mask) | mask_hi | (xo << 1) | rc as u32
}

/// [PPC-Book1 p:10 s:1.7.12 A-Form] OPCD FRT FRA FRB FRC(21:25) XO(26:30) Rc(31).
pub(super) const fn a_form(primary: u32, t: u8, a: u8, b: u8, c: u8, xo: u32, rc: bool) -> u32 {
    p(primary) | rt(t) | ra(a) | rb(b) | slot_6(c) | (xo << 1) | rc as u32
}
