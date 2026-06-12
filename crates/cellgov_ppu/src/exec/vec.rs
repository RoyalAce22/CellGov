//! VMX / AltiVec execution helpers. Vector registers are 128-bit
//! big-endian (byte 0 = MSB); per-op helpers take/return `u128` in
//! that order. Memory-touching vector ops live in [`mem`](super::mem).

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::ops::{VaOp, VxOp};
use crate::state::PpuState;

/// Execute a VX-form VMX instruction. `rc` is the VXR record bit;
/// recording compare forms are not implemented yet and fault.
///
/// Exhaustive over [`VxOp`]: adding an op fails compilation here
/// until its execution (or explicit fault) is decided.
pub(crate) fn execute_vx(
    state: &mut PpuState,
    op: VxOp,
    rc: bool,
    vt: u8,
    va: u8,
    vb: u8,
) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];

    if rc {
        // Rc=1 compare forms also update CR6; not modelled yet.
        return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(op as u64 + 1024));
    }

    let result = match op {
        // -- Integer add/sub --
        // [AltiVec-PEM p:6-35 s:6.2] vaddubm: Vector Add Unsigned Byte Modulo
        VxOp::Vaddubm => vadd_bytes(a, b),
        // [AltiVec-PEM p:6-37 s:6.2] vadduhm: Vector Add Unsigned Halfword Modulo
        VxOp::Vadduhm => vadd_halfs(a, b),
        // [AltiVec-PEM p:6-39 s:6.2] vadduwm: Vector Add Unsigned Word Modulo
        VxOp::Vadduwm => vadd_words(a, b),

        // -- Integer compare --
        // [AltiVec-PEM p:6-56 s:6.2] vcmpequw: Vector Compare Equal-to Unsigned Word
        VxOp::Vcmpequw => vcmpequw(a, b),

        // -- Logical --
        // [AltiVec-PEM p:6-41 s:6.2] vand
        VxOp::Vand => a & b,
        // [AltiVec-PEM p:6-111 s:6.2] vor
        VxOp::Vor => a | b,
        // [AltiVec-PEM p:6-177 s:6.2] vxor
        VxOp::Vxor => a ^ b,
        // [AltiVec-PEM p:6-42 s:6.2] vandc
        VxOp::Vandc => a & !b,
        // [AltiVec-PEM p:6-110 s:6.2] vnor
        VxOp::Vnor => !(a | b),

        // -- Shift --
        // [AltiVec-PEM p:6-139 s:6.2] vslw
        VxOp::Vslw => vslw(a, b),
        // [AltiVec-PEM p:6-154 s:6.2] vsrw
        VxOp::Vsrw => vsrw(a, b),
        // [AltiVec-PEM p:6-150 s:6.2] vsraw
        VxOp::Vsraw => vsraw(a, b),
        // [AltiVec-PEM p:6-149 s:6.2] vsrah
        VxOp::Vsrah => vsrah(a, b),
        // [AltiVec-PEM p:6-148 s:6.2] vsrab
        VxOp::Vsrab => vsrab(a, b),

        // -- Splat --
        // [AltiVec-PEM p:6-140 s:6.2] vspltb (va is byte index)
        VxOp::Vspltb => vspltb(b, va),
        // [AltiVec-PEM p:6-141 s:6.2] vsplth (va is halfword index)
        VxOp::Vsplth => vsplth(b, va),
        // [AltiVec-PEM p:6-145 s:6.2] vspltw (va is word index)
        VxOp::Vspltw => vspltw(b, va),
        // [AltiVec-PEM p:6-142 s:6.2] vspltisb (sign-extended 5-bit imm)
        VxOp::Vspltisb => vspltisb(va),
        // [AltiVec-PEM p:6-143 s:6.2] vspltish
        VxOp::Vspltish => vspltish(va),
        // [AltiVec-PEM p:6-144 s:6.2] vspltisw
        VxOp::Vspltisw => vspltisw(va),

        // -- Merge --
        // [AltiVec-PEM p:6-89 s:6.2] vmrghb
        VxOp::Vmrghb => vmrghb(a, b),
        // [AltiVec-PEM p:6-90 s:6.2] vmrghh
        VxOp::Vmrghh => vmrghh(a, b),
        // [AltiVec-PEM p:6-91 s:6.2] vmrghw
        VxOp::Vmrghw => vmrghw(a, b),
        // [AltiVec-PEM p:6-92 s:6.2] vmrglb
        VxOp::Vmrglb => vmrglb(a, b),
        // [AltiVec-PEM p:6-93 s:6.2] vmrglh
        VxOp::Vmrglh => vmrglh(a, b),
        // [AltiVec-PEM p:6-94 s:6.2] vmrglw
        VxOp::Vmrglw => vmrglw(a, b),

        // -- Multiply --
        // [AltiVec-PEM p:6-108 s:6.2] vmulouh
        VxOp::Vmulouh => vmulouh(a, b),

        // -- Subtract --
        // [AltiVec-PEM p:6-161 s:6.2] vsububs
        VxOp::Vsububs => vsub_ubytes_sat(a, b),

        // -- Int <-> Float conversions (va field is uimm scale) --
        // [AltiVec-PEM p:6-49 s:6.2] vcfsx
        VxOp::Vcfsx => vcfsx(b, va),
        // [AltiVec-PEM p:6-50 s:6.2] vcfux
        VxOp::Vcfux => vcfux(b, va),

        VxOp::Vmaxub
        | VxOp::Vrlb
        | VxOp::Vcmpequb
        | VxOp::Vmuloub
        | VxOp::Vaddfp
        | VxOp::Vpkuhum
        | VxOp::Vmaxuh
        | VxOp::Vrlh
        | VxOp::Vcmpequh
        | VxOp::Vsubfp
        | VxOp::Vpkuwum
        | VxOp::Vmaxuw
        | VxOp::Vrlw
        | VxOp::Vpkuhus
        | VxOp::Vcmpeqfp
        | VxOp::Vpkuwus
        | VxOp::Vmaxsb
        | VxOp::Vslb
        | VxOp::Vmulosb
        | VxOp::Vrefp
        | VxOp::Vpkshus
        | VxOp::Vmaxsh
        | VxOp::Vslh
        | VxOp::Vmulosh
        | VxOp::Vrsqrtefp
        | VxOp::Vpkswus
        | VxOp::Vaddcuw
        | VxOp::Vmaxsw
        | VxOp::Vexptefp
        | VxOp::Vpkshss
        | VxOp::Vsl
        | VxOp::Vcmpgefp
        | VxOp::Vlogefp
        | VxOp::Vpkswss
        | VxOp::Vaddubs
        | VxOp::Vminub
        | VxOp::Vsrb
        | VxOp::Vcmpgtub
        | VxOp::Vmuleub
        | VxOp::Vrfin
        | VxOp::Vupkhsb
        | VxOp::Vadduhs
        | VxOp::Vminuh
        | VxOp::Vsrh
        | VxOp::Vcmpgtuh
        | VxOp::Vmuleuh
        | VxOp::Vrfiz
        | VxOp::Vupkhsh
        | VxOp::Vadduws
        | VxOp::Vminuw
        | VxOp::Vcmpgtuw
        | VxOp::Vrfip
        | VxOp::Vupklsb
        | VxOp::Vsr
        | VxOp::Vcmpgtfp
        | VxOp::Vrfim
        | VxOp::Vupklsh
        | VxOp::Vaddsbs
        | VxOp::Vminsb
        | VxOp::Vcmpgtsb
        | VxOp::Vmulesb
        | VxOp::Vpkpx
        | VxOp::Vaddshs
        | VxOp::Vminsh
        | VxOp::Vcmpgtsh
        | VxOp::Vmulesh
        | VxOp::Vupkhpx
        | VxOp::Vaddsws
        | VxOp::Vminsw
        | VxOp::Vcmpgtsw
        | VxOp::Vctuxs
        | VxOp::Vcmpbfp
        | VxOp::Vctsxs
        | VxOp::Vupklpx
        | VxOp::Vsububm
        | VxOp::Vavgub
        | VxOp::Vmaxfp
        | VxOp::Vslo
        | VxOp::Vsubuhm
        | VxOp::Vavguh
        | VxOp::Vminfp
        | VxOp::Vsro
        | VxOp::Vsubuwm
        | VxOp::Vavguw
        | VxOp::Vavgsb
        | VxOp::Vavgsh
        | VxOp::Vsubcuw
        | VxOp::Vavgsw
        | VxOp::Vsum4ubs
        | VxOp::Vsubuhs
        | VxOp::Vsum4shs
        | VxOp::Vsubuws
        | VxOp::Vsum2sws
        | VxOp::Vsubsbs
        | VxOp::Vsum4sbs
        | VxOp::Vsubshs
        | VxOp::Vsubsws
        | VxOp::Vsumsws => {
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(op as u64));
        }
    };

    state.vr[vt as usize] = result;
    ExecuteVerdict::Continue
}

/// Execute a VA-form VMX instruction (primary=4, 6-bit sub-opcode, 4 registers).
pub(crate) fn execute_va(
    state: &mut PpuState,
    op: VaOp,
    vt: u8,
    va: u8,
    vb: u8,
    vc: u8,
) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];
    let c = state.vr[vc as usize];

    let result = match op {
        // [AltiVec-PEM p:6-133 s:6.2] vsel: Vector Select
        VaOp::Vsel => vsel(a, b, c),
        // [AltiVec-PEM p:6-112 s:6.2] vperm: Vector Permute
        VaOp::Vperm => vperm(a, b, c),
        // Decode routes vsldoi to the typed variant; this arm keeps
        // the match total for hand-built instructions.
        VaOp::Vsldoi => vsldoi(a, b, vc & 0xF),
        VaOp::Vmhaddshs
        | VaOp::Vmhraddshs
        | VaOp::Vmladduhm
        | VaOp::Vmsumubm
        | VaOp::Vmsummbm
        | VaOp::Vmsumuhm
        | VaOp::Vmsumuhs
        | VaOp::Vmsumshm
        | VaOp::Vmsumshs
        | VaOp::Vmaddfp
        | VaOp::Vnmsubfp => {
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(op as u64));
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
#[path = "tests/vec_tests.rs"]
mod tests;
