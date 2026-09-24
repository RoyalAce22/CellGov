//! The extended-mnemonic simplifier consulted before canonical rendering.

use core::fmt;

use crate::funcmap::FunctionMap;
use crate::instruction::PpuInstruction;

use super::mnemonic::{mn_rc, op, op0, shown_target, CrBit, Mn};
use super::text::Target;

/// Operand shape of a simplified rendering.
enum SimpleOps {
    /// Mnemonic only (`nop`, `blr`).
    None,
    /// `rN, imm` signed decimal (`li`).
    RegImmDec { r: u8, imm: i16 },
    /// `rN, 0ximm` (`lis`).
    RegImmHex { r: u8, imm: u16 },
    /// `rA, rS` (`mr`, `not`).
    TwoRegs { a: u8, s: u8 },
    /// `rA, rS, n` (`slwi` family).
    RegRegN { a: u8, s: u8, n: u8 },
    /// `[crF, ] target` (`blt`, `bne cr3, ...`).
    CrTarget { crf: u8, target: u64 },
    /// `[crF]` (`bltlr`, `bnectr cr7`).
    CrOnly { crf: u8 },
    /// `crbit, target` (`bdnzt eq, ...`).
    CrBitTarget { bi: u8, target: u64 },
    /// `crbit` (`bdnztlr so`).
    CrBitOnly { bi: u8 },
    /// `target` (`b`, `bdnz`).
    Target { target: u64 },
}

/// A simplified (extended-mnemonic) rendering of one instruction.
pub(super) struct Simplified {
    mn: Mn,
    ops: SimpleOps,
}

impl Simplified {
    pub(super) fn render(
        &self,
        symbols: Option<&FunctionMap>,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        let mn = self.mn.as_str();
        let sym = |target| Target { target, symbols };
        match self.ops {
            SimpleOps::None => op0(f, mn),
            SimpleOps::RegImmDec { r, imm } => op(f, mn, format_args!("r{r}, {imm}")),
            SimpleOps::RegImmHex { r, imm } => op(f, mn, format_args!("r{r}, 0x{imm:x}")),
            SimpleOps::TwoRegs { a, s } => op(f, mn, format_args!("r{a}, r{s}")),
            SimpleOps::RegRegN { a, s, n } => op(f, mn, format_args!("r{a}, r{s}, {n}")),
            SimpleOps::CrTarget { crf: 0, target } => op(f, mn, format_args!("{}", sym(target))),
            SimpleOps::CrTarget { crf, target } => {
                op(f, mn, format_args!("cr{crf}, {}", sym(target)))
            }
            SimpleOps::CrOnly { crf: 0 } => op0(f, mn),
            SimpleOps::CrOnly { crf } => op(f, mn, format_args!("cr{crf}")),
            SimpleOps::CrBitTarget { bi, target } => {
                op(f, mn, format_args!("{}, {}", CrBit(bi), sym(target)))
            }
            SimpleOps::CrBitOnly { bi } => op(f, mn, format_args!("{}", CrBit(bi))),
            SimpleOps::Target { target } => op(f, mn, format_args!("{}", sym(target))),
        }
    }
}

/// What a Branch Conditional's BO field tests, hint bits masked out.
// [PPC-Book1 p:20 s:2.4.1] Figure 21 BO field encodings: 0000z bdnzf,
// 0001z bdzf, 001at cond-false, 0100z bdnzt, 0101z bdzt, 011at
// cond-true, 1a00t bdnz, 1a01t bdz, 1z1zz branch-always.
enum BranchKind {
    /// Branch always.
    Always,
    /// Decrement CTR, branch on CTR nonzero (`nz`) / zero.
    Ctr { nz: bool },
    /// Branch on CR bit true / false.
    Cond { wanted: bool },
    /// Decrement CTR and test a CR bit.
    CtrCond { nz: bool, wanted: bool },
}

/// Classify BO, requiring `bi == 0` for the kinds whose BI field is
/// architecturally ignored (never guess on nonstandard encodings).
fn branch_kind(bo: u8, bi: u8) -> Option<BranchKind> {
    if bo & 0b10100 == 0b10100 {
        return (bi == 0).then_some(BranchKind::Always);
    }
    if bo & 0b10110 == 0b10000 {
        return (bi == 0).then_some(BranchKind::Ctr { nz: true });
    }
    if bo & 0b10110 == 0b10010 {
        return (bi == 0).then_some(BranchKind::Ctr { nz: false });
    }
    if bo & 0b11100 == 0b01100 {
        return Some(BranchKind::Cond { wanted: true });
    }
    if bo & 0b11100 == 0b00100 {
        return Some(BranchKind::Cond { wanted: false });
    }
    match bo & 0b11110 {
        0b01000 => Some(BranchKind::CtrCond {
            nz: true,
            wanted: true,
        }),
        0b00000 => Some(BranchKind::CtrCond {
            nz: true,
            wanted: false,
        }),
        0b01010 => Some(BranchKind::CtrCond {
            nz: false,
            wanted: true,
        }),
        0b00010 => Some(BranchKind::CtrCond {
            nz: false,
            wanted: false,
        }),
        _ => None,
    }
}

/// Condition code for a tested CR bit: `blt`-style when the branch
/// fires on the bit being 1, `bge`-style on 0.
// [PPC-Book1 p:153 s:B.2.3] standard condition codes lt/gt/eq/so and
// their negations ge/le/ne/ns.
fn cond_code(bi: u8, wanted: bool) -> &'static str {
    if wanted {
        ["lt", "gt", "eq", "so"][(bi & 3) as usize]
    } else {
        ["ge", "le", "ne", "ns"][(bi & 3) as usize]
    }
}

/// CTR-decrement stem: `bdnz` / `bdz`, optionally with the `t`/`f`
/// CR-bit test suffix.
// [PPC-Book1 p:152 s:B.2.2] Table 3 simple branch mnemonics.
fn ctr_stem(nz: bool, cond: Option<bool>) -> Mn {
    let mut m = Mn::new(if nz { "bdnz" } else { "bdz" });
    match cond {
        Some(true) => m.push("t"),
        Some(false) => m.push("f"),
        None => {}
    }
    m
}

/// Extended-mnemonic table, consulted before canonical rendering.
/// Each arm's gate is exact; anything outside the table returns
/// `None` and renders canonically. Quickened variants (`Li`, `Mr`,
/// ...) never reach this: they already carry the extended form.
pub(super) fn simplify(insn: &PpuInstruction, addr: u64) -> Option<Simplified> {
    use PpuInstruction as I;
    match *insn {
        // [PPC-Book1 p:162 s:B.9] nop is the preferred form ori 0,0,0.
        I::Ori {
            ra: 0,
            rs: 0,
            imm: 0,
        } => Some(Simplified {
            mn: Mn::new("nop"),
            ops: SimpleOps::None,
        }),
        // [PPC-Book1 p:162 s:B.9] li rT,value is addi rT,0,value.
        I::Addi { rt, ra: 0, imm } => Some(Simplified {
            mn: Mn::new("li"),
            ops: SimpleOps::RegImmDec { r: rt, imm },
        }),
        // [PPC-Book1 p:162 s:B.9] lis rT,value is addis rT,0,value.
        I::Addis { rt, ra: 0, imm } => Some(Simplified {
            mn: Mn::new("lis"),
            ops: SimpleOps::RegImmHex {
                r: rt,
                imm: imm as u16,
            },
        }),
        // [PPC-Book1 p:163 s:B.9] mr rX,rY is or rX,rY,rY.
        I::Or { ra, rs, rb, rc } if rs == rb => Some(Simplified {
            mn: mn_rc("mr", rc),
            ops: SimpleOps::TwoRegs { a: ra, s: rs },
        }),
        // [PPC-Book1 p:163 s:B.9] not rX,rY is nor rX,rY,rY.
        I::Nor { ra, rs, rb, rc } if rs == rb => Some(Simplified {
            mn: mn_rc("not", rc),
            ops: SimpleOps::TwoRegs { a: ra, s: rs },
        }),
        // [PPC-Book1 p:160 s:B.7.2] clrlwi n is rlwinm 0,n,31.
        I::Rlwinm {
            ra,
            rs,
            sh: 0,
            mb,
            me: 31,
            rc,
        } => Some(Simplified {
            mn: mn_rc("clrlwi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: mb,
            },
        }),
        // [PPC-Book1 p:160 s:B.7.2] slwi n is rlwinm n,0,31-n.
        I::Rlwinm {
            ra,
            rs,
            sh,
            mb: 0,
            me,
            rc,
        } if me == 31 - sh => Some(Simplified {
            mn: mn_rc("slwi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: sh,
            },
        }),
        // [PPC-Book1 p:160 s:B.7.2] srwi n is rlwinm 32-n,n,31.
        I::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me: 31,
            rc,
        } if sh != 0 && mb == 32 - sh => Some(Simplified {
            mn: mn_rc("srwi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: 32 - sh,
            },
        }),
        // [PPC-Book1 p:159 s:B.7.1] sldi n is rldicr n,63-n.
        I::Rldicr { ra, rs, sh, me, rc } if me == 63 - sh => Some(Simplified {
            mn: mn_rc("sldi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: sh,
            },
        }),
        // [PPC-Book1 p:160 s:B.7.1] clrldi n is rldicl 0,n.
        I::Rldicl {
            ra,
            rs,
            sh: 0,
            mb,
            rc,
        } => Some(Simplified {
            mn: mn_rc("clrldi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: mb,
            },
        }),
        // [PPC-Book1 p:160 s:B.7.1] srdi n is rldicl 64-n,n.
        I::Rldicl { ra, rs, sh, mb, rc } if sh != 0 && mb == 64 - sh => Some(Simplified {
            mn: mn_rc("srdi", rc),
            ops: SimpleOps::RegRegN {
                a: ra,
                s: rs,
                n: 64 - sh,
            },
        }),

        // [PPC-Book1 p:152 s:B.2.2] Table 3: there is no extended
        // mnemonic for an unconditional or CTR-decrementing `bc`
        // beyond bdnz/bdz; branch-always via bc renders canonically.
        I::Bc {
            bo,
            bi,
            offset,
            aa,
            link,
        } => {
            let target = shown_target(addr, i32::from(offset), aa);
            match branch_kind(bo, bi)? {
                BranchKind::Always => None,
                BranchKind::Ctr { nz } => {
                    let mut mn = ctr_stem(nz, None);
                    if link {
                        mn.push("l");
                    }
                    if aa {
                        mn.push("a");
                    }
                    Some(Simplified {
                        mn,
                        ops: SimpleOps::Target { target },
                    })
                }
                // [PPC-Book1 p:153 s:B.2.3] b<cond> [crF,] target;
                // the crF operand is omitted for CR field 0.
                BranchKind::Cond { wanted } => {
                    let mut mn = Mn::new("b");
                    mn.push(cond_code(bi, wanted));
                    if link {
                        mn.push("l");
                    }
                    if aa {
                        mn.push("a");
                    }
                    Some(Simplified {
                        mn,
                        ops: SimpleOps::CrTarget {
                            crf: bi >> 2,
                            target,
                        },
                    })
                }
                // [PPC-Book1 p:152 s:B.2.2] bdnzt/bdnzf/bdzt/bdzf
                // take the tested CR bit as the first operand.
                BranchKind::CtrCond { nz, wanted } => {
                    let mut mn = ctr_stem(nz, Some(wanted));
                    if link {
                        mn.push("l");
                    }
                    if aa {
                        mn.push("a");
                    }
                    Some(Simplified {
                        mn,
                        ops: SimpleOps::CrBitTarget { bi, target },
                    })
                }
            }
        }
        // [PPC-Book1 p:152 s:B.2.2] Table 3, bclr column: blr, bdnzlr,
        // bdztlr, ... plus [p:153 s:B.2.3] b<cond>lr forms.
        I::Bclr { bo, bi, link } => {
            let mut mn = match branch_kind(bo, bi)? {
                BranchKind::Always => Mn::new("blr"),
                BranchKind::Ctr { nz } => {
                    let mut m = ctr_stem(nz, None);
                    m.push("lr");
                    m
                }
                BranchKind::Cond { wanted } => {
                    let mut m = Mn::new("b");
                    m.push(cond_code(bi, wanted));
                    m.push("lr");
                    m
                }
                BranchKind::CtrCond { nz, wanted } => {
                    let mut m = ctr_stem(nz, Some(wanted));
                    m.push("lr");
                    m
                }
            };
            if link {
                mn.push("l");
            }
            let ops = match branch_kind(bo, bi)? {
                BranchKind::Always | BranchKind::Ctr { .. } => SimpleOps::None,
                BranchKind::Cond { .. } => SimpleOps::CrOnly { crf: bi >> 2 },
                BranchKind::CtrCond { .. } => SimpleOps::CrBitOnly { bi },
            };
            Some(Simplified { mn, ops })
        }
        // [PPC-Book1 p:152 s:B.2.2] Table 3, bcctr column: bctr and
        // b<cond>ctr only; CTR-decrement forms have no bcctr
        // mnemonic (and are architecturally invalid).
        I::Bcctr { bo, bi, link } => {
            let (mut mn, ops) = match branch_kind(bo, bi)? {
                BranchKind::Always => (Mn::new("bctr"), SimpleOps::None),
                BranchKind::Cond { wanted } => {
                    let mut m = Mn::new("b");
                    m.push(cond_code(bi, wanted));
                    m.push("ctr");
                    (m, SimpleOps::CrOnly { crf: bi >> 2 })
                }
                BranchKind::Ctr { .. } | BranchKind::CtrCond { .. } => return None,
            };
            if link {
                mn.push("l");
            }
            Some(Simplified { mn, ops })
        }

        I::Lwz { .. }
        | I::Lbz { .. }
        | I::Lhz { .. }
        | I::Lha { .. }
        | I::Lhau { .. }
        | I::Lmw { .. }
        | I::Lwzu { .. }
        | I::Lbzu { .. }
        | I::Lhzu { .. }
        | I::Ldu { .. }
        | I::Ld { .. }
        | I::Lwa { .. }
        | I::Stw { .. }
        | I::Stwu { .. }
        | I::Stdu { .. }
        | I::Stb { .. }
        | I::Stbu { .. }
        | I::Stmw { .. }
        | I::Sth { .. }
        | I::Sthu { .. }
        | I::Std { .. }
        | I::Addi { .. }
        | I::Addis { .. }
        | I::Subfic { .. }
        | I::Mulli { .. }
        | I::Addic { .. }
        | I::AddicDot { .. }
        | I::Add { .. }
        | I::Or { .. }
        | I::Subf { .. }
        | I::Subfc { .. }
        | I::Subfe { .. }
        | I::Neg { .. }
        | I::Mullw { .. }
        | I::Mulhwu { .. }
        | I::Mulhw { .. }
        | I::Mulhdu { .. }
        | I::Mulhd { .. }
        | I::Adde { .. }
        | I::Addze { .. }
        | I::Subfze { .. }
        | I::Subfme { .. }
        | I::Addme { .. }
        | I::Mulld { .. }
        | I::Ldarx { .. }
        | I::Stdcx { .. }
        | I::Lwarx { .. }
        | I::Stwcx { .. }
        | I::Xori { .. }
        | I::Xoris { .. }
        | I::Divw { .. }
        | I::Divwu { .. }
        | I::Divd { .. }
        | I::Divdu { .. }
        | I::And { .. }
        | I::Andc { .. }
        | I::Nor { .. }
        | I::Xor { .. }
        | I::Eqv { .. }
        | I::Nand { .. }
        | I::AndiDot { .. }
        | I::AndisDot { .. }
        | I::Slw { .. }
        | I::Srw { .. }
        | I::Srawi { .. }
        | I::Sraw { .. }
        | I::Srad { .. }
        | I::Sradi { .. }
        | I::Sld { .. }
        | I::Srd { .. }
        | I::Cntlzw { .. }
        | I::Cntlzd { .. }
        | I::Popcntb { .. }
        | I::Tw { .. }
        | I::Td { .. }
        | I::Mcrxr { .. }
        | I::Orc { .. }
        | I::Extsh { .. }
        | I::Extsb { .. }
        | I::Extsw { .. }
        | I::Ori { .. }
        | I::Oris { .. }
        | I::Cmpwi { .. }
        | I::Cmplwi { .. }
        | I::Cmpdi { .. }
        | I::Cmpldi { .. }
        | I::Cmpw { .. }
        | I::Cmplw { .. }
        | I::Cmpd { .. }
        | I::Cmpld { .. }
        | I::B { .. }
        | I::Mcrf { .. }
        | I::Crand { .. }
        | I::Crandc { .. }
        | I::Cror { .. }
        | I::Crorc { .. }
        | I::Crxor { .. }
        | I::Crnand { .. }
        | I::Crnor { .. }
        | I::Creqv { .. }
        | I::Lwzx { .. }
        | I::Lbzx { .. }
        | I::Ldx { .. }
        | I::Lhzx { .. }
        | I::Stwx { .. }
        | I::Stdx { .. }
        | I::Stdux { .. }
        | I::Stbx { .. }
        | I::Lwzux { .. }
        | I::Lbzux { .. }
        | I::Lhzux { .. }
        | I::Ldux { .. }
        | I::Lhax { .. }
        | I::Lhaux { .. }
        | I::Lwax { .. }
        | I::Lwaux { .. }
        | I::Sthx { .. }
        | I::Sthux { .. }
        | I::Stwux { .. }
        | I::Stbux { .. }
        | I::Lswi { .. }
        | I::Lswx { .. }
        | I::Stswi { .. }
        | I::Stswx { .. }
        | I::Ldbrx { .. }
        | I::Lwbrx { .. }
        | I::Lhbrx { .. }
        | I::Sdbrx { .. }
        | I::Stwbrx { .. }
        | I::Sthbrx { .. }
        | I::Mftb { .. }
        | I::Mftbu { .. }
        | I::Mfcr { .. }
        | I::Mtcrf { .. }
        | I::Mfocrf { .. }
        | I::Mtocrf { .. }
        | I::Mflr { .. }
        | I::Mtlr { .. }
        | I::Mfctr { .. }
        | I::Mtctr { .. }
        | I::Mfxer { .. }
        | I::Mtxer { .. }
        | I::Mfvrsave { .. }
        | I::Mtvrsave { .. }
        | I::Rlwinm { .. }
        | I::Rlwimi { .. }
        | I::Rlwnm { .. }
        | I::Rldicl { .. }
        | I::Rldicr { .. }
        | I::Rldic { .. }
        | I::Rldimi { .. }
        | I::Rldcl { .. }
        | I::Rldcr { .. }
        | I::Vx { .. }
        | I::Va { .. }
        | I::Vxor { .. }
        | I::Vsldoi { .. }
        | I::Lvlx { .. }
        | I::Lvrx { .. }
        | I::Lvlxl { .. }
        | I::Lvrxl { .. }
        | I::Stvlx { .. }
        | I::Stvrx { .. }
        | I::Stvlxl { .. }
        | I::Stvrxl { .. }
        | I::Lvsl { .. }
        | I::Lvebx { .. }
        | I::Lvsr { .. }
        | I::Lvehx { .. }
        | I::Lvewx { .. }
        | I::Lvx { .. }
        | I::Stvebx { .. }
        | I::Stvehx { .. }
        | I::Stvewx { .. }
        | I::Lvxl { .. }
        | I::Stvx { .. }
        | I::Stvxl { .. }
        | I::Lfs { .. }
        | I::Lfsu { .. }
        | I::Lfd { .. }
        | I::Lfdu { .. }
        | I::Stfs { .. }
        | I::Stfd { .. }
        | I::Stfsu { .. }
        | I::Stfdu { .. }
        | I::Stfiwx { .. }
        | I::Lfsx { .. }
        | I::Lfsux { .. }
        | I::Lfdx { .. }
        | I::Lfdux { .. }
        | I::Stfsx { .. }
        | I::Stfsux { .. }
        | I::Stfdx { .. }
        | I::Stfdux { .. }
        | I::Fp63 { .. }
        | I::Fp59 { .. }
        | I::Li { .. }
        | I::Mr { .. }
        | I::Slwi { .. }
        | I::Srwi { .. }
        | I::Clrlwi { .. }
        | I::Nop
        | I::CmpwZero { .. }
        | I::Clrldi { .. }
        | I::Sldi { .. }
        | I::Srdi { .. }
        | I::LwzCmpwi { .. }
        | I::LiStw { .. }
        | I::MflrStw { .. }
        | I::LwzMtlr { .. }
        | I::MflrStd { .. }
        | I::LdMtlr { .. }
        | I::StdStd { .. }
        | I::CmpwiBc { .. }
        | I::CmpwBc { .. }
        | I::Consumed
        | I::Dcbz { .. }
        | I::Sc { .. } => None,
    }
}
