//! How the floating-point and vector words move a copy of CTR into and
//! out of the FPRs, the VRs, FPSCR and VSCR.
//!
//! The CTR flow keeps one mark for all of these registers. A copy gets
//! into one of them in these ways:
//! - a floating-point or vector load of a stack slot that holds a copy;
//! - a vector word that places bits of its EA into a VR, when a GPR of
//!   the EA holds a copy.
//!
//! A copy gets back into a GPR in these ways:
//! - a floating-point or vector store, and then a load into a GPR;
//! - a word that sets a CR field from these registers, and then a move
//!   out of CR.
//!
//! [PPC-Book1 p:86 s:4.2] The floating-point instructions move data between storage and the FPRs, and the computational ones place status information into FPSCR.
//! [AltiVec-PEM p:2-1 s:2.1] The VRs are separate from the GPRs and the FPRs, and only the vector loads and stores move data between storage and the VRs.

use super::cells::Cells;
use super::constants::gpr_bit;
use super::slot_set::Slot;
use crate::instruction::PpuInstruction;

/// The bytes that a vector load or store with EA = r1 can reach, as a
/// slot from r1.
///
/// The word aligns EA down within the quadword that holds the byte at
/// r1, and the flow does not know the alignment of r1. So the word
/// reaches bytes from 15 below r1 to 15 above it, and the flow can place
/// no byte that it surely writes.
///
/// [AltiVec-PEM p:6-21 s:6.2] `lvx` loads the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [AltiVec-PEM p:6-28 s:6.2] `stvx` stores VS into the quadword at EA = ((RA|0) + (RB)) & ~0xF.
/// [AltiVec-PEM p:6-14 s:6.2] `lvebx` loads the byte at EA into byte EA\[28:31\] of VD.
/// [CBE-Handbook p:710 s:24.9] `lvlx` and `lvrx` load the left and right parts of a quadword at an unaligned EA, and `stvlx` and `stvrx` store them.
pub(super) const VECTOR_SPAN: Slot = (-15, 31);

/// Whether a vector word loads or stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Load,
    Store,
}

/// The access of a vector load or store whose EA is r1, with RA at 0
/// and RB at 1, or `None` for any other word.
///
/// A vector load or store has no displacement, so this is the only form
/// whose EA the flow can place on the stack.
///
/// [AltiVec-PEM p:6-21 s:6.2] `lvx` computes EA from (RA|0) + (RB).
pub(super) fn vector_at_r1(decoded: &PpuInstruction) -> Option<Access> {
    use PpuInstruction as I;
    let (access, ra, rb) = match *decoded {
        I::Lvx { ra, rb, .. }
        | I::Lvxl { ra, rb, .. }
        | I::Lvebx { ra, rb, .. }
        | I::Lvehx { ra, rb, .. }
        | I::Lvewx { ra, rb, .. }
        | I::Lvlx { ra, rb, .. }
        | I::Lvlxl { ra, rb, .. }
        | I::Lvrx { ra, rb, .. }
        | I::Lvrxl { ra, rb, .. } => (Access::Load, ra, rb),
        I::Stvx { ra, rb, .. }
        | I::Stvxl { ra, rb, .. }
        | I::Stvebx { ra, rb, .. }
        | I::Stvehx { ra, rb, .. }
        | I::Stvewx { ra, rb, .. }
        | I::Stvlx { ra, rb, .. }
        | I::Stvlxl { ra, rb, .. }
        | I::Stvrx { ra, rb, .. }
        | I::Stvrxl { ra, rb, .. } => (Access::Store, ra, rb),
        _ => return None,
    };
    (ra == 0 && rb == 1).then_some(access)
}

/// The GPRs whose bits `decoded` places into its VR through its EA, as a
/// mask with bit `n` set for GPR `n`.
///
/// These words place bits of EA into the VR:
/// - `lvsl` and `lvsr` compute the VR from EA\[28:31\];
/// - an element load places its element at the position that EA\[28:31\]
///   gives;
/// - `lvlx` and `lvrx` place their bytes at a position that EA gives.
///
/// [AltiVec-PEM p:6-18 s:6.2] `lvsl` places a vector computed from sh = EA\[28:31\] into VD.
/// [AltiVec-PEM p:6-20 s:6.2] `lvsr` places a vector computed from sh = EA\[28:31\] into VD.
/// [AltiVec-PEM p:6-14 s:6.2] `lvebx` loads the byte at EA into byte EA\[28:31\] of VD.
/// [AltiVec-PEM p:6-16 s:6.2] `lvehx` loads the halfword at EA into halfword EA\[28:30\] of VD.
/// [AltiVec-PEM p:6-17 s:6.2] `lvewx` loads the word at EA into word EA\[28:29\] of VD.
/// [CBE-Handbook p:710 s:24.9] `lvlx` and `lvrx` load the left and right parts of a quadword at an unaligned EA.
pub(super) fn address_into_vector(decoded: &PpuInstruction) -> u32 {
    use PpuInstruction as I;
    match *decoded {
        I::Lvsl { ra, rb, .. }
        | I::Lvsr { ra, rb, .. }
        | I::Lvebx { ra, rb, .. }
        | I::Lvehx { ra, rb, .. }
        | I::Lvewx { ra, rb, .. }
        | I::Lvlx { ra, rb, .. }
        | I::Lvlxl { ra, rb, .. }
        | I::Lvrx { ra, rb, .. }
        | I::Lvrxl { ra, rb, .. } => {
            let base = if ra == 0 { 0 } else { gpr_bit(ra) };
            base | gpr_bit(rb)
        }
        _ => 0,
    }
}

/// The CR fields that `decoded` sets from the FPRs, the VRs or FPSCR.
///
/// These words set CR fields:
/// - `fcmpu` and `fcmpo` set field BF;
/// - `mcrfs` sets field BF from a field of FPSCR;
/// - a floating-point word with Rc = 1 sets CR1 from FPSCR;
/// - a vector compare with Rc = 1 sets CR6.
///
/// [PPC-Book1 p:119 s:4.6.7] `fcmpu` and `fcmpo` place the result of the compare into CR field BF.
/// [PPC-Book1 p:120 s:4.6.8] `mcrfs` copies FPSCR field BFA into CR field BF.
/// [PPC-Book1 p:18 s:2.3.1] A floating-point instruction can set CR field 1 as an implicit result.
/// [AltiVec-PEM p:2-7 s:2.4] A vector compare with Rc = 1 updates CR6.
pub(super) fn cr_writes(decoded: &PpuInstruction) -> Cells {
    use crate::instruction::ops::Fp63Op;
    use PpuInstruction as I;
    let record = |rc: bool| {
        if rc {
            Cells::cr_field(1)
        } else {
            Cells::NONE
        }
    };
    match *decoded {
        I::Fp63 { op, frt, rc, .. } => {
            let field = match op {
                Fp63Op::Fcmpu | Fp63Op::Fcmpo | Fp63Op::Mcrfs => Cells::cr_field(frt >> 2),
                _ => Cells::NONE,
            };
            field.union(record(rc))
        }
        I::Fp59 { rc, .. } => record(rc),
        I::Vx { op, rc: true, .. } if op.is_vxr_compare() => Cells::cr_field(6),
        _ => Cells::NONE,
    }
}
