//! The VX-form and VA-form dispatchers and the `vsldoi` entry.

use super::convert::{vcfsx, vcfux, vctsxs, vctuxs};
use super::float_arith::{vaddfp, vmaddfp, vnmsubfp, vrfi, vsubfp};
use super::float_compare::{vcmpbfp, vcmpeqfp, vcmpgefp, vcmpgtfp};
use super::float_minmax::{vmaxfp, vminfp};
use super::integer::{
    vadd_bytes, vadd_halfs, vadd_sbytes_sat, vadd_shalfs_sat, vadd_swords_sat, vadd_ubytes_sat,
    vadd_uhalfs_sat, vadd_uwords_sat, vadd_words, vaddcuw, vcmpequb, vcmpequh, vcmpequw, vcmpgtsb,
    vcmpgtsh, vcmpgtsw, vcmpgtub, vcmpgtuh, vcmpgtuw, vmulesb, vmulesh, vmuleub, vmuleuh, vmulosb,
    vmulosh, vmuloub, vmulouh, vsub_bytes, vsub_halfs, vsub_sbytes_sat, vsub_shalfs_sat,
    vsub_swords_sat, vsub_ubytes_sat, vsub_uhalfs_sat, vsub_uwords_sat, vsub_words, vsubcuw,
};
use super::merge::{vmrghb, vmrghh, vmrghw, vmrglb, vmrglh, vmrglw};
use super::minmax::{
    vavgsb, vavgsh, vavgsw, vavgub, vavguh, vavguw, vmaxsb, vmaxsh, vmaxsw, vmaxub, vmaxuh, vmaxuw,
    vminsb, vminsh, vminsw, vminub, vminuh, vminuw,
};
use super::multiply_add::{
    vmhaddshs, vmhraddshs, vmladduhm, vmsummbm, vmsumshm, vmsumshs, vmsumubm, vmsumuhm, vmsumuhs,
};
use super::pack::{vpkpx, vpkshss, vpkshus, vpkswss, vpkswus, vpkuhum, vpkuhus, vpkuwum, vpkuwus};
use super::permute::{vperm, vsel, vsldoi};
use super::shift::{
    vrlb, vrlh, vrlw, vsl, vslb, vslh, vslo, vslw, vsr, vsrab, vsrah, vsraw, vsrb, vsrh, vsro, vsrw,
};
use super::splat::{vspltb, vsplth, vspltisb, vspltish, vspltisw, vspltw};
use super::sum_across::{vsum2sws, vsum4sbs, vsum4shs, vsum4ubs, vsumsws};
use super::unpack::{vupkhpx, vupkhsb, vupkhsh, vupklpx, vupklsb, vupklsh};
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::ops::{VaOp, VxOp};
use crate::instruction::PpuInstructionKind;
use crate::state::PpuState;
use cellgov_float::{Rounding, VmxMode};
use cellgov_ps3_abi::hw::ppu::VSCR_NJ;

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
    let mode = VmxMode::from_nj(state.vscr() & VSCR_NJ != 0);
    let unimplemented = PpuFault::UnimplementedInstruction {
        kind: PpuInstructionKind::Vx,
        mnemonic: op.into(),
    };

    if !vx_executes(op, rc) {
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
        // [AltiVec-PEM p:6-30 s:6.2] vaddcuw: each vD word is the carry out of bit 0 of the unsigned word sum, zero-extended.
        VxOp::Vaddcuw => (vaddcuw(a, b), false),
        // [AltiVec-PEM p:6-160 s:6.2] vsububm: Vector Subtract Byte Modulo
        VxOp::Vsububm => (vsub_bytes(a, b), false),
        // [AltiVec-PEM p:6-162 s:6.2] vsubuhm: Vector Subtract Half Word Modulo
        VxOp::Vsubuhm => (vsub_halfs(a, b), false),
        // [AltiVec-PEM p:6-164 s:6.2] vsubuwm: Vector Subtract Word Modulo
        VxOp::Vsubuwm => (vsub_words(a, b), false),
        // [AltiVec-PEM p:6-155 s:6.2] vsubcuw: each vD word is the complement of the borrow out of bit 0 of the unsigned word difference, zero-extended.
        VxOp::Vsubcuw => (vsubcuw(a, b), false),

        // -- Integer compare --
        // [AltiVec-PEM p:6-56 s:6.2] vcmpequw: Vector Compare Equal-to Unsigned Word
        VxOp::Vcmpequw => (vcmpequw(a, b), false),
        // [AltiVec-PEM p:6-54 s:6.2] vcmpequb: Vector Compare Equal-to Unsigned Byte
        VxOp::Vcmpequb => (vcmpequb(a, b), false),
        // [AltiVec-PEM p:6-55 s:6.2] vcmpequh: Vector Compare Equal-to Unsigned Half Word
        VxOp::Vcmpequh => (vcmpequh(a, b), false),
        // [AltiVec-PEM p:6-62 s:6.2] vcmpgtub: Vector Compare Greater-Than Unsigned Byte
        VxOp::Vcmpgtub => (vcmpgtub(a, b), false),
        // [AltiVec-PEM p:6-63 s:6.2] vcmpgtuh: Vector Compare Greater-Than Unsigned Half Word
        VxOp::Vcmpgtuh => (vcmpgtuh(a, b), false),
        // [AltiVec-PEM p:6-64 s:6.2] vcmpgtuw: Vector Compare Greater-Than Unsigned Word
        VxOp::Vcmpgtuw => (vcmpgtuw(a, b), false),
        // [AltiVec-PEM p:6-59 s:6.2] vcmpgtsb: Vector Compare Greater-Than Signed Byte
        VxOp::Vcmpgtsb => (vcmpgtsb(a, b), false),
        // [AltiVec-PEM p:6-60 s:6.2] vcmpgtsh: Vector Compare Greater-Than Signed Half Word
        VxOp::Vcmpgtsh => (vcmpgtsh(a, b), false),
        // [AltiVec-PEM p:6-61 s:6.2] vcmpgtsw: Vector Compare Greater-Than Signed Word
        VxOp::Vcmpgtsw => (vcmpgtsw(a, b), false),

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
        // [AltiVec-PEM p:6-148 s:6.2] vsrab: shift each signed byte right by the low 3 bits of its vB byte.
        // The page's pseudocode reads six count bits and its prose three; the ppu_vsplat_vsrab console capture settles it as three.
        VxOp::Vsrab => (vsrab(a, b), false),
        // [AltiVec-PEM p:6-135 s:6.2] vslb: shift each byte left by the low 3 bits of its vB byte, zero fill.
        VxOp::Vslb => (vslb(a, b), false),
        // [AltiVec-PEM p:6-137 s:6.2] vslh: shift each half word left by the low 4 bits of its vB half word, zero fill.
        VxOp::Vslh => (vslh(a, b), false),
        // [AltiVec-PEM p:6-151 s:6.2] vsrb: shift each byte right by the low 3 bits of its vB byte, zero fill.
        VxOp::Vsrb => (vsrb(a, b), false),
        // [AltiVec-PEM p:6-152 s:6.2] vsrh: shift each half word right by the low 4 bits of its vB half word, zero fill.
        VxOp::Vsrh => (vsrh(a, b), false),
        // [AltiVec-PEM p:6-129 s:6.2] vrlb: rotate each byte left by the low 3 bits of its vB byte.
        VxOp::Vrlb => (vrlb(a, b), false),
        // [AltiVec-PEM p:6-130 s:6.2] vrlh: rotate each half word left by the low 4 bits of its vB half word.
        VxOp::Vrlh => (vrlh(a, b), false),
        // [AltiVec-PEM p:6-131 s:6.2] vrlw: rotate each word left by the low 5 bits of its vB word.
        VxOp::Vrlw => (vrlw(a, b), false),

        // -- Splat --
        // The PEM leaves a UIMM past the last element open; the ppu_vsplat_vsrab console capture shows it taken modulo the element count.
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
        // [AltiVec-PEM p:6-104 s:6.2] vmuleuh: Vector Multiply Even Unsigned Half Word
        VxOp::Vmuleuh => (vmuleuh(a, b), false),
        // [AltiVec-PEM p:6-106 s:6.2] vmulosh: Vector Multiply Odd Signed Half Word
        VxOp::Vmulosh => (vmulosh(a, b), false),
        // [AltiVec-PEM p:6-102 s:6.2] vmulesh: Vector Multiply Even Signed Half Word
        VxOp::Vmulesh => (vmulesh(a, b), false),
        // [AltiVec-PEM p:6-107 s:6.2] vmuloub: Vector Multiply Odd Unsigned Byte
        VxOp::Vmuloub => (vmuloub(a, b), false),
        // [AltiVec-PEM p:6-103 s:6.2] vmuleub: Vector Multiply Even Unsigned Byte
        VxOp::Vmuleub => (vmuleub(a, b), false),
        // [AltiVec-PEM p:6-105 s:6.2] vmulosb: Vector Multiply Odd Signed Byte
        VxOp::Vmulosb => (vmulosb(a, b), false),
        // [AltiVec-PEM p:6-101 s:6.2] vmulesb: Vector Multiply Even Signed Byte
        VxOp::Vmulesb => (vmulesb(a, b), false),

        // -- Subtract --
        // [AltiVec-PEM p:6-161 s:6.2] vsububs
        VxOp::Vsububs => vsub_ubytes_sat(a, b),

        // -- Saturating add and subtract (each sets SAT when an element clamps) --
        // [AltiVec-PEM p:6-36 s:6.2] vaddubs: each byte sum clamps to [0, 2^8 - 1].
        VxOp::Vaddubs => vadd_ubytes_sat(a, b),
        // [AltiVec-PEM p:6-38 s:6.2] vadduhs: each half-word sum clamps to [0, 2^16 - 1].
        VxOp::Vadduhs => vadd_uhalfs_sat(a, b),
        // [AltiVec-PEM p:6-40 s:6.2] vadduws: each word sum clamps to [0, 2^32 - 1].
        VxOp::Vadduws => vadd_uwords_sat(a, b),
        // [AltiVec-PEM p:6-32 s:6.2] vaddsbs: each signed byte sum clamps to [-2^7, 2^7 - 1].
        VxOp::Vaddsbs => vadd_sbytes_sat(a, b),
        // [AltiVec-PEM p:6-33 s:6.2] vaddshs: each signed half-word sum clamps to [-2^15, 2^15 - 1].
        VxOp::Vaddshs => vadd_shalfs_sat(a, b),
        // [AltiVec-PEM p:6-34 s:6.2] vaddsws: each signed word sum clamps to [-2^31, 2^31 - 1].
        VxOp::Vaddsws => vadd_swords_sat(a, b),
        // [AltiVec-PEM p:6-163 s:6.2] vsubuhs: each half-word difference clamps at 0.
        VxOp::Vsubuhs => vsub_uhalfs_sat(a, b),
        // [AltiVec-PEM p:6-165 s:6.2] vsubuws: each word difference clamps at 0.
        VxOp::Vsubuws => vsub_uwords_sat(a, b),
        // [AltiVec-PEM p:6-157 s:6.2] vsubsbs: each signed byte difference clamps to [-2^7, 2^7 - 1].
        VxOp::Vsubsbs => vsub_sbytes_sat(a, b),
        // [AltiVec-PEM p:6-158 s:6.2] vsubshs: each signed half-word difference clamps to [-2^15, 2^15 - 1].
        VxOp::Vsubshs => vsub_shalfs_sat(a, b),
        // [AltiVec-PEM p:6-159 s:6.2] vsubsws: each signed word difference clamps to [-2^31, 2^31 - 1].
        VxOp::Vsubsws => vsub_swords_sat(a, b),

        // -- Maximum, minimum, average --
        // [AltiVec-PEM p:6-76 s:6.2] vmaxub: Vector Maximum Unsigned Byte
        VxOp::Vmaxub => (vmaxub(a, b), false),
        // [AltiVec-PEM p:6-77 s:6.2] vmaxuh: Vector Maximum Unsigned Half Word
        VxOp::Vmaxuh => (vmaxuh(a, b), false),
        // [AltiVec-PEM p:6-78 s:6.2] vmaxuw: Vector Maximum Unsigned Word
        VxOp::Vmaxuw => (vmaxuw(a, b), false),
        // [AltiVec-PEM p:6-73 s:6.2] vmaxsb: Vector Maximum Signed Byte
        VxOp::Vmaxsb => (vmaxsb(a, b), false),
        // [AltiVec-PEM p:6-74 s:6.2] vmaxsh: Vector Maximum Signed Half Word
        VxOp::Vmaxsh => (vmaxsh(a, b), false),
        // [AltiVec-PEM p:6-75 s:6.2] vmaxsw: Vector Maximum Signed Word
        VxOp::Vmaxsw => (vmaxsw(a, b), false),
        // [AltiVec-PEM p:6-85 s:6.2] vminub: Vector Minimum Unsigned Byte
        VxOp::Vminub => (vminub(a, b), false),
        // [AltiVec-PEM p:6-86 s:6.2] vminuh: Vector Minimum Unsigned Half Word
        VxOp::Vminuh => (vminuh(a, b), false),
        // [AltiVec-PEM p:6-87 s:6.2] vminuw: Vector Minimum Unsigned Word
        VxOp::Vminuw => (vminuw(a, b), false),
        // [AltiVec-PEM p:6-82 s:6.2] vminsb: Vector Minimum Signed Byte
        VxOp::Vminsb => (vminsb(a, b), false),
        // [AltiVec-PEM p:6-83 s:6.2] vminsh: Vector Minimum Signed Half Word
        VxOp::Vminsh => (vminsh(a, b), false),
        // [AltiVec-PEM p:6-84 s:6.2] vminsw: Vector Minimum Signed Word
        VxOp::Vminsw => (vminsw(a, b), false),
        // [AltiVec-PEM p:6-46 s:6.2] vavgub: zero-extend to 9 bits, add with 1, keep the high 8 bits.
        VxOp::Vavgub => (vavgub(a, b), false),
        // [AltiVec-PEM p:6-47 s:6.2] vavguh: zero-extend to 17 bits, add with 1, keep the high 16 bits.
        VxOp::Vavguh => (vavguh(a, b), false),
        // [AltiVec-PEM p:6-48 s:6.2] vavguw: zero-extend to 33 bits, add with 1, keep the high 32 bits.
        VxOp::Vavguw => (vavguw(a, b), false),
        // [AltiVec-PEM p:6-43 s:6.2] vavgsb: sign-extend to 9 bits, add with 1, keep the high 8 bits.
        VxOp::Vavgsb => (vavgsb(a, b), false),
        // [AltiVec-PEM p:6-44 s:6.2] vavgsh: sign-extend to 17 bits, add with 1, keep the high 16 bits.
        VxOp::Vavgsh => (vavgsh(a, b), false),
        // [AltiVec-PEM p:6-45 s:6.2] vavgsw: sign-extend to 33 bits, add with 1, keep the high 32 bits.
        VxOp::Vavgsw => (vavgsw(a, b), false),

        // -- Pack (vA then vB, narrowed; the saturating forms set SAT when an element clamps) --
        // [AltiVec-PEM p:6-119 s:6.2] vpkuhum: the low-order byte of each half word.
        VxOp::Vpkuhum => vpkuhum(a, b),
        // [AltiVec-PEM p:6-121 s:6.2] vpkuwum: the low-order half word of each word.
        VxOp::Vpkuwum => vpkuwum(a, b),
        // [AltiVec-PEM p:6-120 s:6.2] vpkuhus: each unsigned half word saturated to 2^8 - 1.
        VxOp::Vpkuhus => vpkuhus(a, b),
        // [AltiVec-PEM p:6-122 s:6.2] vpkuwus: each unsigned word saturated to 2^16 - 1.
        VxOp::Vpkuwus => vpkuwus(a, b),
        // [AltiVec-PEM p:6-116 s:6.2] vpkshus: each signed half word saturated to [0, 2^8 - 1]; the prose converts the whole half word, where the pseudocode names only its high byte.
        VxOp::Vpkshus => vpkshus(a, b),
        // [AltiVec-PEM p:6-118 s:6.2] vpkswus: each signed word saturated to [0, 2^16 - 1].
        VxOp::Vpkswus => vpkswus(a, b),
        // [AltiVec-PEM p:6-115 s:6.2] vpkshss: each signed half word saturated to [-2^7, 2^7 - 1].
        VxOp::Vpkshss => vpkshss(a, b),
        // [AltiVec-PEM p:6-117 s:6.2] vpkswss: each signed word saturated to [-2^15, 2^15 - 1].
        VxOp::Vpkswss => vpkswss(a, b),
        // [AltiVec-PEM p:6-113 s:6.2] vpkpx: each word packs to its bit 7, bits 8:12, bits 16:20 and bits 24:28, in order.
        VxOp::Vpkpx => vpkpx(a, b),

        // -- Sum across (each sets SAT when a word clamps) --
        // [AltiVec-PEM p:6-166 s:6.2] vsumsws: vB word 3 plus the four vA words in 35 bits, saturated to a signed word in vD word 3; vD words 0..2 are cleared.
        VxOp::Vsumsws => vsumsws(a, b),
        // [AltiVec-PEM p:6-167 s:6.2] vsum2sws: vA words 0 + 1 plus vB word 1 into vD word 1, vA words 2 + 3 plus vB word 3 into vD word 3, each saturated to a signed word; vD words 0 and 2 are cleared.
        VxOp::Vsum2sws => vsum2sws(a, b),
        // [AltiVec-PEM p:6-168 s:6.2] vsum4sbs: each vB word plus the four signed bytes of the vA word, saturated to a signed word.
        VxOp::Vsum4sbs => vsum4sbs(a, b),
        // [AltiVec-PEM p:6-169 s:6.2] vsum4shs: each vB word plus the two signed half words of the vA word, saturated to a signed word.
        VxOp::Vsum4shs => vsum4shs(a, b),
        // [AltiVec-PEM p:6-170 s:6.2] vsum4ubs: each vB word plus the four unsigned bytes of the vA word, saturated to an unsigned word.
        VxOp::Vsum4ubs => vsum4ubs(a, b),

        // -- Unpack (vA is a reserved field; only vB is read) --
        // [AltiVec-PEM p:6-172 s:6.2] vupkhsb: sign-extend each byte of the high half of vB to a half word.
        VxOp::Vupkhsb => (vupkhsb(b), false),
        // [AltiVec-PEM p:6-175 s:6.2] vupklsb: sign-extend each byte of the low half of vB to a half word.
        VxOp::Vupklsb => (vupklsb(b), false),
        // [AltiVec-PEM p:6-173 s:6.2] vupkhsh: sign-extend each half word of the high half of vB to a word.
        VxOp::Vupkhsh => (vupkhsh(b), false),
        // [AltiVec-PEM p:6-176 s:6.2] vupklsh: sign-extend each half word of the low half of vB to a word.
        VxOp::Vupklsh => (vupklsh(b), false),
        // [AltiVec-PEM p:6-171 s:6.2] vupkhpx: each high-half pixel becomes bit 0 sign-extended to 8 bits, then bits 1:5, 6:10 and 11:15 each zero-extended to 8 bits.
        VxOp::Vupkhpx => (vupkhpx(b), false),
        // [AltiVec-PEM p:6-174 s:6.2] vupklpx: the same for each low-half pixel.
        VxOp::Vupklpx => (vupklpx(b), false),

        // -- Whole-register shifts --
        // [AltiVec-PEM p:6-138 s:6.2] vslo: vA shifted left by vB bits 121:124 octets, zero fill.
        VxOp::Vslo => (vslo(a, b), false),
        // [AltiVec-PEM p:6-153 s:6.2] vsro: vA shifted right by vB bits 121:124 octets, zero fill.
        VxOp::Vsro => (vsro(a, b), false),
        // [AltiVec-PEM p:6-134 s:6.2] vsl: vA shifted left by vB bits 125:127 when the low three bits of every vB byte agree; vD is undefined otherwise.
        // The ppu_reference_vectors capture settles the undefined case: each byte shifts by the count in its own vB byte.
        VxOp::Vsl => (vsl(a, b), false),
        // [AltiVec-PEM p:6-146 s:6.2] vsr: vA shifted right by vB bits 125:127 when the low three bits of every vB byte agree; vD is undefined otherwise.
        // The ppu_reference_vectors capture settles the undefined case as for vsl.
        VxOp::Vsr => (vsr(a, b), false),

        // -- Fixed-point <-> single-precision conversions (the vA field is the UIMM scale) --
        // [AltiVec-PEM p:6-49 s:6.2] vcfsx: each signed word to the nearest single-precision value, divided by 2^UIMM.
        VxOp::Vcfsx => (vcfsx(b, va), false),
        // [AltiVec-PEM p:6-50 s:6.2] vcfux: the same for each unsigned word.
        VxOp::Vcfux => (vcfux(b, va), false),
        // [AltiVec-PEM p:6-65 s:6.2] vctsxs: each slot x 2^UIMM toward zero, saturated to a signed word; saturation sets SAT.
        VxOp::Vctsxs => vctsxs(mode, b, va),
        // [AltiVec-PEM p:6-66 s:6.2] vctuxs: the same, saturated to an unsigned word.
        VxOp::Vctuxs => vctuxs(mode, b, va),

        // -- Single-precision compares (the engine orders the slots; NJ flushes denormals) --
        // [AltiVec-PEM p:6-53 s:6.2] vcmpeqfp: all ones where vA = vB, 0 where not or where either is a NaN.
        VxOp::Vcmpeqfp => (vcmpeqfp(mode, a, b), false),
        // [AltiVec-PEM p:6-57 s:6.2] vcmpgefp: all ones where vA >= vB, 0 where not or where either is a NaN.
        VxOp::Vcmpgefp => (vcmpgefp(mode, a, b), false),
        // [AltiVec-PEM p:6-58 s:6.2] vcmpgtfp: all ones where vA > vB, 0 where not or where either is a NaN.
        VxOp::Vcmpgtfp => (vcmpgtfp(mode, a, b), false),
        // [AltiVec-PEM p:6-51 s:6.2] vcmpbfp: bit 0 clear when vA <= vB, bit 1 clear when vA >= -vB, the other bits 0.
        // [AltiVec-PEM p:3-13 s:3.2.4.1] a NaN operand gives 0xC000_0000.
        VxOp::Vcmpbfp => (vcmpbfp(mode, a, b), false),

        // -- Single-precision maximum and minimum (no rounding) --
        // [AltiVec-PEM p:6-72 s:6.2] vmaxfp: the larger slot; the maximum of +0 and -0 is +0; a NaN operand gives a QNaN.
        // The page's pseudocode returns vA for vmaxfp(-0, +0) and vB for a NaN in vA; the ppu_vfp_maxmin console capture follows the text in both orders and the vA-first NaN rule.
        VxOp::Vmaxfp => (vmaxfp(mode, a, b), false),
        // [AltiVec-PEM p:6-81 s:6.2] vminfp: the smaller slot; the minimum of +0 and -0 is -0; a NaN operand gives a QNaN.
        VxOp::Vminfp => (vminfp(mode, a, b), false),

        // -- Single-precision add and subtract --
        // [AltiVec-PEM p:6-31 s:6.2] vaddfp: vA + vB rounded to nearest; NJ = 1 truncates a denormal operand or result to a zero of its sign.
        VxOp::Vaddfp => (vaddfp(mode, a, b), false),
        // [AltiVec-PEM p:6-156 s:6.2] vsubfp: vA - vB rounded to nearest, under the same NJ rule.
        VxOp::Vsubfp => (vsubfp(mode, a, b), false),

        // -- Single-precision round to integer (vD, vB) --
        // [AltiVec-PEM p:6-126 s:6.2] vrfin: each vB slot rounded to the nearest integer value; the result is independent of NJ.
        VxOp::Vrfin => (vrfi(mode, Rounding::NearestEven, b), false),
        // [AltiVec-PEM p:6-128 s:6.2] vrfiz: rounded toward zero; the result is independent of NJ.
        VxOp::Vrfiz => (vrfi(mode, Rounding::TowardZero, b), false),
        // [AltiVec-PEM p:6-127 s:6.2] vrfip: rounded toward plus infinity; NJ = 1 truncates a denormal operand to 0 first.
        VxOp::Vrfip => (vrfi(mode, Rounding::TowardPositive, b), false),
        // [AltiVec-PEM p:6-125 s:6.2] vrfim: rounded toward minus infinity; the page names no NJ rule, so the chapter 3 rule flushes a denormal operand under NJ = 1.
        VxOp::Vrfim => (vrfi(mode, Rounding::TowardNegative, b), false),

        // `vx_executes` refuses these before the match.
        VxOp::Vrefp | VxOp::Vrsqrtefp | VxOp::Vexptefp | VxOp::Vlogefp => {
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

/// Whether `execute_vx` executes `op` with record bit `rc`. Every other
/// pair faults as unimplemented, and only a VXR compare has a record form.
pub(crate) fn vx_executes(op: VxOp, rc: bool) -> bool {
    let unimplemented = matches!(
        op,
        VxOp::Vrefp | VxOp::Vrsqrtefp | VxOp::Vexptefp | VxOp::Vlogefp
    );
    !unimplemented && (!rc || op.is_vxr_compare())
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

/// Execute a VA-form VMX instruction (primary=4, 6-bit sub-opcode, 4
/// registers). A saturating op that saturates sets `VSCR[SAT]`.
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
    let mode = VmxMode::from_nj(state.vscr() & VSCR_NJ != 0);

    let (result, saturated) = match op {
        // [AltiVec-PEM p:6-133 s:6.2] vsel: Vector Select
        VaOp::Vsel => (vsel(a, b, c), false),
        // [AltiVec-PEM p:6-112 s:6.2] vperm: Vector Permute
        VaOp::Vperm => (vperm(a, b, c), false),
        // Decode routes vsldoi to the typed variant; this arm keeps
        // the match total for hand-built instructions.
        VaOp::Vsldoi => (vsldoi(a, b, vc & 0xF), false),

        // -- Integer multiply-add and multiply-sum (vD, vA, vB, vC) --
        // [AltiVec-PEM p:6-79 s:6.2] vmhaddshs: bits 0:16 of the signed product vA x vB, plus vC sign-extended to 17 bits, saturated to a signed half word.
        VaOp::Vmhaddshs => vmhaddshs(a, b, c),
        // [AltiVec-PEM p:6-80 s:6.2] vmhraddshs: as vmhaddshs with 0x0000_4000 added to the product first; the pseudocode adds vC.
        VaOp::Vmhraddshs => vmhraddshs(a, b, c),
        // [AltiVec-PEM p:6-88 s:6.2] vmladduhm: the low half word of vA x vB + vC.
        VaOp::Vmladduhm => vmladduhm(a, b, c),
        // [AltiVec-PEM p:6-98 s:6.2] vmsumubm: each vC word plus the four unsigned byte products, modulo.
        VaOp::Vmsumubm => vmsumubm(a, b, c),
        // [AltiVec-PEM p:6-95 s:6.2] vmsummbm: each vC word plus the four signed vA byte x unsigned vB byte products, modulo.
        VaOp::Vmsummbm => vmsummbm(a, b, c),
        // [AltiVec-PEM p:6-99 s:6.2] vmsumuhm: each vC word plus the two unsigned half-word products, modulo.
        VaOp::Vmsumuhm => vmsumuhm(a, b, c),
        // [AltiVec-PEM p:6-100 s:6.2] vmsumuhs: each vC word plus the two unsigned half-word products in 34 bits, saturated to an unsigned word.
        VaOp::Vmsumuhs => vmsumuhs(a, b, c),
        // [AltiVec-PEM p:6-96 s:6.2] vmsumshm: each vC word plus the two signed half-word products, modulo.
        VaOp::Vmsumshm => vmsumshm(a, b, c),
        // [AltiVec-PEM p:6-97 s:6.2] vmsumshs: each vC word plus the two signed half-word products in 34 bits, saturated to a signed word.
        // The page's pseudocode saturates inside the product loop and its prose saturates the final sum once; the ppu_vmul_add_sum console capture settles it as the final sum.
        VaOp::Vmsumshs => vmsumshs(a, b, c),

        // -- Single-precision multiply-add (vD, vA, vC, vB) --
        // [AltiVec-PEM p:6-71 s:6.2] vmaddfp: (vA x vC) + vB rounded once to nearest; NJ = 1 truncates a denormal operand or result to a zero of its sign.
        // [AltiVec-PEM p:3-15 s:3.2.5.1] a NaN result takes vA's NaN, then vB's, then vC's.
        VaOp::Vmaddfp => (vmaddfp(mode, a, b, c), false),
        // [AltiVec-PEM p:6-109 s:6.2] vnmsubfp: -((vA x vC) - vB) with one rounding; a QNaN result is not negated.
        VaOp::Vnmsubfp => (vnmsubfp(mode, a, b, c), false),
    };

    state.set_vr(vt as usize, result);
    if saturated {
        state.set_sat();
    }
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
