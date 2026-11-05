//! Super-pairing pass: fuse adjacent decoded instructions into a
//! single super-instruction variant when an idiom matches. Called by
//! the shadow's `super_pair` walk, which is left-to-right and
//! first-pair-wins: `lwz; cmpwi; bc` fuses to `LwzCmpwi + bc`, never
//! `lwz + CmpwiBc`.
//!
//! [ErtlGregg2003 p:20 s:6.3] super-instruction fusion eliminates one dispatch per pair.
//! [ErtlGregg2003 p:19 s:5.2.7] empirical effectiveness of super-instructions.

use crate::instruction::{BranchOffset, FusedPair, PpuInstruction};

/// Fuse two adjacent instructions, or `None` if no rule applies.
pub(super) fn make_super_pair(a: PpuInstruction, b: PpuInstruction) -> Option<PpuInstruction> {
    fuse(a, b).map(PpuInstruction::Fused)
}

/// The fused pair of `a` then `b`, or `None` if no rule applies.
fn fuse(a: PpuInstruction, b: PpuInstruction) -> Option<FusedPair> {
    match (a, decoded_form(b)) {
        // lwz rT, off(rA) + cmpwi crF, rT, imm
        (
            PpuInstruction::Lwz { rt, ra, imm },
            PpuInstruction::Cmpwi {
                bf,
                ra: cmp_ra,
                imm: cmp_imm,
            },
        ) if rt == cmp_ra => Some(FusedPair::LwzCmpwi {
            rt,
            ra_load: ra,
            offset: imm,
            bf,
            cmp_imm,
        }),
        // li rT, imm + stw rT, off(rA)
        (
            PpuInstruction::Li { rt, imm },
            PpuInstruction::Stw {
                rs,
                ra,
                imm: st_off,
            },
        ) if rt == rs => Some(FusedPair::LiStw {
            rt,
            imm,
            ra_store: ra,
            store_offset: st_off,
        }),
        // mflr rT + stw rT, off(rA)
        (PpuInstruction::Mflr { rt }, PpuInstruction::Stw { rs, ra, imm }) if rt == rs => {
            Some(FusedPair::MflrStw {
                rt,
                ra_store: ra,
                store_offset: imm,
            })
        }
        // lwz rT, off(rA) + mtlr rT
        (PpuInstruction::Lwz { rt, ra, imm }, PpuInstruction::Mtlr { rs }) if rt == rs => {
            Some(FusedPair::LwzMtlr {
                rt,
                ra_load: ra,
                offset: imm,
            })
        }
        // mflr rT + std rT, off(rA) (PPC64 prologue)
        (PpuInstruction::Mflr { rt }, PpuInstruction::Std { rs, ra, imm }) if rt == rs => {
            Some(FusedPair::MflrStd {
                rt,
                ra_store: ra,
                store_offset: imm,
            })
        }
        // ld rT, off(rA) + mtlr rT (PPC64 epilogue)
        (PpuInstruction::Ld { rt, ra, imm }, PpuInstruction::Mtlr { rs }) if rt == rs => {
            Some(FusedPair::LdMtlr {
                rt,
                ra_load: ra,
                offset: imm,
            })
        }
        // std rS1, off1(rA) + std rS2, off2(rA) where off2 = off1 + 8.
        // checked_add (not wrapping_add) rejects a near-i16::MAX off1
        // whose +8 would wrap to a negative offset and falsely match a
        // distant store -- the two stores would not actually be
        // accessing adjacent doublewords.
        (
            PpuInstruction::Std {
                rs: rs1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Std {
                rs: rs2,
                ra: ra2,
                imm: off2,
            },
        ) if ra1 == ra2 && off1.checked_add(8) == Some(off2) => Some(FusedPair::StdStd {
            rs1,
            rs2,
            ra: ra1,
            offset1: off1,
        }),
        // ld rT1, off1(rA) + ld rT2, off1+8(rA). A first load that writes a
        // nonzero RA moves the second EA, so that shape stays split.
        (
            PpuInstruction::Ld {
                rt: rt1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Ld {
                rt: rt2,
                ra: ra2,
                imm: off2,
            },
        ) if ra1 == ra2 && (rt1 != ra1 || ra1 == 0) && off1.checked_add(8) == Some(off2) => {
            Some(FusedPair::LdLd {
                rt1,
                rt2,
                ra: ra1,
                offset1: off1,
            })
        }
        // lbzu rT, off(rA) + cmpwi crF, rC, 0. An invalid lbzu (RA = 0 or
        // RA = RT) does not fuse, so it refuses on its own path.
        (PpuInstruction::Lbzu { rt, ra, imm }, PpuInstruction::CmpwZero { bf, ra: cmp_ra })
            if ra != 0 && ra != rt =>
        {
            Some(FusedPair::LbzuCmpwi {
                rt,
                ra,
                offset: imm,
                bf,
                cmp_ra,
            })
        }
        // ldu rT1, off1(rA1) + ldu rT2, off2(rA2). An invalid half (RA = 0 or
        // RA = RT) does not fuse. A second base equal to RT1 does not fuse,
        // because EA2 then depends on the first load's value.
        (
            PpuInstruction::Ldu {
                rt: rt1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Ldu {
                rt: rt2,
                ra: ra2,
                imm: off2,
            },
        ) if ra1 != 0 && ra1 != rt1 && ra2 != 0 && ra2 != rt2 && ra2 != rt1 => {
            Some(FusedPair::LduLdu {
                rt1,
                ra1,
                off1,
                rt2,
                ra2,
                off2,
            })
        }
        // cmpd crF, rA, rB + nor rA2, rS, rB2 without Rc.
        (
            PpuInstruction::Cmpd { bf, ra, rb },
            PpuInstruction::Nor {
                ra: nor_ra,
                rs: nor_rs,
                rb: nor_rb,
                rc: false,
            },
        ) => Some(FusedPair::CmpdNor {
            bf,
            ra,
            rb,
            nor_ra,
            nor_rs,
            nor_rb,
        }),
        // extsw rA, rS + blr. The shadow quickens only a branch-always bclr
        // to Blr.
        (PpuInstruction::Extsw { ra, rs, rc: false }, PpuInstruction::Blr { bo, bi }) => {
            Some(FusedPair::ExtswBlr { ra, rs, bo, bi })
        }
        // or rA, rS, rB (or mr rA, rS) + add rT, rA2, rB2. Neither half sets
        // Rc, and the add has OE = 0.
        // [PPC-Book1 p:67 s:3.3.11 OR] or with RS = RA = RB is a priority hint;
        // like the quickener, the pass keeps that form as a raw `or`.
        (
            PpuInstruction::Or {
                ra,
                rs,
                rb,
                rc: false,
            },
            PpuInstruction::Add {
                rt,
                ra: add_ra,
                rb: add_rb,
                oe: false,
                rc: false,
            },
        ) if !(ra == rs && rs == rb) => Some(FusedPair::OrAdd {
            ra,
            rs,
            rb,
            rt,
            add_ra,
            add_rb,
        }),
        (
            PpuInstruction::Mr { ra, rs },
            PpuInstruction::Add {
                rt,
                ra: add_ra,
                rb: add_rb,
                oe: false,
                rc: false,
            },
        ) => Some(FusedPair::OrAdd {
            ra,
            rs,
            rb: rs,
            rt,
            add_ra,
            add_rb,
        }),
        // lis rT, hi + ori rT, rT, lo. The shadow quickens `ori 0, 0, 0` to
        // Nop first, so that shape never reaches this arm.
        (PpuInstruction::Lis { rt, imm: hi }, PpuInstruction::Ori { ra, rs, imm: lo })
            if ra == rt && rs == rt && !(rt == 0 && lo == 0) =>
        {
            Some(FusedPair::LisOri { rt, hi, lo })
        }
        // lwz rT, off(rA) + CmpwZero crF, rT (quickened cmpwi-zero)
        (PpuInstruction::Lwz { rt, ra, imm }, PpuInstruction::CmpwZero { bf, ra: cmp_ra })
            if rt == cmp_ra =>
        {
            Some(FusedPair::LwzCmpwi {
                rt,
                ra_load: ra,
                offset: imm,
                bf,
                cmp_imm: 0,
            })
        }
        // cmpwi crF, rA, imm + bc BO, BI, offset (non-linking)
        (
            PpuInstruction::Cmpwi { bf, ra, imm },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset: offset,
        }),
        // CmpwZero + bc (quickened cmpwi-zero still fuses)
        (
            PpuInstruction::CmpwZero { bf, ra },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpwiBc {
            bf,
            ra,
            imm: 0,
            bo,
            bi,
            target_offset: offset,
        }),
        // cmpw crF, rA, rB + bc BO, BI, offset (non-linking)
        (
            PpuInstruction::Cmpw { bf, ra, rb },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset: offset,
        }),
        // cmpdi crF, rA, imm + bc (non-linking)
        (
            PpuInstruction::Cmpdi { bf, ra, imm },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpdiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset: offset,
        }),
        // CmpdZero + bc (quickened cmpdi-zero still fuses)
        (
            PpuInstruction::CmpdZero { bf, ra },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpdiBc {
            bf,
            ra,
            imm: 0,
            bo,
            bi,
            target_offset: offset,
        }),
        // cmpd crF, rA, rB + bc (non-linking)
        (
            PpuInstruction::Cmpd { bf, ra, rb },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmpdBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset: offset,
        }),
        // cmplw crF, rA, rB + bc (non-linking)
        (
            PpuInstruction::Cmplw { bf, ra, rb },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::CmplwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset: offset,
        }),
        // or rA, rS, rB (or mr rA, rS) without Rc + rlwinm in any form. An or
        // that is a priority hint stays raw, as it does before an add.
        (
            PpuInstruction::Or {
                ra,
                rs,
                rb,
                rc: false,
            },
            PpuInstruction::Rlwinm {
                ra: rlwinm_ra,
                rs: rlwinm_rs,
                sh,
                mb,
                me,
                rc,
            },
        ) if !(ra == rs && rs == rb) => Some(FusedPair::OrRlwinm {
            ra,
            rs,
            rb,
            rlwinm_ra,
            rlwinm_rs,
            sh,
            mb,
            me,
            rc,
        }),
        (
            PpuInstruction::Mr { ra, rs },
            PpuInstruction::Rlwinm {
                ra: rlwinm_ra,
                rs: rlwinm_rs,
                sh,
                mb,
                me,
                rc,
            },
        ) => Some(FusedPair::OrRlwinm {
            ra,
            rs,
            rb: rs,
            rlwinm_ra,
            rlwinm_rs,
            sh,
            mb,
            me,
            rc,
        }),
        // lwz rT1, off1(rA1) + lwz rT2, off2(rA2). A second base equal to a
        // nonzero RT1 does not fuse, because EA2 then depends on the first
        // load's value.
        (
            PpuInstruction::Lwz {
                rt: rt1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Lwz {
                rt: rt2,
                ra: ra2,
                imm: off2,
            },
        ) if ra2 == 0 || ra2 != rt1 => Some(FusedPair::LwzLwz {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        }),
        // ld rT, off(r1) + rlwinm in any form, recording or not.
        (
            PpuInstruction::Ld { rt, ra: 1, imm },
            PpuInstruction::Rlwinm {
                ra,
                rs,
                sh,
                mb,
                me,
                rc,
            },
        ) => Some(FusedPair::LdRlwinm {
            rt,
            offset: imm,
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        }),
        // addi rT, rA, imm + bc (non-linking)
        (
            PpuInstruction::Addi { rt, ra, imm },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::AddiBc {
            rt,
            ra,
            imm,
            bo,
            bi,
            target_offset: offset,
        }),
        // li rT, imm + bc: the shadow quickens addi rT, 0, imm to Li before
        // pairing, so that shape fuses as an addi with RA = 0.
        (
            PpuInstruction::Li { rt, imm },
            PpuInstruction::Bc {
                bo,
                bi,
                offset,
                aa: false,
                link: false,
            },
        ) => Some(FusedPair::AddiBc {
            rt,
            ra: 0,
            imm,
            bo,
            bi,
            target_offset: offset,
        }),
        // std rS, off(r1) + lwz rT, off2(rA). The std writes no register, so
        // the lwz EA does not depend on it.
        (
            PpuInstruction::Std {
                rs,
                ra: 1,
                imm: store_offset,
            },
            PpuInstruction::Lwz {
                rt,
                ra,
                imm: offset,
            },
        ) => Some(FusedPair::StdLwz {
            rs,
            store_offset,
            rt,
            ra,
            offset,
        }),
        // ld rT1, off1(rA1) + ld rT2, off2(rA2) that the LdLd arm did not
        // take. A second base equal to a nonzero RT1 does not fuse, because
        // EA2 then depends on the first load's value.
        (
            PpuInstruction::Ld {
                rt: rt1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Ld {
                rt: rt2,
                ra: ra2,
                imm: off2,
            },
        ) if ra2 == 0 || ra2 != rt1 => Some(FusedPair::LdLdAny {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        }),
        // or rA, rS, rB in any form (or mr rA, rS) + b with AA = 0 and LK = 0.
        // The or neither faults nor branches, so it runs as it does when the
        // pair stays split.
        (
            PpuInstruction::Or { ra, rs, rb, rc },
            PpuInstruction::B {
                offset,
                aa: false,
                link: false,
            },
        ) => BranchOffset::new(offset).map(|offset| FusedPair::OrB {
            ra,
            rs,
            rb,
            rc,
            offset,
        }),
        (
            PpuInstruction::Mr { ra, rs },
            PpuInstruction::B {
                offset,
                aa: false,
                link: false,
            },
        ) => BranchOffset::new(offset).map(|offset| FusedPair::OrB {
            ra,
            rs,
            rb: rs,
            rc: false,
            offset,
        }),
        // lbz rT1, off1(rA1) + lbz rT2, off2(rA2). A second base equal to a
        // nonzero RT1 does not fuse, because EA2 then depends on the first
        // load's value.
        (
            PpuInstruction::Lbz {
                rt: rt1,
                ra: ra1,
                imm: off1,
            },
            PpuInstruction::Lbz {
                rt: rt2,
                ra: ra2,
                imm: off2,
            },
        ) if ra2 == 0 || ra2 != rt1 => Some(FusedPair::LbzLbz {
            rt1,
            ra1,
            off1,
            rt2,
            ra2,
            off2,
        }),
        // mtlr rS + addi rT, rA, imm. The shadow quickens addi rT, 0, imm to
        // Li, so that shape fuses as an addi with RA = 0.
        (PpuInstruction::Mtlr { rs }, PpuInstruction::Addi { rt, ra, imm }) => {
            Some(FusedPair::MtlrAddi { rs, rt, ra, imm })
        }
        (PpuInstruction::Mtlr { rs }, PpuInstruction::Li { rt, imm }) => {
            Some(FusedPair::MtlrAddi { rs, rt, ra: 0, imm })
        }
        // rldicl + rldicl, either half in any form, recording or not.
        (
            first,
            PpuInstruction::Rldicl {
                ra: ra2,
                rs: rs2,
                sh: sh2,
                mb: mb2,
                rc: rc2,
            },
        ) => match decoded_form(first) {
            PpuInstruction::Rldicl {
                ra: ra1,
                rs: rs1,
                sh: sh1,
                mb: mb1,
                rc: rc1,
            } => Some(FusedPair::RldiclRldicl {
                ra1,
                rs1,
                sh1,
                mb1,
                ra2,
                rs2,
                sh2,
                mb2,
                rc: u8::from(rc1) | (u8::from(rc2) << 1),
            }),
            _ => None,
        },
        // ori rA, rS, imm (or the nop the shadow quickens ori 0,0,0 to) + and
        // in any form. The ori neither faults nor branches.
        (
            PpuInstruction::Ori { ra, rs, imm },
            PpuInstruction::And {
                ra: and_ra,
                rs: and_rs,
                rb: and_rb,
                rc,
            },
        ) => Some(FusedPair::OriAnd {
            ra,
            rs,
            imm,
            and_ra,
            and_rs,
            and_rb,
            rc,
        }),
        (
            PpuInstruction::Nop,
            PpuInstruction::And {
                ra: and_ra,
                rs: and_rs,
                rb: and_rb,
                rc,
            },
        ) => Some(FusedPair::OriAnd {
            ra: 0,
            rs: 0,
            imm: 0,
            and_ra,
            and_rs,
            and_rb,
            rc,
        }),
        _ => fuse_multi_form(a, decoded_form(b)),
    }
}

/// The fused pair of `a` then `b` for the pairs whose halves take more than
/// one form, or `None` if no rule applies. `b` is in its decoded form.
///
/// An `or` in either half fuses raw or as the quickened `mr`. Every first
/// half here other than a load neither faults nor branches.
fn fuse_multi_form(a: PpuInstruction, b: PpuInstruction) -> Option<FusedPair> {
    if let Some(first) = or_form(a) {
        return fuse_after_or(first, b);
    }
    match (decoded_form(a), b) {
        // rldicr rA, rS, sh, me (or sldi) + oris rA2, rS2, imm
        (
            PpuInstruction::Rldicr { ra, rs, sh, me, rc },
            PpuInstruction::Oris {
                ra: oris_ra,
                rs: oris_rs,
                imm,
            },
        ) => Some(FusedPair::RldicrOris {
            ra,
            rs,
            sh,
            me,
            rc,
            oris_ra,
            oris_rs,
            imm,
        }),
        // ld rT, off(r1) + addi rT2, rA, imm. The shadow quickens addi rT, 0,
        // imm to Li, so that shape fuses as an addi with RA = 0.
        (
            PpuInstruction::Ld { rt, ra: 1, imm },
            PpuInstruction::Addi {
                rt: addi_rt,
                ra: addi_ra,
                imm: addi_imm,
            },
        ) => Some(FusedPair::LdAddi {
            rt,
            offset: imm,
            addi_rt,
            addi_ra,
            imm: addi_imm,
        }),
        (
            PpuInstruction::Ld { rt, ra: 1, imm },
            PpuInstruction::Li {
                rt: addi_rt,
                imm: addi_imm,
            },
        ) => Some(FusedPair::LdAddi {
            rt,
            offset: imm,
            addi_rt,
            addi_ra: 0,
            imm: addi_imm,
        }),
        // lwz rT, off(rA) + or. The lwz writes nothing when it faults.
        (PpuInstruction::Lwz { rt, ra, imm }, second) => {
            or_form(second).map(|(or_ra, or_rs, or_rb, rc)| FusedPair::LwzOr {
                rt,
                ra,
                offset: imm,
                or_ra,
                or_rs,
                or_rb,
                rc,
            })
        }
        _ => None,
    }
}

/// The fused pair of an `or` with fields `(ra, rs, rb, rc)` then `b`, or
/// `None` if no rule applies.
fn fuse_after_or((ra, rs, rb, rc): (u8, u8, u8, bool), b: PpuInstruction) -> Option<FusedPair> {
    match b {
        // bc with AA = 0 and LK = 0, in any quickened form.
        PpuInstruction::Bc {
            bo,
            bi,
            offset,
            aa: false,
            link: false,
        } => Some(FusedPair::OrBc {
            ra,
            rs,
            rb,
            rc,
            bo,
            bi,
            target_offset: offset,
        }),
        PpuInstruction::Lwz {
            rt,
            ra: lwz_ra,
            imm,
        } => Some(FusedPair::OrLwz {
            ra,
            rs,
            rb,
            rc,
            rt,
            lwz_ra,
            offset: imm,
        }),
        // [PPC-Book1 p:20 s:2.4.1] BO 1z1zz branches always and leaves CTR alone.
        PpuInstruction::Bcctr { bo, bi, link: true } if bo & 0b10100 == 0b10100 => {
            Some(FusedPair::OrBctrl {
                ra,
                rs,
                rb,
                rc,
                bo,
                bi,
            })
        }
        second => or_form(second).map(|(ra2, rs2, rb2, rc2)| FusedPair::OrOr {
            ra1: ra,
            rs1: rs,
            rb1: rb,
            rc1: rc,
            ra2,
            rs2,
            rb2,
            rc2,
        }),
    }
}

/// The fields `(ra, rs, rb, rc)` of `insn` as an `or`, raw or as the
/// quickened `mr`, or `None` for any other instruction.
fn or_form(insn: PpuInstruction) -> Option<(u8, u8, u8, bool)> {
    match insn {
        PpuInstruction::Or { ra, rs, rb, rc } => Some((ra, rs, rb, rc)),
        PpuInstruction::Mr { ra, rs } => Some((ra, rs, rs, false)),
        _ => None,
    }
}

/// The original form of a quickened branch, rotate or doubleword rotate, or
/// any other `insn` unchanged.
///
/// A quickened second half then pairs as its original instruction does.
fn decoded_form(insn: PpuInstruction) -> PpuInstruction {
    let rlwinm = |ra, rs, sh, mb, me| PpuInstruction::Rlwinm {
        ra,
        rs,
        sh,
        mb,
        me,
        rc: false,
    };
    match insn {
        PpuInstruction::Bdnz { bo, bi, offset }
        | PpuInstruction::BcTrue { bo, bi, offset }
        | PpuInstruction::BcFalse { bo, bi, offset } => PpuInstruction::Bc {
            bo,
            bi,
            offset,
            aa: false,
            link: false,
        },
        // [PPC-Book1 p:160 s:B.7.2 Table 10] slwi n is rlwinm n,0,31-n; srwi n
        // is rlwinm 32-n,n,31; clrlwi n is rlwinm 0,n,31.
        PpuInstruction::Slwi { ra, rs, n } => rlwinm(ra, rs, n, 0, 31 - n),
        PpuInstruction::Srwi { ra, rs, n } => rlwinm(ra, rs, 32 - n, n, 31),
        PpuInstruction::Clrlwi { ra, rs, n } => rlwinm(ra, rs, 0, n, 31),
        PpuInstruction::RlwinmMask {
            ra, rs, sh, mb, me, ..
        } => rlwinm(ra, rs, sh, mb, me),
        // [PPC-Book1 p:160 s:B.7.1 Table 9] clrldi n is rldicl 0,n; srdi n is
        // rldicl 64-n,n.
        PpuInstruction::Clrldi { ra, rs, n } => PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 0,
            mb: n,
            rc: false,
        },
        // [PPC-Book1 p:159 s:B.7.1 Table 9] sldi n is rldicr n,63-n.
        PpuInstruction::Sldi { ra, rs, n } => PpuInstruction::Rldicr {
            ra,
            rs,
            sh: n,
            me: 63 - n,
            rc: false,
        },
        PpuInstruction::Srdi { ra, rs, n } => PpuInstruction::Rldicl {
            ra,
            rs,
            sh: 64 - n,
            mb: n,
            rc: false,
        },
        other => other,
    }
}

#[cfg(test)]
#[path = "tests/superpair_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/superpair_proptests.rs"]
mod proptests;

#[cfg(test)]
#[path = "tests/superpair_compare_bc_tests.rs"]
mod compare_bc_tests;

#[cfg(test)]
#[path = "tests/superpair_lis_tests.rs"]
mod lis_tests;

#[cfg(test)]
#[path = "tests/superpair_ld_tests.rs"]
mod ld_tests;

#[cfg(test)]
#[path = "tests/superpair_or_add_tests.rs"]
mod or_add_tests;

#[cfg(test)]
#[path = "tests/superpair_lbzu_tests.rs"]
mod lbzu_tests;

#[cfg(test)]
#[path = "tests/superpair_extsw_tests.rs"]
mod extsw_tests;

#[cfg(test)]
#[path = "tests/superpair_ldu_tests.rs"]
mod ldu_tests;

#[cfg(test)]
#[path = "tests/superpair_cmpd_nor_tests.rs"]
mod cmpd_nor_tests;

#[cfg(test)]
#[path = "tests/superpair_addi_bc_tests.rs"]
mod addi_bc_tests;

#[cfg(test)]
#[path = "tests/superpair_ld_rlwinm_tests.rs"]
mod ld_rlwinm_tests;

#[cfg(test)]
#[path = "tests/superpair_lwz_pair_tests.rs"]
mod lwz_pair_tests;

#[cfg(test)]
#[path = "tests/superpair_or_rlwinm_tests.rs"]
mod or_rlwinm_tests;

#[cfg(test)]
#[path = "tests/superpair_std_lwz_tests.rs"]
mod std_lwz_tests;

#[cfg(test)]
#[path = "tests/superpair_ld_any_tests.rs"]
mod ld_any_tests;

#[cfg(test)]
#[path = "tests/superpair_or_b_tests.rs"]
mod or_b_tests;

#[cfg(test)]
#[path = "tests/superpair_lbz_pair_tests.rs"]
mod lbz_pair_tests;

#[cfg(test)]
#[path = "tests/superpair_mtlr_addi_tests.rs"]
mod mtlr_addi_tests;

#[cfg(test)]
#[path = "tests/superpair_rldicl_pair_tests.rs"]
mod rldicl_pair_tests;

#[cfg(test)]
#[path = "tests/superpair_ori_and_tests.rs"]
mod ori_and_tests;

#[cfg(test)]
#[path = "tests/superpair_multi_form_tests.rs"]
mod multi_form_tests;
