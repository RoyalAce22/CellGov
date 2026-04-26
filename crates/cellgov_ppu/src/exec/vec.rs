//! VMX / AltiVec execution helpers. Vector registers are 128-bit,
//! stored big-endian (byte 0 is the MSB); the per-operation helpers
//! in this module take and return `u128` values in that order.
//!
//! Memory-touching vector ops (`lvx`, `lvlx`, `lvrx`, `stvx`) live in
//! the [`mem`](super::mem) module so all loads / stores share one
//! store-buffer-forward / region-view fallback.

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::state::PpuState;

/// Execute a VX-form VMX instruction (primary=4, 11-bit sub-opcode, 3 registers).
///
/// Pure register-in / register-out: the parent dispatcher peels off
/// `xo == 103` (`lvx`) and routes it to `mem::execute_lvx` before this
/// function is reached.
pub(crate) fn execute_vx(state: &mut PpuState, xo: u16, vt: u8, va: u8, vb: u8) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];

    let result = match xo {
        // -- Integer add/sub --
        0x000 => vadd_bytes(a, b), // vaddubm
        0x040 => vadd_halfs(a, b), // vadduhm
        0x080 => vadd_words(a, b), // vadduwm

        // -- Integer compare --
        0x086 => vcmpequw(a, b), // vcmpequw

        // -- Logical --
        0xac4 => a & b,    // vand
        0x6c4 => a | b,    // vor
        0x4c4 => a ^ b,    // vxor (fallback)
        0x8c4 => a & !b,   // vandc
        0x7c4 => !(a | b), // vnor

        // -- Shift --
        0x284 => vslw(a, b),  // vslw
        0x384 => vsrw(a, b),  // vsrw
        0x484 => vsraw(a, b), // vsraw
        0x444 => vsrah(a, b), // vsrah
        0x304 => vsrab(a, b), // vsrab

        // -- Splat (PPC AltiVec ISA XO values) --
        0x20c => vspltb(b, va), // vspltb (va is byte index)
        0x24c => vsplth(b, va), // vsplth (va is halfword index)
        0x28c => vspltw(b, va), // vspltw (va is word index)
        0x30c => vspltisb(va),  // vspltisb (sign-extended 5-bit imm)
        0x34c => vspltish(va),  // vspltish
        0x38c => vspltisw(va),  // vspltisw

        // -- Merge --
        0x00c => vmrghb(a, b), // vmrghb
        0x04c => vmrghh(a, b), // vmrghh
        0x08c => vmrghw(a, b), // vmrghw
        0x40a => vmrglb(a, b), // vmrglb
        0x44a => vmrglh(a, b), // vmrglh
        0x48a => vmrglw(a, b), // vmrglw

        // -- Multiply --
        0x048 => vmulouh(a, b), // vmulouh

        // -- Subtract --
        0x600 => vsub_ubytes_sat(a, b), // vsububs (saturating)

        // -- Int <-> Float conversions (VX-form, va field is uimm scale) --
        0x34a => vcfsx(b, va), // vcfsx
        0x38a => vcfux(b, va), // vcfux

        _ => {
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(xo as u64));
        }
    };

    state.vr[vt as usize] = result;
    ExecuteVerdict::Continue
}

/// Execute a VA-form VMX instruction (primary=4, 6-bit sub-opcode, 4 registers).
pub(crate) fn execute_va(
    state: &mut PpuState,
    xo: u8,
    vt: u8,
    va: u8,
    vb: u8,
    vc: u8,
) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];
    let c = state.vr[vc as usize];

    let result = match xo {
        0x2a => vsel(a, b, c),  // vsel
        0x2b => vperm(a, b, c), // vperm
        _ => {
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(xo as u64));
        }
    };

    state.vr[vt as usize] = result;
    ExecuteVerdict::Continue
}

/// Execute `vsldoi`. `shb` is the 4-bit byte-shift immediate carved out
/// of the VA-form vc slot by the decoder.
pub(crate) fn execute_vsldoi(
    state: &mut PpuState,
    vt: u8,
    va: u8,
    vb: u8,
    shb: u8,
) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];
    state.vr[vt as usize] = vsldoi(a, b, shb);
    ExecuteVerdict::Continue
}

fn vadd_bytes(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = ab[i].wrapping_add(bb[i]);
    }
    u128::from_be_bytes(r)
}

fn vadd_halfs(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        let av = u16::from_be_bytes([ab[i], ab[i + 1]]);
        let bv = u16::from_be_bytes([bb[i], bb[i + 1]]);
        let rv = av.wrapping_add(bv);
        let [h, l] = rv.to_be_bytes();
        r[i] = h;
        r[i + 1] = l;
    }
    u128::from_be_bytes(r)
}

fn vadd_words(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        let av = u32::from_be_bytes([ab[i], ab[i + 1], ab[i + 2], ab[i + 3]]);
        let bv = u32::from_be_bytes([bb[i], bb[i + 1], bb[i + 2], bb[i + 3]]);
        let rv = av.wrapping_add(bv);
        let bytes = rv.to_be_bytes();
        r[i..i + 4].copy_from_slice(&bytes);
    }
    u128::from_be_bytes(r)
}

fn vcmpequw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        let av = u32::from_be_bytes([ab[i], ab[i + 1], ab[i + 2], ab[i + 3]]);
        let bv = u32::from_be_bytes([bb[i], bb[i + 1], bb[i + 2], bb[i + 3]]);
        let mask: u32 = if av == bv { 0xFFFF_FFFF } else { 0 };
        r[i..i + 4].copy_from_slice(&mask.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vslw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        let av = u32::from_be_bytes([ab[i], ab[i + 1], ab[i + 2], ab[i + 3]]);
        let sh = u32::from_be_bytes([bb[i], bb[i + 1], bb[i + 2], bb[i + 3]]) & 0x1F;
        let rv = av << sh;
        r[i..i + 4].copy_from_slice(&rv.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vsrw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        let av = u32::from_be_bytes([ab[i], ab[i + 1], ab[i + 2], ab[i + 3]]);
        let sh = u32::from_be_bytes([bb[i], bb[i + 1], bb[i + 2], bb[i + 3]]) & 0x1F;
        let rv = av >> sh;
        r[i..i + 4].copy_from_slice(&rv.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vsraw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        let av = i32::from_be_bytes([ab[i], ab[i + 1], ab[i + 2], ab[i + 3]]);
        let sh = u32::from_be_bytes([bb[i], bb[i + 1], bb[i + 2], bb[i + 3]]) & 0x1F;
        let rv = av >> sh;
        r[i..i + 4].copy_from_slice(&rv.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vsrah(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        let av = i16::from_be_bytes([ab[i], ab[i + 1]]);
        let sh = u16::from_be_bytes([bb[i], bb[i + 1]]) & 0xF;
        let rv = av >> sh;
        let [h, l] = rv.to_be_bytes();
        r[i] = h;
        r[i + 1] = l;
    }
    u128::from_be_bytes(r)
}

fn vsrab(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..16 {
        let av = ab[i] as i8;
        let sh = bb[i] & 0x7;
        r[i] = (av >> sh) as u8;
    }
    u128::from_be_bytes(r)
}

fn vspltw(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let start = (idx as usize & 3) * 4;
    let word = u32::from_be_bytes([bb[start], bb[start + 1], bb[start + 2], bb[start + 3]]);
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        r[i..i + 4].copy_from_slice(&word.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vspltisw(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm as i8 | !0x1F_u8 as i8) as i32
    } else {
        imm as i32
    };
    let word = (val as u32).to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(4) {
        r[i..i + 4].copy_from_slice(&word);
    }
    u128::from_be_bytes(r)
}

fn vspltb(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let byte = bb[idx as usize & 0xF];
    u128::from_be_bytes([byte; 16])
}

fn vsplth(b: u128, idx: u8) -> u128 {
    let bb = b.to_be_bytes();
    let start = (idx as usize & 7) * 2;
    let half = u16::from_be_bytes([bb[start], bb[start + 1]]);
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        let bytes = half.to_be_bytes();
        r[i] = bytes[0];
        r[i + 1] = bytes[1];
    }
    u128::from_be_bytes(r)
}

fn vspltisb(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm | 0xE0) as i8
    } else {
        imm as i8
    };
    u128::from_be_bytes([val as u8; 16])
}

fn vspltish(imm: u8) -> u128 {
    let val = if imm & 0x10 != 0 {
        (imm as i8 | !0x1F_u8 as i8) as i16
    } else {
        imm as i16
    };
    let half = (val as u16).to_be_bytes();
    let mut r = [0u8; 16];
    for i in (0..16).step_by(2) {
        r[i] = half[0];
        r[i + 1] = half[1];
    }
    u128::from_be_bytes(r)
}

fn vmrghb(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..8 {
        r[i * 2] = ab[i];
        r[i * 2 + 1] = bb[i];
    }
    u128::from_be_bytes(r)
}

fn vmrghh(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..4 {
        r[i * 4] = ab[i * 2];
        r[i * 4 + 1] = ab[i * 2 + 1];
        r[i * 4 + 2] = bb[i * 2];
        r[i * 4 + 3] = bb[i * 2 + 1];
    }
    u128::from_be_bytes(r)
}

fn vmrghw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    r[0..4].copy_from_slice(&ab[0..4]);
    r[4..8].copy_from_slice(&bb[0..4]);
    r[8..12].copy_from_slice(&ab[4..8]);
    r[12..16].copy_from_slice(&bb[4..8]);
    u128::from_be_bytes(r)
}

fn vmrglb(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..8 {
        r[i * 2] = ab[i + 8];
        r[i * 2 + 1] = bb[i + 8];
    }
    u128::from_be_bytes(r)
}

fn vmrglh(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..4 {
        r[i * 4] = ab[8 + i * 2];
        r[i * 4 + 1] = ab[8 + i * 2 + 1];
        r[i * 4 + 2] = bb[8 + i * 2];
        r[i * 4 + 3] = bb[8 + i * 2 + 1];
    }
    u128::from_be_bytes(r)
}

fn vmrglw(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    r[0..4].copy_from_slice(&ab[8..12]);
    r[4..8].copy_from_slice(&bb[8..12]);
    r[8..12].copy_from_slice(&ab[12..16]);
    r[12..16].copy_from_slice(&bb[12..16]);
    u128::from_be_bytes(r)
}

fn vmulouh(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..4 {
        let ah = u16::from_be_bytes([ab[i * 4 + 2], ab[i * 4 + 3]]) as u32;
        let bh = u16::from_be_bytes([bb[i * 4 + 2], bb[i * 4 + 3]]) as u32;
        let prod = ah * bh;
        r[i * 4..i * 4 + 4].copy_from_slice(&prod.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vsub_ubytes_sat(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = ab[i].saturating_sub(bb[i]);
    }
    u128::from_be_bytes(r)
}

fn vsel(a: u128, b: u128, c: u128) -> u128 {
    // Per-bit mux: c_bit ? b_bit : a_bit.
    (a & !c) | (b & c)
}

fn vperm(a: u128, b: u128, c: u128) -> u128 {
    // Low 5 bits of each c byte index the a:b concatenation (32 bytes).
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let cb = c.to_be_bytes();
    let mut concat = [0u8; 32];
    concat[0..16].copy_from_slice(&ab);
    concat[16..32].copy_from_slice(&bb);
    let mut r = [0u8; 16];
    for i in 0..16 {
        let idx = (cb[i] & 0x1F) as usize;
        r[i] = concat[idx];
    }
    u128::from_be_bytes(r)
}

fn vsldoi(a: u128, b: u128, sh: u8) -> u128 {
    // Left-shift the a:b concatenation by `sh` bytes, return the
    // high 16. PEM constrains SH to 0..=15, so the [shift, shift+15]
    // window always falls inside the 32-byte concat.
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut concat = [0u8; 32];
    concat[0..16].copy_from_slice(&ab);
    concat[16..32].copy_from_slice(&bb);
    let shift = (sh & 0xF) as usize;
    let mut r = [0u8; 16];
    r.copy_from_slice(&concat[shift..shift + 16]);
    u128::from_be_bytes(r)
}

fn vcfsx(b: u128, uimm: u8) -> u128 {
    let bytes = b.to_be_bytes();
    let mut r = [0u8; 16];
    let scale = (1u32 << (uimm & 0x1F)) as f32;
    for i in 0..4 {
        let off = i * 4;
        let v = i32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let f = (v as f32) / scale;
        r[off..off + 4].copy_from_slice(&f.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

fn vcfux(b: u128, uimm: u8) -> u128 {
    let bytes = b.to_be_bytes();
    let mut r = [0u8; 16];
    let scale = (1u32 << (uimm & 0x1F)) as f32;
    for i in 0..4 {
        let off = i * 4;
        let v = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let f = (v as f32) / scale;
        r[off..off + 4].copy_from_slice(&f.to_be_bytes());
    }
    u128::from_be_bytes(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::test_support::exec_no_mem;
    use crate::exec::ExecuteVerdict;
    use crate::instruction::PpuInstruction;

    #[test]
    fn vxor_self_zeros_vector_register() {
        let mut s = PpuState::new();
        s.vr[5] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
        let v = exec_no_mem(
            &PpuInstruction::Vxor {
                vt: 5,
                va: 5,
                vb: 5,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        assert_eq!(s.vr[5], 0);
    }

    #[test]
    fn vsldoi_shifts_by_shb_bytes() {
        let mut s = PpuState::new();
        s.vr[1] = u128::from_be_bytes([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0xFF,
        ]);
        s.vr[2] = u128::from_be_bytes([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ]);
        exec_no_mem(
            &PpuInstruction::Vsldoi {
                vt: 3,
                va: 1,
                vb: 2,
                shb: 4,
            },
            &mut s,
        );
        // Shift left by 4 bytes: result[0..12] = va[4..16], result[12..16] = vb[0..4].
        let expected = u128::from_be_bytes([
            0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x01, 0x02,
            0x03, 0x04,
        ]);
        assert_eq!(s.vr[3], expected);
    }

    fn pack_u32x4(lanes: [u32; 4]) -> u128 {
        let mut r = [0u8; 16];
        for (i, v) in lanes.iter().enumerate() {
            r[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        u128::from_be_bytes(r)
    }

    fn unpack_f32x4(v: u128) -> [f32; 4] {
        let b = v.to_be_bytes();
        let mut r = [0.0f32; 4];
        for i in 0..4 {
            r[i] = f32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
        r
    }

    #[test]
    fn vcfsx_converts_signed_ints_with_scale() {
        let v = pack_u32x4([1i32 as u32, (-1i32) as u32, 1024u32, (-1024i32) as u32]);
        let lanes = unpack_f32x4(vcfsx(v, 0));
        assert_eq!(lanes, [1.0, -1.0, 1024.0, -1024.0]);

        let lanes2 = unpack_f32x4(vcfsx(v, 10));
        assert_eq!(lanes2, [1.0 / 1024.0, -1.0 / 1024.0, 1.0, -1.0]);
    }

    #[test]
    fn vcfux_converts_unsigned_ints_with_scale() {
        let v = pack_u32x4([0, 1, 0xFFFF_FFFF, 0x8000_0000]);
        let lanes = unpack_f32x4(vcfux(v, 0));
        assert_eq!(lanes[0], 0.0);
        assert_eq!(lanes[1], 1.0);
        // 0xFFFF_FFFF as u32 as f32 rounds to 2^32.
        assert!((lanes[2] - 4294967296.0).abs() < 1.0);
        assert!((lanes[3] - 2147483648.0).abs() < 1.0);
    }

    #[test]
    fn vspltb_replicates_byte_index() {
        let src = u128::from_be_bytes([
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x10,
        ]);
        let result = vspltb(src, 4);
        assert_eq!(result, u128::from_be_bytes([0x55; 16]));
    }

    #[test]
    fn vsplth_replicates_halfword_index() {
        let src = u128::from_be_bytes([
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x10,
        ]);
        let result = vsplth(src, 2);
        let mut expected = [0u8; 16];
        for i in (0..16).step_by(2) {
            expected[i] = 0x55;
            expected[i + 1] = 0x66;
        }
        assert_eq!(result, u128::from_be_bytes(expected));
    }

    #[test]
    fn vspltisb_sign_extends_5_bit_immediate() {
        assert_eq!(vspltisb(7), u128::from_be_bytes([7; 16]));
        // 0x1F = -1 as 5-bit signed.
        assert_eq!(vspltisb(0x1F), u128::from_be_bytes([0xFF; 16]));
    }

    #[test]
    fn vspltish_sign_extends_to_halfword() {
        let mut expected = [0u8; 16];
        for i in (0..16).step_by(2) {
            expected[i + 1] = 3;
        }
        assert_eq!(vspltish(3), u128::from_be_bytes(expected));
        // 0x1F = -1 as 5-bit signed.
        assert_eq!(vspltish(0x1F), u128::from_be_bytes([0xFF; 16]));
    }
}
