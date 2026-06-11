//! VMX / AltiVec execution helpers. Vector registers are 128-bit
//! big-endian (byte 0 = MSB); per-op helpers take/return `u128` in
//! that order. Memory-touching vector ops live in [`mem`](super::mem).

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::state::PpuState;

/// Execute a VX-form VMX instruction (primary=4, 11-bit sub-opcode, 3 registers).
pub(crate) fn execute_vx(state: &mut PpuState, xo: u16, vt: u8, va: u8, vb: u8) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];

    let result = match xo {
        // -- Integer add/sub --
        // [AltiVec-PEM p:6-35 s:6.2] vaddubm: Vector Add Unsigned Byte Modulo
        0x000 => vadd_bytes(a, b),
        // [AltiVec-PEM p:6-37 s:6.2] vadduhm: Vector Add Unsigned Halfword Modulo
        0x040 => vadd_halfs(a, b),
        // [AltiVec-PEM p:6-39 s:6.2] vadduwm: Vector Add Unsigned Word Modulo
        0x080 => vadd_words(a, b),

        // -- Integer compare --
        // [AltiVec-PEM p:6-56 s:6.2] vcmpequw: Vector Compare Equal-to Unsigned Word
        0x086 => vcmpequw(a, b),

        // -- Logical -- (XOs per AltiVec-PEM Appendix A.5 Table A-6)
        // [AltiVec-PEM p:6-41 s:6.2] vand: VX-form XO=1028=0x404
        0x404 => a & b,
        // [AltiVec-PEM p:6-111 s:6.2] vor: VX-form XO=1284=0x504
        0x504 => a | b,
        // [AltiVec-PEM p:6-177 s:6.2] vxor: VX-form XO=1220=0x4C4
        0x4c4 => a ^ b,
        // [AltiVec-PEM p:6-42 s:6.2] vandc: VX-form XO=1092=0x444
        0x444 => a & !b,
        // [AltiVec-PEM p:6-110 s:6.2] vnor: VX-form XO=1156=0x484
        0x484 => !(a | b),

        // -- Shift -- (XOs per AltiVec-PEM Appendix A.5 Table A-6)
        // [AltiVec-PEM p:6-139 s:6.2] vslw: VX-form XO=388=0x184
        0x184 => vslw(a, b),
        // [AltiVec-PEM p:6-154 s:6.2] vsrw: VX-form XO=644=0x284
        0x284 => vsrw(a, b),
        // [AltiVec-PEM p:6-150 s:6.2] vsraw: VX-form XO=900=0x384
        0x384 => vsraw(a, b),
        // [AltiVec-PEM p:6-149 s:6.2] vsrah: VX-form XO=836=0x344
        0x344 => vsrah(a, b),
        // [AltiVec-PEM p:6-148 s:6.2] vsrab: VX-form XO=772=0x304
        0x304 => vsrab(a, b),

        // -- Splat (PPC AltiVec ISA XO values) --
        // [AltiVec-PEM p:6-140 s:6.2] vspltb: Vector Splat Byte (va is byte index)
        0x20c => vspltb(b, va),
        // [AltiVec-PEM p:6-141 s:6.2] vsplth: Vector Splat Half Word (va is halfword index)
        0x24c => vsplth(b, va),
        // [AltiVec-PEM p:6-145 s:6.2] vspltw: Vector Splat Word (va is word index)
        0x28c => vspltw(b, va),
        // [AltiVec-PEM p:6-142 s:6.2] vspltisb: Vector Splat Immediate Signed Byte (sign-extended 5-bit imm)
        0x30c => vspltisb(va),
        // [AltiVec-PEM p:6-143 s:6.2] vspltish: Vector Splat Immediate Signed Half Word
        0x34c => vspltish(va),
        // [AltiVec-PEM p:6-144 s:6.2] vspltisw: Vector Splat Immediate Signed Word
        0x38c => vspltisw(va),

        // -- Merge --
        // [AltiVec-PEM p:6-89 s:6.2] vmrghb: Vector Merge High Byte
        0x00c => vmrghb(a, b),
        // [AltiVec-PEM p:6-90 s:6.2] vmrghh: Vector Merge High Half Word
        0x04c => vmrghh(a, b),
        // [AltiVec-PEM p:6-91 s:6.2] vmrghw: Vector Merge High Word
        0x08c => vmrghw(a, b),
        // [AltiVec-PEM p:6-92 s:6.2] vmrglb: VX-form XO=268=0x10C
        0x10c => vmrglb(a, b),
        // [AltiVec-PEM p:6-93 s:6.2] vmrglh: VX-form XO=332=0x14C
        0x14c => vmrglh(a, b),
        // [AltiVec-PEM p:6-94 s:6.2] vmrglw: VX-form XO=396=0x18C
        0x18c => vmrglw(a, b),

        // -- Multiply --
        // [AltiVec-PEM p:6-108 s:6.2] vmulouh: Vector Multiply Odd Unsigned Half Word
        0x048 => vmulouh(a, b),

        // -- Subtract --
        // [AltiVec-PEM p:6-161 s:6.2] vsububs: Vector Subtract Unsigned Byte Saturate
        0x600 => vsub_ubytes_sat(a, b),

        // -- Int <-> Float conversions (VX-form, va field is uimm scale) --
        // [AltiVec-PEM p:6-49 s:6.2] vcfsx: Vector Convert from Signed Fixed-Point Word
        0x34a => vcfsx(b, va),
        // [AltiVec-PEM p:6-50 s:6.2] vcfux: Vector Convert from Unsigned Fixed-Point Word
        0x38a => vcfux(b, va),

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
        // [AltiVec-PEM p:6-133 s:6.2] vsel: Vector Select
        0x2a => vsel(a, b, c),
        // [AltiVec-PEM p:6-112 s:6.2] vperm: Vector Permute
        0x2b => vperm(a, b, c),
        _ => {
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(xo as u64));
        }
    };

    state.vr[vt as usize] = result;
    ExecuteVerdict::Continue
}

/// Execute `vsldoi`. `shb` is the 4-bit byte-shift immediate carved out
/// of the VA-form vc slot by the decoder.
// [AltiVec-PEM p:6-136 s:6.2] Vector Shift Left Double by Octet Immediate
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
    // [AltiVec-PEM p:6-161 s:6.2] vsububs: clamp vA[i]-vB[i] at 0.
    // [AltiVec-PEM p:4-4 s:4.2] VSCR[SAT] not modelled here.
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut r = [0u8; 16];
    for i in 0..16 {
        r[i] = ab[i].saturating_sub(bb[i]);
    }
    u128::from_be_bytes(r)
}

fn vsel(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-133 s:6.2] per-bit mux: c_bit ? b_bit : a_bit.
    (a & !c) | (b & c)
}

fn vperm(a: u128, b: u128, c: u128) -> u128 {
    // [AltiVec-PEM p:6-112 s:6.2] low 5 bits of each c byte index the
    // a:b concatenation (32 bytes).
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
    // [AltiVec-PEM p:6-136 s:6.2] SH is 4-bit; left-shift the a:b
    // concatenation by `sh` bytes and return the high 16.
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
    fn vxor_typed_and_vx_stub_paths_produce_identical_state() {
        let seed_va = 0x1111_2222_3333_4444_5555_6666_7777_8888u128;
        let seed_vb = 0xAAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000_1111u128;

        let mut s_typed = PpuState::new();
        s_typed.vr[1] = seed_va;
        s_typed.vr[2] = seed_vb;
        s_typed.vr[3] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
        exec_no_mem(
            &PpuInstruction::Vxor {
                vt: 3,
                va: 1,
                vb: 2,
            },
            &mut s_typed,
        );

        let mut s_stub = PpuState::new();
        s_stub.vr[1] = seed_va;
        s_stub.vr[2] = seed_vb;
        s_stub.vr[3] = 0xDEAD_BEEF_DEAD_BEEF_DEAD_BEEF_DEAD_BEEFu128;
        exec_no_mem(
            &PpuInstruction::Vx {
                xo: 0x4c4,
                vt: 3,
                va: 1,
                vb: 2,
            },
            &mut s_stub,
        );

        assert_eq!(s_typed.vr[3], s_stub.vr[3]);
        assert_eq!(s_typed.vr[3], seed_va ^ seed_vb);
    }

    /// Per AltiVec-PEM Appendix A.5 Table A-6: the VX-form logical and
    /// shift instructions have specific 11-bit XO values. An earlier
    /// implementation used wrong XOs that either fell out of the 11-bit
    /// range (silently dead arms) or aliased onto OTHER instructions'
    /// canonical XOs, so guest code using e.g. `vmuleub` was silently
    /// getting `vmrglb` results. This test pins every corrected XO.
    #[test]
    fn vx_xo_table_matches_altivec_pem_canonical() {
        let pairs: &[(u16, u128, u128, u128, &str)] = &[
            // (xo, va, vb, expected, name)
            (
                0x404,
                0xFF00_FF00_FF00_FF00_AAAA_BBBB_CCCC_DDDD,
                0x0FF0_0FF0_0FF0_0FF0_FFFF_0000_FFFF_0000,
                0x0F00_0F00_0F00_0F00_AAAA_0000_CCCC_0000,
                "vand",
            ),
            (
                0x504,
                0x1234_0000_0000_0000_0000_0000_0000_0000,
                0x0000_5678_0000_0000_0000_0000_0000_0000,
                0x1234_5678_0000_0000_0000_0000_0000_0000,
                "vor",
            ),
            (
                0x4c4,
                0xFFFF_0000_AAAA_5555_1234_5678_9ABC_DEF0,
                0xF0F0_F0F0_F0F0_F0F0_0F0F_0F0F_0F0F_0F0F,
                0x0F0F_F0F0_5A5A_A5A5_1D3B_5977_95B3_D1FF,
                "vxor",
            ),
            (
                0x444,
                0xFF00_FF00_0000_0000_0000_0000_0000_0000,
                0x0F00_0F00_0000_0000_0000_0000_0000_0000,
                0xF000_F000_0000_0000_0000_0000_0000_0000,
                "vandc",
            ),
            (
                0x484,
                0x0F00_0000_0000_0000_0000_0000_0000_0000,
                0x00F0_0000_0000_0000_0000_0000_0000_0000,
                0xF00F_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF,
                "vnor",
            ),
        ];
        for &(xo, va, vb, expected, name) in pairs {
            let mut s = PpuState::new();
            s.vr[1] = va;
            s.vr[2] = vb;
            exec_no_mem(
                &PpuInstruction::Vx {
                    xo,
                    vt: 3,
                    va: 1,
                    vb: 2,
                },
                &mut s,
            );
            assert_eq!(
                s.vr[3], expected,
                "{name} (xo={xo:#x}) produced wrong result"
            );
        }
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

    fn run_vx(xo: u16, va: u128, vb: u128) -> u128 {
        let mut s = PpuState::new();
        s.vr[1] = va;
        s.vr[2] = vb;
        let v = exec_no_mem(
            &PpuInstruction::Vx {
                xo,
                vt: 3,
                va: 1,
                vb: 2,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        s.vr[3]
    }

    fn run_vx_imm(xo: u16, va_field: u8, vb: u128) -> u128 {
        // For splat-immediate and vspltX / vcfX ops the va register field is
        // a 5-bit immediate, not a register selector. Use vr[0] (zero) as
        // the unused va register.
        let mut s = PpuState::new();
        s.vr[2] = vb;
        let v = exec_no_mem(
            &PpuInstruction::Vx {
                xo,
                vt: 3,
                va: va_field,
                vb: 2,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        s.vr[3]
    }

    fn run_va(xo: u8, va: u128, vb: u128, vc: u128) -> u128 {
        let mut s = PpuState::new();
        s.vr[1] = va;
        s.vr[2] = vb;
        s.vr[3] = vc;
        let v = exec_no_mem(
            &PpuInstruction::Va {
                xo,
                vt: 4,
                va: 1,
                vb: 2,
                vc: 3,
            },
            &mut s,
        );
        assert_eq!(v, ExecuteVerdict::Continue);
        s.vr[4]
    }

    // -- Integer add/sub --

    #[test]
    fn vaddubm_adds_each_byte_modulo_256() {
        // 0xFF + 0x01 = 0x00 (wrap), 0x10 + 0x20 = 0x30, 0x7F + 0x7F = 0xFE.
        let va = u128::from_be_bytes([
            0xFF, 0x10, 0x7F, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
            0x0C, 0x0D,
        ]);
        let vb = u128::from_be_bytes([
            0x01, 0x20, 0x7F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0x00, 0x30, 0xFE, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
            0x0C, 0x0D,
        ]);
        assert_eq!(run_vx(0x000, va, vb), expected);
    }

    #[test]
    fn vadduhm_adds_each_halfword_modulo_65536() {
        // 0xFFFF + 0x0001 = 0x0000, 0x1234 + 0x0001 = 0x1235.
        let va = u128::from_be_bytes([
            0xFF, 0xFF, 0x12, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let vb = u128::from_be_bytes([
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0x00, 0x00, 0x12, 0x35, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        assert_eq!(run_vx(0x040, va, vb), expected);
    }

    #[test]
    fn vadduwm_adds_each_word_modulo_2_pow_32() {
        let va = pack_u32x4([0xFFFF_FFFF, 0x0000_0001, 0x1234_5678, 0x8000_0000]);
        let vb = pack_u32x4([0x0000_0001, 0x0000_0002, 0x0000_0001, 0x8000_0000]);
        let expected = pack_u32x4([0x0000_0000, 0x0000_0003, 0x1234_5679, 0x0000_0000]);
        assert_eq!(run_vx(0x080, va, vb), expected);
    }

    // -- Integer compare --

    #[test]
    fn vcmpequw_emits_all_ones_or_zero_per_word() {
        let va = pack_u32x4([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]);
        let vb = pack_u32x4([0x1111_1111, 0x0000_0000, 0x3333_3333, 0xFFFF_FFFF]);
        let expected = pack_u32x4([0xFFFF_FFFF, 0x0000_0000, 0xFFFF_FFFF, 0x0000_0000]);
        assert_eq!(run_vx(0x086, va, vb), expected);
    }

    // -- Shifts --

    #[test]
    fn vslw_shifts_low_5_bits_of_each_word() {
        // Shift counts are taken from low 5 bits of vb's matching word.
        // Word 0: 1 << 4 = 0x10. Word 1: 0x0001 << 31 = 0x8000_0000.
        // Word 2: 0x0F << 0 = 0x0F (sh=0x20 & 0x1F = 0). Word 3: 0xAAAA << 1 = 0x15554.
        let va = pack_u32x4([0x0000_0001, 0x0000_0001, 0x0000_000F, 0x0000_AAAA]);
        let vb = pack_u32x4([4, 31, 0x20, 1]);
        let expected = pack_u32x4([0x0000_0010, 0x8000_0000, 0x0000_000F, 0x0001_5554]);
        assert_eq!(run_vx(0x184, va, vb), expected);
    }

    #[test]
    fn vsrw_logical_shifts_low_5_bits_of_each_word() {
        // Word 0: 0x8000_0000 >> 4 = 0x0800_0000 (zero-fill).
        // Word 1: 0xFFFF_FFFF >> 31 = 1. Word 2: 0xF0 >> 0 = 0xF0 (sh=0x20 -> 0).
        // Word 3: 0xDEAD_BEEF >> 8 = 0x00DE_ADBE.
        let va = pack_u32x4([0x8000_0000, 0xFFFF_FFFF, 0x0000_00F0, 0xDEAD_BEEF]);
        let vb = pack_u32x4([4, 31, 0x20, 8]);
        let expected = pack_u32x4([0x0800_0000, 0x0000_0001, 0x0000_00F0, 0x00DE_ADBE]);
        assert_eq!(run_vx(0x284, va, vb), expected);
    }

    #[test]
    fn vsraw_arithmetic_shifts_low_5_bits_of_each_word() {
        // Word 0: 0x8000_0000 >>a 4 = 0xF800_0000 (sign-fill).
        // Word 1: 0x4000_0000 >>a 4 = 0x0400_0000. Word 2: (-1) >>a 5 = -1.
        // Word 3: -16 >>a 2 = -4 = 0xFFFF_FFFC.
        let va = pack_u32x4([0x8000_0000, 0x4000_0000, 0xFFFF_FFFFu32, (-16i32) as u32]);
        let vb = pack_u32x4([4, 4, 5, 2]);
        let expected = pack_u32x4([0xF800_0000, 0x0400_0000, 0xFFFF_FFFF, (-4i32) as u32]);
        assert_eq!(run_vx(0x384, va, vb), expected);
    }

    #[test]
    fn vsrah_arithmetic_shifts_low_4_bits_of_each_halfword() {
        // Halfword 0: 0x8000 >>a 4 = 0xF800. Halfword 1: 0x4000 >>a 2 = 0x1000.
        // Halfword 2: -1 >>a 15 = -1 (0xFFFF). Halfword 3: -8 >>a 1 = -4 = 0xFFFC.
        // Sh field is masked to low 4 bits.
        let va = u128::from_be_bytes([
            0x80, 0x00, 0x40, 0x00, 0xFF, 0xFF, 0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let vb = u128::from_be_bytes([
            0x00, 0x04, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0xF8, 0x00, 0x10, 0x00, 0xFF, 0xFF, 0xFF, 0xFC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        assert_eq!(run_vx(0x344, va, vb), expected);
    }

    #[test]
    fn vsrab_arithmetic_shifts_low_3_bits_of_each_byte() {
        // Byte 0: 0x80 >>a 1 = 0xC0. Byte 1: 0x40 >>a 1 = 0x20.
        // Byte 2: 0xFF >>a 7 = 0xFF. Byte 3: 0x08 >>a 2 = 0x02.
        // Sh field is masked to low 3 bits.
        let va = u128::from_be_bytes([
            0x80, 0x40, 0xFF, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let vb = u128::from_be_bytes([
            0x01, 0x01, 0x07, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0xC0, 0x20, 0xFF, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        assert_eq!(run_vx(0x304, va, vb), expected);
    }

    // -- Splats --

    #[test]
    fn vspltw_replicates_word_lane_to_all_lanes() {
        let src = pack_u32x4([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]);
        // va field = 2 -> replicate word 2 (0x3333_3333).
        let result = run_vx_imm(0x28c, 2, src);
        assert_eq!(result, pack_u32x4([0x3333_3333; 4]));
    }

    #[test]
    fn vspltisw_sign_extends_5_bit_immediate_to_word() {
        // imm = 7 -> +7 in each word.
        let result = run_vx_imm(0x38c, 7, 0);
        assert_eq!(result, pack_u32x4([7; 4]));
        // imm = 0x1F = -1 as 5-bit signed -> 0xFFFF_FFFF in each word.
        let result = run_vx_imm(0x38c, 0x1F, 0);
        assert_eq!(result, pack_u32x4([0xFFFF_FFFF; 4]));
    }

    // -- Merges --

    #[test]
    fn vmrghb_interleaves_high_8_bytes_of_a_and_b() {
        let va = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let vb = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        // Big-endian "high" = first 8 bytes. Interleave: a0,b0,a1,b1,...
        let expected = u128::from_be_bytes([
            0xA0, 0xB0, 0xA1, 0xB1, 0xA2, 0xB2, 0xA3, 0xB3, 0xA4, 0xB4, 0xA5, 0xB5, 0xA6, 0xB6,
            0xA7, 0xB7,
        ]);
        assert_eq!(run_vx(0x00c, va, vb), expected);
    }

    #[test]
    fn vmrghh_interleaves_high_4_halfwords_of_a_and_b() {
        let va = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let vb = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        // Interleave high 4 halfwords: a[0..2],b[0..2],a[2..4],b[2..4],...
        let expected = u128::from_be_bytes([
            0xA0, 0xA1, 0xB0, 0xB1, 0xA2, 0xA3, 0xB2, 0xB3, 0xA4, 0xA5, 0xB4, 0xB5, 0xA6, 0xA7,
            0xB6, 0xB7,
        ]);
        assert_eq!(run_vx(0x04c, va, vb), expected);
    }

    #[test]
    fn vmrghw_interleaves_high_2_words_of_a_and_b() {
        let va = pack_u32x4([0xA0A0_A0A0, 0xA1A1_A1A1, 0xA2A2_A2A2, 0xA3A3_A3A3]);
        let vb = pack_u32x4([0xB0B0_B0B0, 0xB1B1_B1B1, 0xB2B2_B2B2, 0xB3B3_B3B3]);
        // Interleave high 2 words: a0,b0,a1,b1.
        let expected = pack_u32x4([0xA0A0_A0A0, 0xB0B0_B0B0, 0xA1A1_A1A1, 0xB1B1_B1B1]);
        assert_eq!(run_vx(0x08c, va, vb), expected);
    }

    #[test]
    fn vmrglb_interleaves_low_8_bytes_of_a_and_b() {
        let va = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let vb = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        // Big-endian "low" = last 8 bytes. Interleave: a8,b8,a9,b9,...
        let expected = u128::from_be_bytes([
            0xA8, 0xB8, 0xA9, 0xB9, 0xAA, 0xBA, 0xAB, 0xBB, 0xAC, 0xBC, 0xAD, 0xBD, 0xAE, 0xBE,
            0xAF, 0xBF,
        ]);
        assert_eq!(run_vx(0x10c, va, vb), expected);
    }

    #[test]
    fn vmrglh_interleaves_low_4_halfwords_of_a_and_b() {
        let va = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let vb = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        let expected = u128::from_be_bytes([
            0xA8, 0xA9, 0xB8, 0xB9, 0xAA, 0xAB, 0xBA, 0xBB, 0xAC, 0xAD, 0xBC, 0xBD, 0xAE, 0xAF,
            0xBE, 0xBF,
        ]);
        assert_eq!(run_vx(0x14c, va, vb), expected);
    }

    #[test]
    fn vmrglw_interleaves_low_2_words_of_a_and_b() {
        let va = pack_u32x4([0xA0A0_A0A0, 0xA1A1_A1A1, 0xA2A2_A2A2, 0xA3A3_A3A3]);
        let vb = pack_u32x4([0xB0B0_B0B0, 0xB1B1_B1B1, 0xB2B2_B2B2, 0xB3B3_B3B3]);
        // Interleave low 2 words: a2,b2,a3,b3.
        let expected = pack_u32x4([0xA2A2_A2A2, 0xB2B2_B2B2, 0xA3A3_A3A3, 0xB3B3_B3B3]);
        assert_eq!(run_vx(0x18c, va, vb), expected);
    }

    // -- Multiply --

    #[test]
    fn vmulouh_multiplies_odd_halfwords_into_words() {
        // For each word i, multiply the ODD halfword (low halfword in BE
        // = bytes [i*4+2..i*4+4]) of va by that of vb. Even halfwords are
        // ignored.
        let va = u128::from_be_bytes([
            0xFF, 0xFF, 0x00, 0x10, // word 0: odd half = 0x0010 = 16
            0xFF, 0xFF, 0xFF, 0xFF, // word 1: odd half = 0xFFFF
            0x00, 0x00, 0x00, 0x02, // word 2: odd half = 2
            0x00, 0x00, 0x12, 0x34, // word 3: odd half = 0x1234
        ]);
        let vb = u128::from_be_bytes([
            0xAA, 0xAA, 0x00, 0x20, // word 0: odd half = 0x0020 = 32
            0xAA, 0xAA, 0xFF, 0xFF, // word 1: odd half = 0xFFFF
            0xAA, 0xAA, 0x00, 0x03, // word 2: odd half = 3
            0xAA, 0xAA, 0x00, 0x10, // word 3: odd half = 0x10
        ]);
        let expected = pack_u32x4([
            16 * 32,               // 0x200
            0xFFFFu32 * 0xFFFFu32, // 0xFFFE_0001
            2 * 3,                 // 6
            0x1234u32 * 0x10u32,   // 0x12340
        ]);
        assert_eq!(run_vx(0x048, va, vb), expected);
    }

    // -- Subtract saturating --

    #[test]
    fn vsububs_saturates_each_byte_subtract_to_zero() {
        // 0x10 - 0x05 = 0x0B. 0x05 - 0x10 saturates to 0x00. 0x00 - 0xFF -> 0.
        // 0xFF - 0x00 = 0xFF.
        let va = u128::from_be_bytes([
            0x10, 0x05, 0x00, 0xFF, 0x80, 0x7F, 0x40, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let vb = u128::from_be_bytes([
            0x05, 0x10, 0xFF, 0x00, 0x7F, 0x80, 0x40, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0x0B, 0x00, 0x00, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        assert_eq!(run_vx(0x600, va, vb), expected);
    }

    // -- VA-form: select / permute --

    #[test]
    fn vsel_picks_b_bits_where_c_is_one_else_a() {
        // c = 0xFF00_FF00_..._FF00. Even bytes from b, odd bytes from a.
        let va = u128::from_be_bytes([0xAA; 16]);
        let vb = u128::from_be_bytes([0xBB; 16]);
        let vc = u128::from_be_bytes([
            0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00,
            0xFF, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA,
            0xBB, 0xAA,
        ]);
        assert_eq!(run_va(0x2a, va, vb, vc), expected);
    }

    #[test]
    fn vperm_indexes_concat_of_a_and_b_by_low_5_bits_of_c() {
        let va = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        let vb = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        // Indices: 0 -> a0=0xA0; 16 -> b0=0xB0; 15 -> a15=0xAF; 31 -> b15=0xBF;
        // 0x20 (masked to 0) -> a0=0xA0. Fill rest with zeros (index 0).
        let vc = u128::from_be_bytes([
            0x00, 0x10, 0x0F, 0x1F, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]);
        let expected = u128::from_be_bytes([
            0xA0, 0xB0, 0xAF, 0xBF, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0, 0xA0,
            0xA0, 0xA0,
        ]);
        assert_eq!(run_va(0x2b, va, vb, vc), expected);
    }

    // -- Level-2: side-effect invariants --
    //
    // AltiVec compare instructions (e.g., vcmpequw) optionally set CR
    // field 6 when the Rc bit is 1, per [AltiVec-PEM p:6-56 s:6.2]:
    //   CR6.bit0 = 1 iff all elements compared true
    //   CR6.bit1 = 0
    //   CR6.bit2 = 1 iff no element compared true
    //   CR6.bit3 = 0
    // The `PpuInstruction::Vx { xo, vt, va, vb }` variant in
    // `instruction/insn.rs` carries no Rc field, so the executor has
    // no way to be ASKED for the Rc=1 behavior. The current
    // `vcmpequw` arm only writes the result vector; CR is never
    // touched. The tests below pin that absence and pin the same
    // "no CR/XER side effect" invariant for every other VX/VA op.

    /// Documents that the `Vx` variant cannot request the Rc=1 CR6
    /// update path. The variant has no Rc field, so the executor
    /// never sets CR6 even on `vcmpequw`. A future implementer
    /// adding Rc plumbing should replace this test with the four
    /// CR6 cases (all-equal, none-equal, partial, Rc=0 untouched)
    /// listed in [AltiVec-PEM p:6-56 s:6.2].
    #[test]
    fn vcmpequw_rc_one_is_currently_unmodeled() {
        let mut s = PpuState::new();
        s.cr = 0xABCD_EF01;
        s.vr[1] = pack_u32x4([1, 2, 3, 4]);
        s.vr[2] = pack_u32x4([1, 2, 3, 4]);
        exec_no_mem(
            &PpuInstruction::Vx {
                xo: 0x086,
                vt: 3,
                va: 1,
                vb: 2,
            },
            &mut s,
        );
        // Result vector is set per the compare semantics.
        assert_eq!(s.vr[3], pack_u32x4([0xFFFF_FFFF; 4]));
        // CR is unchanged because the variant carries no Rc bit.
        assert_eq!(s.cr, 0xABCD_EF01);
    }

    #[test]
    fn vx_ops_do_not_touch_cr_or_xer_when_rc_zero() {
        let mut s = PpuState::new();
        s.cr = 0xABCD_EF01;
        s.xer = 0x1234_5678;
        s.vr[1] = pack_u32x4([1, 2, 3, 4]);
        s.vr[2] = pack_u32x4([10, 20, 30, 40]);
        exec_no_mem(
            &PpuInstruction::Vx {
                xo: 0x080,
                vt: 3,
                va: 1,
                vb: 2,
            },
            &mut s,
        );
        assert_eq!(s.vr[3], pack_u32x4([11, 22, 33, 44]));
        assert_eq!(s.cr, 0xABCD_EF01);
        assert_eq!(s.xer, 0x1234_5678);
    }

    /// [AltiVec-PEM p:6-133 s:6.2] vsel is a VA-form op with no Rc bit.
    #[test]
    fn vsel_does_not_touch_cr() {
        let mut s = PpuState::new();
        s.cr = 0xABCD_EF01;
        s.xer = 0x1234_5678;
        s.vr[1] = u128::from_be_bytes([0xAA; 16]);
        s.vr[2] = u128::from_be_bytes([0xBB; 16]);
        s.vr[3] = u128::from_be_bytes([0xFF; 16]);
        exec_no_mem(
            &PpuInstruction::Va {
                xo: 0x2a,
                vt: 4,
                va: 1,
                vb: 2,
                vc: 3,
            },
            &mut s,
        );
        assert_eq!(s.vr[4], u128::from_be_bytes([0xBB; 16]));
        assert_eq!(s.cr, 0xABCD_EF01);
        assert_eq!(s.xer, 0x1234_5678);
    }

    /// [AltiVec-PEM p:6-112 s:6.2] vperm is a VA-form op with no Rc bit.
    #[test]
    fn vperm_does_not_touch_cr() {
        let mut s = PpuState::new();
        s.cr = 0xABCD_EF01;
        s.xer = 0x1234_5678;
        s.vr[1] = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ]);
        s.vr[2] = u128::from_be_bytes([
            0xB0, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD,
            0xBE, 0xBF,
        ]);
        s.vr[3] = 0;
        exec_no_mem(
            &PpuInstruction::Va {
                xo: 0x2b,
                vt: 4,
                va: 1,
                vb: 2,
                vc: 3,
            },
            &mut s,
        );
        assert_eq!(s.cr, 0xABCD_EF01);
        assert_eq!(s.xer, 0x1234_5678);
    }
}
