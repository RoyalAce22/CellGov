//! The places one instruction leaves open from a given start state, as
//! the documented sets name them.
//!
//! The sets enumerate inputs. This module takes one observed step and
//! names the end-state bits that the documents leave undefined for that
//! word and those inputs. A disagreement that falls only on those bits
//! is no defect of either side.

use cellgov_ppu::decode::decode;
use cellgov_ppu::differential::PpuStateSnapshot;
use cellgov_ppu::instruction::{Fp59Op, Fp63Op, PpuInstruction, VxOp};
use cellgov_ps3_abi::hw::ppc_isa::PPC_FPSCR_OE;
use cellgov_ps3_abi::hw::ppu_status::{
    CR0_EQ, CR0_LT_GT_EQ, FPSCR_FPRF, FPSCR_FR, FPSCR_NI, FPSCR_OX, XER_BYTE_COUNT, XER_RESERVED,
};
use cellgov_sync::ReservedLine;

use super::generate::draft::{DWORD, HI};
use super::generate::float::{FCTIW, FP59, MFFS, NI, OVERFLOW, STATUS, STFS, WORD};
use super::generate::integer::{field_mask, DIV, DIVU, LSWX, MFOCRF, MTOCRF, MULH, RESERVED};
use super::generate::storage::{DCBF, STCX};
use super::generate::vmx::{LVEBX, LVEHX, LVEWX, VSL, VSR};
use super::set::Set;

/// A piece of the end state the documents leave open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenAt {
    /// Bits of a general-purpose register.
    Gpr {
        /// The register.
        index: u8,
        /// The open bits.
        mask: u64,
    },
    /// Bits of a floating-point register.
    Fpr {
        /// The register.
        index: u8,
        /// The open bits.
        mask: u64,
    },
    /// Bits of a vector register.
    Vr {
        /// The register.
        index: u8,
        /// The open bits.
        mask: u128,
    },
    /// Bits of the condition register.
    Cr(u32),
    /// Bits of the fixed-point exception register.
    Xer(u64),
    /// Bits of the floating-point status and control register.
    Fpscr(u32),
    /// Bytes of storage.
    Memory {
        /// The first byte.
        ea: u64,
        /// The byte count.
        len: u64,
    },
    /// Whether a reservation remains.
    Reservation,
}

/// One open place, with the set that reaches it and the page that leaves
/// it open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenPlace {
    /// The set whose vectors reach the place.
    pub set: Set,
    /// The open bits.
    pub at: OpenAt,
    /// The page that leaves them open.
    pub citation: &'static str,
}

/// The places `raw` leaves open when it runs from `pre`.
///
/// `posts` are the end states that the two sides of a comparison reach.
/// The documents leave FR open after an overflow, and only `posts` show
/// the overflow.
pub fn open_places(
    raw: u32,
    pre: &PpuStateSnapshot,
    posts: [&PpuStateSnapshot; 2],
) -> Vec<OpenPlace> {
    let Ok(insn) = decode(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut open = |set: Set, at: OpenAt, citation: &'static str| {
        out.push(OpenPlace { set, at, citation });
    };
    fixed_point(&insn, pre, &mut open);
    floating_point(&insn, pre, posts, &mut open);
    storage(&insn, pre, &mut open);
    vector(&insn, pre, &mut open);
    out
}

type Open<'a> = dyn FnMut(Set, OpenAt, &'static str) + 'a;

/// `muldiv-undef` and `move-undef`.
fn fixed_point(insn: &PpuInstruction, pre: &PpuStateSnapshot, open: &mut Open<'_>) {
    let set = Set::MuldivUndef;
    let gpr = |index: u8, mask: u128| OpenAt::Gpr {
        index,
        mask: mask as u64,
    };
    let cr0 = OpenAt::Cr(CR0_LT_GT_EQ);
    match *insn {
        PpuInstruction::Mulhw { rt, rc, .. } | PpuInstruction::Mulhwu { rt, rc, .. } => {
            open(set, gpr(rt, HI), MULH);
            if rc {
                open(set, cr0, MULH);
            }
        }
        PpuInstruction::Divw { rt, ra, rb, rc, .. }
        | PpuInstruction::Divwu { rt, ra, rb, rc, .. } => {
            let signed = matches!(insn, PpuInstruction::Divw { .. });
            let cite = if signed { DIV } else { DIVU };
            let (a, b) = (pre.gpr[ra as usize] as u32, pre.gpr[rb as usize] as u32);
            let undefined = b == 0 || (signed && a == 0x8000_0000 && b == u32::MAX);
            open(set, gpr(rt, if undefined { DWORD } else { HI }), cite);
            if rc {
                open(set, cr0, cite);
            }
        }
        PpuInstruction::Divd { rt, ra, rb, rc, .. }
        | PpuInstruction::Divdu { rt, ra, rb, rc, .. } => {
            let signed = matches!(insn, PpuInstruction::Divd { .. });
            let cite = if signed { DIV } else { DIVU };
            let (a, b) = (pre.gpr[ra as usize], pre.gpr[rb as usize]);
            let undefined = b == 0 || (signed && a == 1 << 63 && b == u64::MAX);
            if !undefined {
                return;
            }
            open(set, gpr(rt, DWORD), cite);
            if rc {
                open(set, cr0, cite);
            }
        }
        PpuInstruction::Mtocrf { crm, .. } if crm.count_ones() != 1 => {
            open(Set::MoveUndef, OpenAt::Cr(u32::MAX), MTOCRF);
        }
        PpuInstruction::Mfocrf { rt, crm } => {
            let mask = if crm.count_ones() == 1 {
                DWORD & !field_mask(crm.leading_zeros())
            } else {
                DWORD
            };
            open(Set::MoveUndef, gpr(rt, mask), MFOCRF);
        }
        PpuInstruction::Lswx { rt, .. } if pre.xer & XER_BYTE_COUNT == 0 => {
            open(Set::MoveUndef, gpr(rt, DWORD), LSWX);
        }
        PpuInstruction::Mtxer { rs } if pre.gpr[rs as usize] & XER_RESERVED != 0 => {
            open(Set::MoveUndef, OpenAt::Xer(XER_RESERVED), RESERVED);
        }
        _ => {}
    }
}

/// `fctiw-high-word`, `mffs-high-word`, `fp59-wide-operand`,
/// `fp-overflow-fr`, `stfs-below-denormal` and `fpscr-ni`.
fn floating_point(
    insn: &PpuInstruction,
    pre: &PpuStateSnapshot,
    posts: [&PpuStateSnapshot; 2],
    open: &mut Open<'_>,
) {
    let fpr = |index: u8, mask: u128| OpenAt::Fpr {
        index,
        mask: mask as u64,
    };
    let Some(arith) = arithmetic(insn) else {
        if let Some((ea, frs)) = stfs_target(insn, pre) {
            if below_single_denormal(pre.fpr[frs as usize]) {
                open(Set::StfsBelowDenormal, OpenAt::Memory { ea, len: 4 }, STFS);
            }
        }
        if let PpuInstruction::Fp63 {
            op: Fp63Op::Mffs,
            frt,
            ..
        } = *insn
        {
            open(Set::MffsHighWord, fpr(frt, HI), MFFS);
        }
        return;
    };
    if let PpuInstruction::Fp63 {
        op: Fp63Op::Fctiw | Fp63Op::Fctiwz,
        frt,
        ..
    } = *insn
    {
        open(Set::FctiwHighWord, fpr(frt, HI), FCTIW);
        open(Set::FctiwHighWord, OpenAt::Fpscr(FPSCR_FPRF), FCTIW);
    }
    if arith.single
        && arith
            .operands
            .iter()
            .any(|&r| !single_holds(pre.fpr[r as usize]))
    {
        open(Set::Fp59WideOperand, fpr(arith.frt, DWORD), FP59);
        open(Set::Fp59WideOperand, OpenAt::Fpscr(STATUS as u32), FP59);
    }
    let overflowed = posts
        .iter()
        .any(|post| pre.fpscr & FPSCR_OX == 0 && post.fpscr & FPSCR_OX != 0);
    if pre.fpscr & PPC_FPSCR_OE == 0 && overflowed {
        open(Set::FpOverflowFr, OpenAt::Fpscr(FPSCR_FR), OVERFLOW);
    }
    if pre.fpscr & FPSCR_NI != 0 {
        open(Set::FpscrNi, fpr(arith.frt, DWORD), NI);
        open(Set::FpscrNi, OpenAt::Fpscr(WORD as u32), NI);
    }
}

/// A floating-point operation that computes a result: its target, the
/// registers it reads, and whether it is single precision.
struct Arithmetic {
    frt: u8,
    operands: Vec<u8>,
    single: bool,
}

/// The operation's target and operands, or `None` for a move, a
/// compare, a select or an FPSCR instruction.
fn arithmetic(insn: &PpuInstruction) -> Option<Arithmetic> {
    let build = |frt, operands: &[u8], single| {
        Some(Arithmetic {
            frt,
            operands: operands.to_vec(),
            single,
        })
    };
    match *insn {
        PpuInstruction::Fp59 {
            op,
            frt,
            fra,
            frb,
            frc,
            ..
        } => match op {
            Fp59Op::Fadds | Fp59Op::Fsubs | Fp59Op::Fdivs => build(frt, &[fra, frb], true),
            Fp59Op::Fmuls => build(frt, &[fra, frc], true),
            Fp59Op::Fsqrts => build(frt, &[frb], true),
            Fp59Op::Fmadds | Fp59Op::Fmsubs | Fp59Op::Fnmadds | Fp59Op::Fnmsubs => {
                build(frt, &[fra, frc, frb], true)
            }
            Fp59Op::Fres => build(frt, &[frb], false),
        },
        PpuInstruction::Fp63 {
            op,
            frt,
            fra,
            frb,
            frc,
            ..
        } => match op {
            Fp63Op::Fadd | Fp63Op::Fsub | Fp63Op::Fdiv => build(frt, &[fra, frb], false),
            Fp63Op::Fmul => build(frt, &[fra, frc], false),
            Fp63Op::Fmadd | Fp63Op::Fmsub | Fp63Op::Fnmadd | Fp63Op::Fnmsub => {
                build(frt, &[fra, frc, frb], false)
            }
            Fp63Op::Fsqrt
            | Fp63Op::Frsqrte
            | Fp63Op::Frsp
            | Fp63Op::Fctiw
            | Fp63Op::Fctiwz
            | Fp63Op::Fctid
            | Fp63Op::Fctidz
            | Fp63Op::Fcfid => build(frt, &[frb], false),
            _ => None,
        },
        _ => None,
    }
}

/// The double fields of `bits`: the biased exponent and the fraction.
fn fields(bits: u64) -> (u64, u64) {
    ((bits >> 52) & 0x7ff, bits & ((1 << 52) - 1))
}

/// Whether single format holds the double `bits` exactly: a zero, or a
/// value whose exponent and fraction fit.
fn single_holds(bits: u64) -> bool {
    let (exp, frac) = fields(bits);
    // A single fraction is 23 bits; the double's low 29 must be zero.
    let low = |n: u32| frac & ((1u64 << n) - 1) == 0;
    match exp {
        0 => frac == 0,
        0x7ff => low(29),
        _ => {
            let e = exp as i64 - 1023;
            match e {
                -126..=127 => low(29),
                // A single denormal keeps 149 + e fraction bits.
                -149..=-127 => low((29 - 126 - e) as u32),
                _ => false,
            }
        }
    }
}

/// Whether the double `bits` is nonzero and below the single denormal
/// range, whose least magnitude is 2^-149.
fn below_single_denormal(bits: u64) -> bool {
    let (exp, frac) = fields(bits);
    if exp == 0 {
        return frac != 0;
    }
    exp != 0x7ff && (exp as i64 - 1023) < -149
}

/// The effective address and source register of a single-precision
/// store.
fn stfs_target(insn: &PpuInstruction, pre: &PpuStateSnapshot) -> Option<(u64, u8)> {
    let base = |ra: u8| if ra == 0 { 0 } else { pre.gpr[ra as usize] };
    match *insn {
        PpuInstruction::Stfs { frs, ra, imm } => {
            Some((base(ra).wrapping_add(imm as i64 as u64), frs))
        }
        PpuInstruction::Stfsu { frs, ra, imm } => {
            Some((pre.gpr[ra as usize].wrapping_add(imm as i64 as u64), frs))
        }
        PpuInstruction::Stfsx { frs, ra, rb } => {
            Some((base(ra).wrapping_add(pre.gpr[rb as usize]), frs))
        }
        PpuInstruction::Stfsux { frs, ra, rb } => {
            Some((pre.gpr[ra as usize].wrapping_add(pre.gpr[rb as usize]), frs))
        }
        _ => None,
    }
}

/// `(RA|0) + RB`.
fn indexed(pre: &PpuStateSnapshot, ra: u8, rb: u8) -> u64 {
    let base = if ra == 0 { 0 } else { pre.gpr[ra as usize] };
    base.wrapping_add(pre.gpr[rb as usize])
}

/// `stwcx-other-address` and `reservation-undefined`.
///
/// The record holds only the reserved line. A store conditional is
/// therefore open only when its line differs from the reserved line.
fn storage(insn: &PpuInstruction, pre: &PpuStateSnapshot, open: &mut Open<'_>) {
    let Some(reserved) = pre.reservation else {
        return;
    };
    let other_line = |ea: u64| ReservedLine::containing(ea) != reserved;
    match *insn {
        PpuInstruction::Stwcx { ra, rb, .. } | PpuInstruction::Stdcx { ra, rb, .. } => {
            let ea = indexed(pre, ra, rb);
            if other_line(ea) {
                let len = if matches!(insn, PpuInstruction::Stwcx { .. }) {
                    4
                } else {
                    8
                };
                open(Set::StwcxOtherAddress, OpenAt::Memory { ea, len }, STCX);
                open(Set::StwcxOtherAddress, OpenAt::Cr(CR0_EQ), STCX);
            }
        }
        PpuInstruction::Dcbf { ra, rb, .. } if !other_line(indexed(pre, ra, rb)) => {
            open(Set::ReservationUndefined, OpenAt::Reservation, DCBF);
        }
        _ => {}
    }
}

/// `lve-other-lanes` and `vshift-nonuniform`.
fn vector(insn: &PpuInstruction, pre: &PpuStateSnapshot, open: &mut Open<'_>) {
    let element = |vt: u8, ra: u8, rb: u8, size: u64, citation: &'static str| {
        let m = indexed(pre, ra, rb) & 0xf & !(size - 1);
        let shift = 8 * (16 - m - size);
        let loaded = (u128::MAX >> (128 - 8 * size)) << shift;
        (
            OpenAt::Vr {
                index: vt,
                mask: !loaded,
            },
            citation,
        )
    };
    let lanes = match *insn {
        PpuInstruction::Lvebx { vt, ra, rb } => Some(element(vt, ra, rb, 1, LVEBX)),
        PpuInstruction::Lvehx { vt, ra, rb } => Some(element(vt, ra, rb, 2, LVEHX)),
        PpuInstruction::Lvewx { vt, ra, rb } => Some(element(vt, ra, rb, 4, LVEWX)),
        _ => None,
    };
    if let Some((at, citation)) = lanes {
        open(Set::LveOtherLanes, at, citation);
    }
    if let PpuInstruction::Vx {
        op: op @ (VxOp::Vsl | VxOp::Vsr),
        vt,
        vb,
        ..
    } = *insn
    {
        let counts = pre.vr[vb as usize].to_be_bytes().map(|byte| byte & 7);
        if counts.iter().any(|&count| count != counts[0]) {
            let citation = if op == VxOp::Vsl { VSL } else { VSR };
            open(
                Set::VshiftNonuniform,
                OpenAt::Vr {
                    index: vt,
                    mask: u128::MAX,
                },
                citation,
            );
        }
    }
}
