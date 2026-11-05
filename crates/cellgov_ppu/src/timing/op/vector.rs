//! The builders for vector instructions.

use super::model::{unit, vr, Op};
use crate::instruction::ops::VxOp;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// A vector operation's latency class.
pub(super) fn vx_class(op: VxOp) -> (u8, bool, u64) {
    use VxOp as V;
    match op {
        V::Vmrghb
        | V::Vmrghh
        | V::Vmrghw
        | V::Vmrglb
        | V::Vmrglh
        | V::Vmrglw
        | V::Vpkuhum
        | V::Vpkuwum
        | V::Vpkuhus
        | V::Vpkuwus
        | V::Vpkshus
        | V::Vpkswus
        | V::Vpkshss
        | V::Vpkswss
        | V::Vpkpx
        | V::Vupkhsb
        | V::Vupkhsh
        | V::Vupklsb
        | V::Vupklsh
        | V::Vupkhpx
        | V::Vupklpx
        | V::Vspltb
        | V::Vsplth
        | V::Vspltw
        | V::Vspltisb
        | V::Vspltish
        | V::Vspltisw
        | V::Vslo
        | V::Vsro => (unit::VSU2, true, k::PPU_VMX_PERMUTE_LATENCY_CYCLES),
        V::Vaddfp
        | V::Vsubfp
        | V::Vlogefp
        | V::Vexptefp
        | V::Vrfin
        | V::Vrfiz
        | V::Vrfip
        | V::Vrfim
        | V::Vcfux
        | V::Vcfsx
        | V::Vctuxs
        | V::Vctsxs => (unit::VSU1, false, k::PPU_VMX_FLOAT_LATENCY_CYCLES),
        V::Vrefp | V::Vrsqrtefp => (unit::VSU1, false, k::PPU_VMX_ESTIMATE_LATENCY_CYCLES),
        V::Vmuloub
        | V::Vmulouh
        | V::Vmulosb
        | V::Vmulosh
        | V::Vmuleub
        | V::Vmuleuh
        | V::Vmulesb
        | V::Vmulesh
        | V::Vsum4ubs
        | V::Vsum4shs
        | V::Vsum4sbs
        | V::Vsum2sws
        | V::Vsumsws => (unit::VSU1, false, k::PPU_VMX_COMPLEX_LATENCY_CYCLES),
        V::Vaddubm
        | V::Vmaxub
        | V::Vrlb
        | V::Vcmpequb
        | V::Vadduhm
        | V::Vmaxuh
        | V::Vrlh
        | V::Vcmpequh
        | V::Vadduwm
        | V::Vmaxuw
        | V::Vrlw
        | V::Vcmpequw
        | V::Vcmpeqfp
        | V::Vmaxsb
        | V::Vslb
        | V::Vmaxsh
        | V::Vslh
        | V::Vaddcuw
        | V::Vmaxsw
        | V::Vslw
        | V::Vsl
        | V::Vcmpgefp
        | V::Vaddubs
        | V::Vminub
        | V::Vsrb
        | V::Vcmpgtub
        | V::Vadduhs
        | V::Vminuh
        | V::Vsrh
        | V::Vcmpgtuh
        | V::Vadduws
        | V::Vminuw
        | V::Vsrw
        | V::Vcmpgtuw
        | V::Vsr
        | V::Vcmpgtfp
        | V::Vaddsbs
        | V::Vminsb
        | V::Vsrab
        | V::Vcmpgtsb
        | V::Vaddshs
        | V::Vminsh
        | V::Vsrah
        | V::Vcmpgtsh
        | V::Vaddsws
        | V::Vminsw
        | V::Vsraw
        | V::Vcmpgtsw
        | V::Vcmpbfp
        | V::Vsububm
        | V::Vavgub
        | V::Vand
        | V::Vmaxfp
        | V::Vsubuhm
        | V::Vavguh
        | V::Vandc
        | V::Vminfp
        | V::Vsubuwm
        | V::Vavguw
        | V::Vor
        | V::Vxor
        | V::Vavgsb
        | V::Vnor
        | V::Vavgsh
        | V::Vsubcuw
        | V::Vavgsw
        | V::Vsububs
        | V::Vsubuhs
        | V::Vsubuws
        | V::Vsubsbs
        | V::Vsubshs
        | V::Vsubsws => (unit::VSU1, false, k::PPU_VMX_SIMPLE_LATENCY_CYCLES),
    }
}

pub(super) fn vector(units: u8, permute: bool, lat: u64, vt: u8, srcs: &[u8]) -> Op {
    let mut op = Op::on(units).dst(vr(vt), lat);
    op.permute = permute;
    for &s in srcs {
        op = op.src(vr(s));
    }
    op
}
