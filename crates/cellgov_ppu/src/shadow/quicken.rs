//! Quickening pass: rewrite a single decoded instruction into a
//! specialized variant when an idiom is recognized. Pure function;
//! the shadow's `quicken` method walks slots and applies this in
//! place.
//!
//! [Brunthaler2010 p:2 s:2] in-place specialization of decoded instructions.
//! [Brunthaler2010 p:3 s:3.2] argument-pattern unfoldings.

use crate::instruction::PpuInstruction;

/// Rewrite one decoded instruction into a specialized variant, or
/// `None` if no rule applies.
pub(super) fn quicken_insn(insn: PpuInstruction) -> Option<PpuInstruction> {
    match insn {
        // addi rT, 0, imm => Li
        // [PPC-Book1 p:162 s:B.9 Load Immediate] li Rx,value == addi Rx,0,value.
        PpuInstruction::Addi { rt, ra: 0, imm } => Some(PpuInstruction::Li { rt, imm }),
        // or rA, rS, rS => Mr (only when Rc=0 and ra != rs).
        // [PPC-Book1 p:163 s:B.9 Move Register] mr Rx,Ry == or Rx,Ry,Ry.
        // [CBE-Handbook p:316 s:Table 10-7] or N,N,N where N in {1,2,3,28..31}
        // are PPE thread-priority / dispatch-stall nops, not register copies.
        //
        // The ra==rs case is the CBE PPE thread-priority and dispatch-stall
        // hint family: `or 1,1,1` / `or 2,2,2` / `or 3,3,3` are cctpl /
        // cctpm / cctph; `or 28..31, ...` are db8cyc / db10cyc / db12cyc /
        // db16cyc. Quickening these to `Mr { ra: N, rs: N }` would erase the
        // architectural side effect; leaving them as raw `Or` keeps the
        // information visible to any later analysis pass that wants to
        // recognize them. Compiler-emitted register copies always have
        // ra != rs, so the guard does not regress the common case.
        PpuInstruction::Or {
            ra,
            rs,
            rb,
            rc: false,
        } if rs == rb && ra != rs => Some(PpuInstruction::Mr { ra, rs }),
        // rlwinm rA, rS, sh, 0, 31-sh => Slwi (only when sh != 0).
        // [PPC-Book1 p:161 s:B.7.2 Table 9] slwi ra,rs,n (n<32) == rlwinm ra,rs,n,0,31-n.
        //
        // The sh=0 case is `rlwinm rA, rS, 0, 0, 31` -- a 32-bit zero-extend,
        // not a left shift. It is canonically Clrlwi { n: 0 }, which the
        // arm below handles. Mirrors the sh != 0 guard on Srwi / Sldi / Srdi.
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if sh != 0 && mb == 0 && me == 31 - sh => Some(PpuInstruction::Slwi { ra, rs, n: sh }),
        // rlwinm rA, rS, 32-n, n, 31 => Srwi
        // [PPC-Book1 p:161 s:B.7.2 Table 9] srwi ra,rs,n (n<32) == rlwinm ra,rs,32-n,n,31.
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if me == 31 && sh != 0 && mb == (32 - sh) => Some(PpuInstruction::Srwi { ra, rs, n: mb }),
        // rlwinm rA, rS, 0, n, 31 => Clrlwi
        // [PPC-Book1 p:161 s:B.7.2 Table 9] clrlwi ra,rs,n (n<32) == rlwinm ra,rs,0,n,31.
        PpuInstruction::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc: false,
        } if sh == 0 && me == 31 => Some(PpuInstruction::Clrlwi { ra, rs, n: mb }),
        // ori rA, rA, 0 => Nop
        // [PPC-Book1 p:162 s:B.9 No-op] preferred no-op form is ori 0,0,0.
        PpuInstruction::Ori { ra, rs, imm: 0 } if ra == rs => Some(PpuInstruction::Nop),
        // cmpwi crF, rA, 0 => CmpwZero
        PpuInstruction::Cmpwi { bf, ra, imm: 0 } => Some(PpuInstruction::CmpwZero { bf, ra }),
        // rldicl rA, rS, 0, n => Clrldi
        // [PPC-Book1 p:160 s:B.7.1 Table 10] clrldi ra,rs,n (n<64) == rldicl ra,rs,0,n.
        PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 0,
            mb,
            rc: false,
        } => Some(PpuInstruction::Clrldi { ra, rs, n: mb }),
        // rldicr rA, rS, n, 63-n => Sldi
        // [PPC-Book1 p:160 s:B.7.1 Table 10] sldi ra,rs,n (n<64) == rldicr ra,rs,n,63-n.
        PpuInstruction::Rldicr {
            ra,
            rs,
            sh,
            me,
            rc: false,
        } if sh != 0 && me == 63 - sh => Some(PpuInstruction::Sldi { ra, rs, n: sh }),
        // rldicl rA, rS, 64-n, n => Srdi
        // [PPC-Book1 p:160 s:B.7.1 Table 10] srdi ra,rs,n (n<64) == rldicl ra,rs,64-n,n.
        PpuInstruction::Rldicl {
            ra,
            rs,
            sh,
            mb,
            rc: false,
        } if sh != 0 && mb == 64 - sh => Some(PpuInstruction::Srdi { ra, rs, n: mb }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/quicken_tests.rs"]
mod tests;
