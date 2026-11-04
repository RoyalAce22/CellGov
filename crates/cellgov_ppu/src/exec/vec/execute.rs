//! The VX-form and VA-form dispatchers and the `vsldoi` entry.

use super::convert::{vcfsx, vcfux};
use super::integer::{vadd_bytes, vadd_halfs, vadd_words, vcmpequw, vmulouh, vsub_ubytes_sat};
use super::merge::{vmrghb, vmrghh, vmrghw, vmrglb, vmrglh, vmrglw};
use super::permute::{vperm, vsel, vsldoi};
use super::shift::{vslw, vsrab, vsrah, vsraw, vsrw};
use super::splat::{vspltb, vsplth, vspltisb, vspltish, vspltisw, vspltw};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::PpuInstructionKind;
use crate::state::PpuState;

/// Execute a VX-form VMX instruction. `rc` is the VXR record bit; a
/// recording compare also sets CR6 from its result. A saturating op
/// that saturates sets `VSCR[SAT]`.
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
    let unimplemented = PpuFault::UnimplementedInstruction {
        kind: PpuInstructionKind::Vx,
        mnemonic: if rc { record_mnemonic(op) } else { op.into() },
    };

    // Only a VXR compare has a record form.
    if rc && !op.is_vxr_compare() {
        return ExecuteVerdict::Fault(unimplemented);
    }

    let (result, saturated) = match op {
        // -- Integer add/sub --
        // [AltiVec-PEM p:6-35 s:6.2] vaddubm: Vector Add Unsigned Byte Modulo
        VxOp::Vaddubm => (vadd_bytes(a, b), false),
        // [AltiVec-PEM p:6-37 s:6.2] vadduhm: Vector Add Unsigned Halfword Modulo
        VxOp::Vadduhm => (vadd_halfs(a, b), false),
        // [AltiVec-PEM p:6-39 s:6.2] vadduwm: Vector Add Unsigned Word Modulo
        VxOp::Vadduwm => (vadd_words(a, b), false),

        // -- Integer compare --
        // [AltiVec-PEM p:6-56 s:6.2] vcmpequw: Vector Compare Equal-to Unsigned Word
        VxOp::Vcmpequw => (vcmpequw(a, b), false),

        // -- Logical --
        // [AltiVec-PEM p:6-41 s:6.2] vand
        VxOp::Vand => (a & b, false),
        // [AltiVec-PEM p:6-111 s:6.2] vor
        VxOp::Vor => (a | b, false),
        // [AltiVec-PEM p:6-177 s:6.2] vxor
        VxOp::Vxor => (a ^ b, false),
        // [AltiVec-PEM p:6-42 s:6.2] vandc
        VxOp::Vandc => (a & !b, false),
        // [AltiVec-PEM p:6-110 s:6.2] vnor
        VxOp::Vnor => (!(a | b), false),

        // -- Shift --
        // [AltiVec-PEM p:6-139 s:6.2] vslw
        VxOp::Vslw => (vslw(a, b), false),
        // [AltiVec-PEM p:6-154 s:6.2] vsrw
        VxOp::Vsrw => (vsrw(a, b), false),
        // [AltiVec-PEM p:6-150 s:6.2] vsraw
        VxOp::Vsraw => (vsraw(a, b), false),
        // [AltiVec-PEM p:6-149 s:6.2] vsrah
        VxOp::Vsrah => (vsrah(a, b), false),
        // [AltiVec-PEM p:6-148 s:6.2] vsrab
        VxOp::Vsrab => (vsrab(a, b), false),

        // -- Splat --
        // [AltiVec-PEM p:6-140 s:6.2] vspltb (va is byte index)
        VxOp::Vspltb => (vspltb(b, va), false),
        // [AltiVec-PEM p:6-141 s:6.2] vsplth (va is halfword index)
        VxOp::Vsplth => (vsplth(b, va), false),
        // [AltiVec-PEM p:6-145 s:6.2] vspltw (va is word index)
        VxOp::Vspltw => (vspltw(b, va), false),
        // [AltiVec-PEM p:6-142 s:6.2] vspltisb (sign-extended 5-bit imm)
        VxOp::Vspltisb => (vspltisb(va), false),
        // [AltiVec-PEM p:6-143 s:6.2] vspltish
        VxOp::Vspltish => (vspltish(va), false),
        // [AltiVec-PEM p:6-144 s:6.2] vspltisw
        VxOp::Vspltisw => (vspltisw(va), false),

        // -- Merge --
        // [AltiVec-PEM p:6-89 s:6.2] vmrghb
        VxOp::Vmrghb => (vmrghb(a, b), false),
        // [AltiVec-PEM p:6-90 s:6.2] vmrghh
        VxOp::Vmrghh => (vmrghh(a, b), false),
        // [AltiVec-PEM p:6-91 s:6.2] vmrghw
        VxOp::Vmrghw => (vmrghw(a, b), false),
        // [AltiVec-PEM p:6-92 s:6.2] vmrglb
        VxOp::Vmrglb => (vmrglb(a, b), false),
        // [AltiVec-PEM p:6-93 s:6.2] vmrglh
        VxOp::Vmrglh => (vmrglh(a, b), false),
        // [AltiVec-PEM p:6-94 s:6.2] vmrglw
        VxOp::Vmrglw => (vmrglw(a, b), false),

        // -- Multiply --
        // [AltiVec-PEM p:6-108 s:6.2] vmulouh
        VxOp::Vmulouh => (vmulouh(a, b), false),

        // -- Subtract --
        // [AltiVec-PEM p:6-161 s:6.2] vsububs
        VxOp::Vsububs => vsub_ubytes_sat(a, b),

        // -- Int <-> Float conversions (va field is uimm scale) --
        // [AltiVec-PEM p:6-49 s:6.2] vcfsx
        VxOp::Vcfsx => (vcfsx(b, va), false),
        // [AltiVec-PEM p:6-50 s:6.2] vcfux
        VxOp::Vcfux => (vcfux(b, va), false),

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
            return ExecuteVerdict::Fault(unimplemented);
        }
    };

    state.set_vr(vt as usize, result);
    if saturated {
        state.set_sat();
    }
    if rc {
        state.set_cr_field(6, record_cr6(op, result));
    }
    ExecuteVerdict::Continue
}

/// The mnemonic of the record form of `op`, or of `op` itself when it has
/// no record form.
fn record_mnemonic(op: VxOp) -> &'static str {
    match op {
        VxOp::Vcmpequb => "vcmpequb.",
        VxOp::Vcmpequh => "vcmpequh.",
        VxOp::Vcmpequw => "vcmpequw.",
        VxOp::Vcmpeqfp => "vcmpeqfp.",
        VxOp::Vcmpgefp => "vcmpgefp.",
        VxOp::Vcmpgtub => "vcmpgtub.",
        VxOp::Vcmpgtuh => "vcmpgtuh.",
        VxOp::Vcmpgtuw => "vcmpgtuw.",
        VxOp::Vcmpgtfp => "vcmpgtfp.",
        VxOp::Vcmpgtsb => "vcmpgtsb.",
        VxOp::Vcmpgtsh => "vcmpgtsh.",
        VxOp::Vcmpgtsw => "vcmpgtsw.",
        VxOp::Vcmpbfp => "vcmpbfp.",
        _ => op.into(),
    }
}

/// CR6 of a recording vector compare, from its result vector.
pub(super) fn record_cr6(op: VxOp, result: u128) -> u8 {
    if op == VxOp::Vcmpbfp {
        // [AltiVec-PEM p:6-51 s:6.2] vcmpbfp.: CR6 = 0b00 || all_within_bounds || 0; an element is within bounds when its result is 0.
        return if result == 0 { 0b0010 } else { 0 };
    }
    // [AltiVec-PEM p:6-56 s:6.2] vcmpequw.: CR6 = all_true || 0b0 || all_false || 0b0 over the result elements.
    let all_true = u8::from(result == u128::MAX) << 3;
    let all_false = u8::from(result == 0) << 1;
    all_true | all_false
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
            return ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction {
                kind: PpuInstructionKind::Va,
                mnemonic: op.into(),
            });
        }
    };

    state.set_vr(vt as usize, result);
    ExecuteVerdict::Continue
}

/// Execute `vsldoi`. `shb` is the 4-bit byte-shift immediate carved out
/// of the VA-form vc slot by the decoder.
///
/// [AltiVec-PEM p:6-136 s:6.2] Vector Shift Left Double by Octet Immediate
pub(crate) fn execute_vsldoi(
    state: &mut PpuState,
    vt: u8,
    va: u8,
    vb: u8,
    shb: u8,
) -> ExecuteVerdict {
    let a = state.vr[va as usize];
    let b = state.vr[vb as usize];
    state.set_vr(vt as usize, vsldoi(a, b, shb));
    ExecuteVerdict::Continue
}
