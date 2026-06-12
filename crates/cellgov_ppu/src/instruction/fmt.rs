//! Assembly-text rendering for [`PpuInstruction`].
//!
//! [`AsmText`] is a pure projection of an already-decoded
//! instruction: it never re-decodes raw words and never carries its
//! own opcode tables. Branch targets render as resolved absolute
//! addresses, which is why the adapter carries the instruction's own
//! `addr`.
//!
//! Output is ASCII-only and allocation-free in the steady state.
//! Every `match` over [`PpuInstruction`] is exhaustive with no `_`
//! arm: a new variant fails compilation here until it gets an
//! explicit rendering.
//!
//! Extended mnemonics follow the PPC v2.02 Book I assembler
//! appendix; the [`simplify`] table is consulted before canonical
//! rendering, and each rewrite has an exact structural gate. Branch
//! `at`-hint bits are dropped (no `+`/`-` suffix is rendered).
// [PPC-Book1 p:154 s:B.2.4] at-bit prediction suffixes; assemblers
// default the at bits to 0b00, and this renderer drops them.

use core::fmt;

use super::ops::{Fp59Shape, Fp63Shape, VaShape, VxShape};
use super::PpuInstruction;
use crate::funcmap::FunctionMap;

/// Width of the mnemonic column; operands start one space later.
const MNEMONIC_COL: usize = 10;

/// Renders one instruction as assembly text. `addr` is the
/// instruction's own vaddr, used to resolve relative branch targets.
pub struct AsmText<'a> {
    /// The decoded instruction to render.
    pub insn: &'a PpuInstruction,
    /// The instruction's own virtual address.
    pub addr: u64,
    /// Optional symbolizer for branch targets. `None` renders bare
    /// hex targets.
    pub symbols: Option<&'a FunctionMap>,
}

impl fmt::Display for AsmText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        render(self.insn, self.addr, self.symbols, f)
    }
}

/// A resolved branch target: bare hex, plus a ` <name+0xoff>` suffix
/// when a [`FunctionMap`] resolves it (`+0x0` is omitted).
struct Target<'a> {
    target: u64,
    symbols: Option<&'a FunctionMap>,
}

impl fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.target)?;
        let Some(map) = self.symbols else {
            return Ok(());
        };
        let Ok(addr32) = u32::try_from(self.target) else {
            return Ok(());
        };
        if let Some(span) = map.span_at(addr32) {
            let delta = addr32 - span.start;
            if delta == 0 {
                write!(f, " <{}>", span.display_name())?;
            } else {
                write!(f, " <{}+0x{delta:x}>", span.display_name())?;
            }
        }
        Ok(())
    }
}

/// Fixed-capacity mnemonic compositor (`addo.`, `bdnzlrl`).
///
/// 23 bytes covers every mnemonic this module emits; overflow is a
/// programming error surfaced via `debug_assert!` and silent
/// truncation in release.
struct Mn {
    buf: [u8; 23],
    len: usize,
}

impl Mn {
    fn new(base: &str) -> Self {
        let mut m = Mn {
            buf: [0; 23],
            len: 0,
        };
        m.push(base);
        m
    }

    fn push(&mut self, s: &str) {
        let take = s.len().min(self.buf.len() - self.len);
        debug_assert!(take == s.len(), "mnemonic overflow: {s}");
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
    }

    fn as_str(&self) -> &str {
        // SAFETY-free: buf is only ever filled from &str bytes.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl fmt::Write for Mn {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s);
        Ok(())
    }
}

/// Compose `base` + optional `o` (OE) + optional `.` (Rc).
fn mn_oe_rc(base: &str, oe: bool, rc: bool) -> Mn {
    let mut m = Mn::new(base);
    if oe {
        m.push("o");
    }
    if rc {
        m.push(".");
    }
    m
}

/// Compose `base` + optional `.` (Rc).
fn mn_rc(base: &str, rc: bool) -> Mn {
    mn_oe_rc(base, false, rc)
}

/// Write `mnemonic` padded to [`MNEMONIC_COL`], then the operands.
fn op(f: &mut fmt::Formatter<'_>, mn: &str, operands: fmt::Arguments<'_>) -> fmt::Result {
    write!(f, "{mn:<MNEMONIC_COL$} ")?;
    f.write_fmt(operands)
}

/// Write a mnemonic with no operands (no trailing padding).
fn op0(f: &mut fmt::Formatter<'_>, mn: &str) -> fmt::Result {
    f.write_str(mn)
}

/// CR bit operand per binutils convention: `4*cr<N>+<cond>`, or the
/// bare condition name when the field is cr0.
struct CrBit(u8);

impl fmt::Display for CrBit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cond = ["lt", "gt", "eq", "so"][(self.0 & 3) as usize];
        let field = self.0 >> 2;
        if field == 0 {
            f.write_str(cond)
        } else {
            write!(f, "4*cr{field}+{cond}")
        }
    }
}

/// Resolve a branch displacement to its absolute target.
// [PPC-Book1 p:24 s:2.4.1] AA=1 takes the sign-extended displacement
// as the absolute address; AA=0 adds it to the branch's own address.
fn branch_target(addr: u64, offset: i64, abs: bool) -> u64 {
    if abs {
        (offset as u64) & 0xFFFF_FFFF
    } else {
        addr.wrapping_add(offset as u64)
    }
}

/// Branch mnemonic suffix composition: `l` for link, then `a` for
/// absolute (`b`, `bl`, `ba`, `bla`).
fn mn_branch(base: &str, link: bool, abs: bool) -> Mn {
    let mut m = Mn::new(base);
    if link {
        m.push("l");
    }
    if abs {
        m.push("a");
    }
    m
}

/// D-form load/store: `mn rt, imm(ra)` (or `fN`/`vN` via `reg`).
fn mem_d(f: &mut fmt::Formatter<'_>, mn: &str, reg: char, rt: u8, imm: i16, ra: u8) -> fmt::Result {
    op(f, mn, format_args!("{reg}{rt}, {imm}(r{ra})"))
}

/// X-form three-register op with a register-class prefix on the
/// first operand: `mn Xt, ra, rb`.
fn mem_x(f: &mut fmt::Formatter<'_>, mn: &str, reg: char, rt: u8, ra: u8, rb: u8) -> fmt::Result {
    op(f, mn, format_args!("{reg}{rt}, r{ra}, r{rb}"))
}

/// Compare operands: the `cr0` field is omitted entirely.
macro_rules! cmp {
    ($f:expr, $mn:expr, $bf:expr, $fmt:literal, $($args:expr),*) => {
        if $bf == 0 {
            op($f, $mn, format_args!($fmt, $($args),*))
        } else {
            op($f, $mn, format_args!(concat!("cr{}, ", $fmt), $bf, $($args),*))
        }
    };
}

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
struct Simplified {
    mn: Mn,
    ops: SimpleOps,
}

impl Simplified {
    fn render(&self, symbols: Option<&FunctionMap>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
fn simplify(insn: &PpuInstruction, addr: u64) -> Option<Simplified> {
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
            let target = branch_target(addr, offset as i64, aa);
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

/// Render `insn` at `addr` into `f`. The single exhaustive match
/// that defines the canonical operand layout for every variant.
fn render(
    insn: &PpuInstruction,
    addr: u64,
    symbols: Option<&FunctionMap>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    use PpuInstruction as I;
    if let Some(simplified) = simplify(insn, addr) {
        return simplified.render(symbols, f);
    }
    match *insn {
        // -- Integer loads (D-form) --
        I::Lwz { rt, ra, imm } => mem_d(f, "lwz", 'r', rt, imm, ra),
        I::Lbz { rt, ra, imm } => mem_d(f, "lbz", 'r', rt, imm, ra),
        I::Lhz { rt, ra, imm } => mem_d(f, "lhz", 'r', rt, imm, ra),
        I::Lha { rt, ra, imm } => mem_d(f, "lha", 'r', rt, imm, ra),
        I::Lhau { rt, ra, imm } => mem_d(f, "lhau", 'r', rt, imm, ra),
        I::Lmw { rt, ra, imm } => mem_d(f, "lmw", 'r', rt, imm, ra),
        I::Lwzu { rt, ra, imm } => mem_d(f, "lwzu", 'r', rt, imm, ra),
        I::Lbzu { rt, ra, imm } => mem_d(f, "lbzu", 'r', rt, imm, ra),
        I::Lhzu { rt, ra, imm } => mem_d(f, "lhzu", 'r', rt, imm, ra),
        I::Ld { rt, ra, imm } => mem_d(f, "ld", 'r', rt, imm, ra),
        I::Ldu { rt, ra, imm } => mem_d(f, "ldu", 'r', rt, imm, ra),
        I::Lwa { rt, ra, imm } => mem_d(f, "lwa", 'r', rt, imm, ra),

        // -- Integer stores (D-form) --
        I::Stw { rs, ra, imm } => mem_d(f, "stw", 'r', rs, imm, ra),
        I::Stwu { rs, ra, imm } => mem_d(f, "stwu", 'r', rs, imm, ra),
        I::Std { rs, ra, imm } => mem_d(f, "std", 'r', rs, imm, ra),
        I::Stdu { rs, ra, imm } => mem_d(f, "stdu", 'r', rs, imm, ra),
        I::Stb { rs, ra, imm } => mem_d(f, "stb", 'r', rs, imm, ra),
        I::Stbu { rs, ra, imm } => mem_d(f, "stbu", 'r', rs, imm, ra),
        I::Sth { rs, ra, imm } => mem_d(f, "sth", 'r', rs, imm, ra),
        I::Sthu { rs, ra, imm } => mem_d(f, "sthu", 'r', rs, imm, ra),
        I::Stmw { rs, ra, imm } => mem_d(f, "stmw", 'r', rs, imm, ra),

        // -- Arithmetic immediates (signed decimal) --
        I::Addi { rt, ra, imm } => op(f, "addi", format_args!("r{rt}, r{ra}, {imm}")),
        I::Addis { rt, ra, imm } => op(f, "addis", format_args!("r{rt}, r{ra}, {imm}")),
        I::Subfic { rt, ra, imm } => op(f, "subfic", format_args!("r{rt}, r{ra}, {imm}")),
        I::Mulli { rt, ra, imm } => op(f, "mulli", format_args!("r{rt}, r{ra}, {imm}")),
        I::Addic { rt, ra, imm } => op(f, "addic", format_args!("r{rt}, r{ra}, {imm}")),
        I::AddicDot { rt, ra, imm } => op(f, "addic.", format_args!("r{rt}, r{ra}, {imm}")),

        // -- Logical immediates (hex) --
        I::Ori { ra, rs, imm } => op(f, "ori", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Oris { ra, rs, imm } => op(f, "oris", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Xori { ra, rs, imm } => op(f, "xori", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::Xoris { ra, rs, imm } => op(f, "xoris", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::AndiDot { ra, rs, imm } => op(f, "andi.", format_args!("r{ra}, r{rs}, 0x{imm:x}")),
        I::AndisDot { ra, rs, imm } => op(f, "andis.", format_args!("r{ra}, r{rs}, 0x{imm:x}")),

        // -- XO-form arithmetic --
        I::Add { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("add", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subf { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subf", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subfc { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subfc", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Subfe { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("subfe", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Adde { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("adde", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mullw { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("mullw", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulld { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("mulld", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divw { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divw", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divwu { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divwu", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divd { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divd", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Divdu { rt, ra, rb, oe, rc } => op(
            f,
            mn_oe_rc("divdu", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhw { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhw", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhwu { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhwu", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhd { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhd", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Mulhdu { rt, ra, rb, rc } => op(
            f,
            mn_rc("mulhdu", rc).as_str(),
            format_args!("r{rt}, r{ra}, r{rb}"),
        ),
        I::Neg { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("neg", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Addze { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("addze", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Subfze { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("subfze", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Subfme { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("subfme", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),
        I::Addme { rt, ra, oe, rc } => op(
            f,
            mn_oe_rc("addme", oe, rc).as_str(),
            format_args!("r{rt}, r{ra}"),
        ),

        // -- X-form logical --
        I::Or { ra, rs, rb, rc } => op(
            f,
            mn_rc("or", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Orc { ra, rs, rb, rc } => op(
            f,
            mn_rc("orc", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::And { ra, rs, rb, rc } => op(
            f,
            mn_rc("and", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Andc { ra, rs, rb, rc } => op(
            f,
            mn_rc("andc", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Nor { ra, rs, rb, rc } => op(
            f,
            mn_rc("nor", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Xor { ra, rs, rb, rc } => op(
            f,
            mn_rc("xor", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Eqv { ra, rs, rb, rc } => op(
            f,
            mn_rc("eqv", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Nand { ra, rs, rb, rc } => op(
            f,
            mn_rc("nand", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),

        // -- Shifts --
        I::Slw { ra, rs, rb, rc } => op(
            f,
            mn_rc("slw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srw { ra, rs, rb, rc } => op(
            f,
            mn_rc("srw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Sld { ra, rs, rb, rc } => op(
            f,
            mn_rc("sld", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srd { ra, rs, rb, rc } => op(
            f,
            mn_rc("srd", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Sraw { ra, rs, rb, rc } => op(
            f,
            mn_rc("sraw", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srad { ra, rs, rb, rc } => op(
            f,
            mn_rc("srad", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}"),
        ),
        I::Srawi { ra, rs, sh, rc } => op(
            f,
            mn_rc("srawi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}"),
        ),
        I::Sradi { ra, rs, sh, rc } => op(
            f,
            mn_rc("sradi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}"),
        ),

        // -- Bit counting / sign extension --
        I::Cntlzw { ra, rs, rc } => op(
            f,
            mn_rc("cntlzw", rc).as_str(),
            format_args!("r{ra}, r{rs}"),
        ),
        I::Cntlzd { ra, rs, rc } => op(
            f,
            mn_rc("cntlzd", rc).as_str(),
            format_args!("r{ra}, r{rs}"),
        ),
        I::Popcntb { ra, rs } => op(f, "popcntb", format_args!("r{ra}, r{rs}")),
        I::Extsb { ra, rs, rc } => op(f, mn_rc("extsb", rc).as_str(), format_args!("r{ra}, r{rs}")),
        I::Extsh { ra, rs, rc } => op(f, mn_rc("extsh", rc).as_str(), format_args!("r{ra}, r{rs}")),
        I::Extsw { ra, rs, rc } => op(f, mn_rc("extsw", rc).as_str(), format_args!("r{ra}, r{rs}")),

        // -- Traps / system-ish X-forms --
        I::Tw { to, ra, rb } => op(f, "tw", format_args!("{to}, r{ra}, r{rb}")),
        I::Td { to, ra, rb } => op(f, "td", format_args!("{to}, r{ra}, r{rb}")),
        I::Mcrxr { bf } => op(f, "mcrxr", format_args!("cr{bf}")),

        // -- Compares (cr0 field omitted per convention) --
        I::Cmpwi { bf, ra, imm } => cmp!(f, "cmpwi", bf, "r{}, {}", ra, imm),
        I::Cmpdi { bf, ra, imm } => cmp!(f, "cmpdi", bf, "r{}, {}", ra, imm),
        I::Cmplwi { bf, ra, imm } => cmp!(f, "cmplwi", bf, "r{}, 0x{:x}", ra, imm),
        I::Cmpldi { bf, ra, imm } => cmp!(f, "cmpldi", bf, "r{}, 0x{:x}", ra, imm),
        I::Cmpw { bf, ra, rb } => cmp!(f, "cmpw", bf, "r{}, r{}", ra, rb),
        I::Cmpd { bf, ra, rb } => cmp!(f, "cmpd", bf, "r{}, r{}", ra, rb),
        I::Cmplw { bf, ra, rb } => cmp!(f, "cmplw", bf, "r{}, r{}", ra, rb),
        I::Cmpld { bf, ra, rb } => cmp!(f, "cmpld", bf, "r{}, r{}", ra, rb),

        // -- Branches (canonical; extended mnemonics live in simplify) --
        I::B { offset, aa, link } => {
            let target = branch_target(addr, offset as i64, aa);
            op(
                f,
                mn_branch("b", link, aa).as_str(),
                format_args!("{}", Target { target, symbols }),
            )
        }
        I::Bc {
            bo,
            bi,
            offset,
            aa,
            link,
        } => {
            let target = branch_target(addr, offset as i64, aa);
            op(
                f,
                mn_branch("bc", link, aa).as_str(),
                format_args!("{bo}, {bi}, {}", Target { target, symbols }),
            )
        }
        I::Bclr { bo, bi, link } => op(
            f,
            mn_branch("bclr", link, false).as_str(),
            format_args!("{bo}, {bi}"),
        ),
        I::Bcctr { bo, bi, link } => op(
            f,
            mn_branch("bcctr", link, false).as_str(),
            format_args!("{bo}, {bi}"),
        ),

        // -- CR logical --
        I::Mcrf { crfd, crfs } => op(f, "mcrf", format_args!("cr{crfd}, cr{crfs}")),
        I::Crand { bt, ba, bb } => op(
            f,
            "crand",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crandc { bt, ba, bb } => op(
            f,
            "crandc",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Cror { bt, ba, bb } => op(
            f,
            "cror",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crorc { bt, ba, bb } => op(
            f,
            "crorc",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crxor { bt, ba, bb } => op(
            f,
            "crxor",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crnand { bt, ba, bb } => op(
            f,
            "crnand",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Crnor { bt, ba, bb } => op(
            f,
            "crnor",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),
        I::Creqv { bt, ba, bb } => op(
            f,
            "creqv",
            format_args!("{}, {}, {}", CrBit(bt), CrBit(ba), CrBit(bb)),
        ),

        // -- Indexed loads/stores --
        I::Lwzx { rt, ra, rb } => mem_x(f, "lwzx", 'r', rt, ra, rb),
        I::Lbzx { rt, ra, rb } => mem_x(f, "lbzx", 'r', rt, ra, rb),
        I::Lhzx { rt, ra, rb } => mem_x(f, "lhzx", 'r', rt, ra, rb),
        I::Ldx { rt, ra, rb } => mem_x(f, "ldx", 'r', rt, ra, rb),
        I::Lwzux { rt, ra, rb } => mem_x(f, "lwzux", 'r', rt, ra, rb),
        I::Lbzux { rt, ra, rb } => mem_x(f, "lbzux", 'r', rt, ra, rb),
        I::Lhzux { rt, ra, rb } => mem_x(f, "lhzux", 'r', rt, ra, rb),
        I::Ldux { rt, ra, rb } => mem_x(f, "ldux", 'r', rt, ra, rb),
        I::Lhax { rt, ra, rb } => mem_x(f, "lhax", 'r', rt, ra, rb),
        I::Lhaux { rt, ra, rb } => mem_x(f, "lhaux", 'r', rt, ra, rb),
        I::Lwax { rt, ra, rb } => mem_x(f, "lwax", 'r', rt, ra, rb),
        I::Lwaux { rt, ra, rb } => mem_x(f, "lwaux", 'r', rt, ra, rb),
        I::Stwx { rs, ra, rb } => mem_x(f, "stwx", 'r', rs, ra, rb),
        I::Stdx { rs, ra, rb } => mem_x(f, "stdx", 'r', rs, ra, rb),
        I::Stdux { rs, ra, rb } => mem_x(f, "stdux", 'r', rs, ra, rb),
        I::Stbx { rs, ra, rb } => mem_x(f, "stbx", 'r', rs, ra, rb),
        I::Stbux { rs, ra, rb } => mem_x(f, "stbux", 'r', rs, ra, rb),
        I::Sthx { rs, ra, rb } => mem_x(f, "sthx", 'r', rs, ra, rb),
        I::Sthux { rs, ra, rb } => mem_x(f, "sthux", 'r', rs, ra, rb),
        I::Stwux { rs, ra, rb } => mem_x(f, "stwux", 'r', rs, ra, rb),

        // -- Atomics --
        I::Lwarx { rt, ra, rb } => mem_x(f, "lwarx", 'r', rt, ra, rb),
        I::Ldarx { rt, ra, rb } => mem_x(f, "ldarx", 'r', rt, ra, rb),
        I::Stwcx { rs, ra, rb } => mem_x(f, "stwcx.", 'r', rs, ra, rb),
        I::Stdcx { rs, ra, rb } => mem_x(f, "stdcx.", 'r', rs, ra, rb),

        // -- String moves --
        I::Lswi { rt, ra, nb } => op(f, "lswi", format_args!("r{rt}, r{ra}, {nb}")),
        I::Stswi { rs, ra, nb } => op(f, "stswi", format_args!("r{rs}, r{ra}, {nb}")),
        I::Lswx { rt, ra, rb } => mem_x(f, "lswx", 'r', rt, ra, rb),
        I::Stswx { rs, ra, rb } => mem_x(f, "stswx", 'r', rs, ra, rb),

        // -- Byte-reverse --
        I::Ldbrx { rt, ra, rb } => mem_x(f, "ldbrx", 'r', rt, ra, rb),
        I::Lwbrx { rt, ra, rb } => mem_x(f, "lwbrx", 'r', rt, ra, rb),
        I::Lhbrx { rt, ra, rb } => mem_x(f, "lhbrx", 'r', rt, ra, rb),
        I::Sdbrx { rs, ra, rb } => mem_x(f, "sdbrx", 'r', rs, ra, rb),
        I::Stwbrx { rs, ra, rb } => mem_x(f, "stwbrx", 'r', rs, ra, rb),
        I::Sthbrx { rs, ra, rb } => mem_x(f, "sthbrx", 'r', rs, ra, rb),

        // -- SPR / CR moves --
        I::Mftb { rt } => op(f, "mftb", format_args!("r{rt}")),
        I::Mftbu { rt } => op(f, "mftbu", format_args!("r{rt}")),
        I::Mfcr { rt } => op(f, "mfcr", format_args!("r{rt}")),
        I::Mtcrf { rs, crm } => op(f, "mtcrf", format_args!("0x{crm:x}, r{rs}")),
        I::Mfocrf { rt, crm } => op(f, "mfocrf", format_args!("r{rt}, 0x{crm:x}")),
        I::Mtocrf { rs, crm } => op(f, "mtocrf", format_args!("0x{crm:x}, r{rs}")),
        I::Mflr { rt } => op(f, "mflr", format_args!("r{rt}")),
        I::Mtlr { rs } => op(f, "mtlr", format_args!("r{rs}")),
        I::Mfctr { rt } => op(f, "mfctr", format_args!("r{rt}")),
        I::Mtctr { rs } => op(f, "mtctr", format_args!("r{rs}")),
        I::Mfxer { rt } => op(f, "mfxer", format_args!("r{rt}")),
        I::Mtxer { rs } => op(f, "mtxer", format_args!("r{rs}")),
        I::Mfvrsave { rt } => op(f, "mfvrsave", format_args!("r{rt}")),
        I::Mtvrsave { rs } => op(f, "mtvrsave", format_args!("r{rs}")),

        // -- Rotates --
        I::Rlwinm {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwinm", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}, {me}"),
        ),
        I::Rlwimi {
            ra,
            rs,
            sh,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwimi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}, {me}"),
        ),
        I::Rlwnm {
            ra,
            rs,
            rb,
            mb,
            me,
            rc,
        } => op(
            f,
            mn_rc("rlwnm", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {mb}, {me}"),
        ),
        I::Rldicl { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldicl", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldicr { ra, rs, sh, me, rc } => op(
            f,
            mn_rc("rldicr", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {me}"),
        ),
        I::Rldic { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldic", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldimi { ra, rs, sh, mb, rc } => op(
            f,
            mn_rc("rldimi", rc).as_str(),
            format_args!("r{ra}, r{rs}, {sh}, {mb}"),
        ),
        I::Rldcl { ra, rs, rb, mb, rc } => op(
            f,
            mn_rc("rldcl", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {mb}"),
        ),
        I::Rldcr { ra, rs, rb, me, rc } => op(
            f,
            mn_rc("rldcr", rc).as_str(),
            format_args!("r{ra}, r{rs}, r{rb}, {me}"),
        ),

        // -- Vector (typed) --
        I::Vxor { vt, va, vb } => op(f, "vxor", format_args!("v{vt}, v{va}, v{vb}")),
        I::Vsldoi { vt, va, vb, shb } => {
            op(f, "vsldoi", format_args!("v{vt}, v{va}, v{vb}, {shb}"))
        }
        I::Lvlx { vt, ra, rb } => mem_x(f, "lvlx", 'v', vt, ra, rb),
        I::Lvrx { vt, ra, rb } => mem_x(f, "lvrx", 'v', vt, ra, rb),
        I::Lvlxl { vt, ra, rb } => mem_x(f, "lvlxl", 'v', vt, ra, rb),
        I::Lvrxl { vt, ra, rb } => mem_x(f, "lvrxl", 'v', vt, ra, rb),
        I::Stvlx { vs, ra, rb } => mem_x(f, "stvlx", 'v', vs, ra, rb),
        I::Stvrx { vs, ra, rb } => mem_x(f, "stvrx", 'v', vs, ra, rb),
        I::Stvlxl { vs, ra, rb } => mem_x(f, "stvlxl", 'v', vs, ra, rb),
        I::Stvrxl { vs, ra, rb } => mem_x(f, "stvrxl", 'v', vs, ra, rb),
        I::Lvsl { vt, ra, rb } => mem_x(f, "lvsl", 'v', vt, ra, rb),
        I::Lvsr { vt, ra, rb } => mem_x(f, "lvsr", 'v', vt, ra, rb),
        I::Lvebx { vt, ra, rb } => mem_x(f, "lvebx", 'v', vt, ra, rb),
        I::Lvehx { vt, ra, rb } => mem_x(f, "lvehx", 'v', vt, ra, rb),
        I::Lvewx { vt, ra, rb } => mem_x(f, "lvewx", 'v', vt, ra, rb),
        I::Lvx { vt, ra, rb } => mem_x(f, "lvx", 'v', vt, ra, rb),
        I::Lvxl { vt, ra, rb } => mem_x(f, "lvxl", 'v', vt, ra, rb),
        I::Stvebx { vs, ra, rb } => mem_x(f, "stvebx", 'v', vs, ra, rb),
        I::Stvehx { vs, ra, rb } => mem_x(f, "stvehx", 'v', vs, ra, rb),
        I::Stvewx { vs, ra, rb } => mem_x(f, "stvewx", 'v', vs, ra, rb),
        I::Stvx { vs, ra, rb } => mem_x(f, "stvx", 'v', vs, ra, rb),
        I::Stvxl { vs, ra, rb } => mem_x(f, "stvxl", 'v', vs, ra, rb),

        // -- Vector family ops (mnemonic is the op-enum name) --
        I::Vx {
            op: vx_op,
            rc,
            vt,
            va,
            vb,
        } => {
            let mn = mn_rc(<&'static str>::from(vx_op), rc);
            match vx_op.shape() {
                VxShape::VdVaVb => op(f, mn.as_str(), format_args!("v{vt}, v{va}, v{vb}")),
                VxShape::VdVb => op(f, mn.as_str(), format_args!("v{vt}, v{vb}")),
                VxShape::VdVbUimm => op(f, mn.as_str(), format_args!("v{vt}, v{vb}, {va}")),
                VxShape::VdSimm => {
                    // The vA slot carries a sign-extended 5-bit SIMM.
                    let simm = ((va << 3) as i8) >> 3;
                    op(f, mn.as_str(), format_args!("v{vt}, {simm}"))
                }
            }
        }
        I::Va {
            op: va_op,
            vt,
            va,
            vb,
            vc,
        } => {
            let mn = <&'static str>::from(va_op);
            match va_op.shape() {
                VaShape::VdVaVbVc => op(f, mn, format_args!("v{vt}, v{va}, v{vb}, v{vc}")),
                VaShape::VdVaVcVb => op(f, mn, format_args!("v{vt}, v{va}, v{vc}, v{vb}")),
                VaShape::VdVaVbShb => op(f, mn, format_args!("v{vt}, v{va}, v{vb}, {}", vc & 0xF)),
            }
        }

        // -- FP loads/stores --
        I::Lfs { frt, ra, imm } => mem_d(f, "lfs", 'f', frt, imm, ra),
        I::Lfsu { frt, ra, imm } => mem_d(f, "lfsu", 'f', frt, imm, ra),
        I::Lfd { frt, ra, imm } => mem_d(f, "lfd", 'f', frt, imm, ra),
        I::Lfdu { frt, ra, imm } => mem_d(f, "lfdu", 'f', frt, imm, ra),
        I::Stfs { frs, ra, imm } => mem_d(f, "stfs", 'f', frs, imm, ra),
        I::Stfsu { frs, ra, imm } => mem_d(f, "stfsu", 'f', frs, imm, ra),
        I::Stfd { frs, ra, imm } => mem_d(f, "stfd", 'f', frs, imm, ra),
        I::Stfdu { frs, ra, imm } => mem_d(f, "stfdu", 'f', frs, imm, ra),
        I::Stfiwx { frs, ra, rb } => mem_x(f, "stfiwx", 'f', frs, ra, rb),
        I::Lfsx { frt, ra, rb } => mem_x(f, "lfsx", 'f', frt, ra, rb),
        I::Lfsux { frt, ra, rb } => mem_x(f, "lfsux", 'f', frt, ra, rb),
        I::Lfdx { frt, ra, rb } => mem_x(f, "lfdx", 'f', frt, ra, rb),
        I::Lfdux { frt, ra, rb } => mem_x(f, "lfdux", 'f', frt, ra, rb),
        I::Stfsx { frs, ra, rb } => mem_x(f, "stfsx", 'f', frs, ra, rb),
        I::Stfsux { frs, ra, rb } => mem_x(f, "stfsux", 'f', frs, ra, rb),
        I::Stfdx { frs, ra, rb } => mem_x(f, "stfdx", 'f', frs, ra, rb),
        I::Stfdux { frs, ra, rb } => mem_x(f, "stfdux", 'f', frs, ra, rb),

        // -- FP family ops (mnemonic is the op-enum name) --
        I::Fp63 {
            op: fp_op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let mn = mn_rc(<&'static str>::from(fp_op), rc);
            let mn = mn.as_str();
            match fp_op.shape() {
                Fp63Shape::FrtFraFrb => op(f, mn, format_args!("f{frt}, f{fra}, f{frb}")),
                Fp63Shape::FrtFrb => op(f, mn, format_args!("f{frt}, f{frb}")),
                Fp63Shape::FrtFraFrc => op(f, mn, format_args!("f{frt}, f{fra}, f{frc}")),
                Fp63Shape::FrtFraFrcFrb => {
                    op(f, mn, format_args!("f{frt}, f{fra}, f{frc}, f{frb}"))
                }
                Fp63Shape::CrfFraFrb => op(f, mn, format_args!("cr{}, f{fra}, f{frb}", frt >> 2)),
                Fp63Shape::Frt => op(f, mn, format_args!("f{frt}")),
                Fp63Shape::CrfCrf => op(f, mn, format_args!("cr{}, cr{}", frt >> 2, fra >> 2)),
                // [PPC-Book1 p:122 s:4.6.9] mtfsfi: U at PPC bits
                // 16:19, the top 4 bits of the frb slot.
                Fp63Shape::CrfImm => op(f, mn, format_args!("cr{}, {}", frt >> 2, frb >> 1)),
                Fp63Shape::Crb => op(f, mn, format_args!("{frt}")),
                // [PPC-Book1 p:9 s:1.7.9 XFL-Form] FLM at PPC bits
                // 7:14: high nibble in the frt slot, low nibble in
                // the fra slot's upper bits.
                Fp63Shape::FmFrb => {
                    let fm = ((frt as u32 & 0x0F) << 4) | ((fra as u32 >> 1) & 0x0F);
                    op(f, mn, format_args!("0x{fm:x}, f{frb}"))
                }
            }
        }
        I::Fp59 {
            op: fp_op,
            frt,
            fra,
            frb,
            frc,
            rc,
        } => {
            let mn = mn_rc(<&'static str>::from(fp_op), rc);
            let mn = mn.as_str();
            match fp_op.shape() {
                Fp59Shape::FrtFraFrb => op(f, mn, format_args!("f{frt}, f{fra}, f{frb}")),
                Fp59Shape::FrtFrb => op(f, mn, format_args!("f{frt}, f{frb}")),
                Fp59Shape::FrtFraFrc => op(f, mn, format_args!("f{frt}, f{fra}, f{frc}")),
                Fp59Shape::FrtFraFrcFrb => {
                    op(f, mn, format_args!("f{frt}, f{fra}, f{frc}, f{frb}"))
                }
            }
        }

        // -- Quickened forms (shadow-builder only; never from decode) --
        I::Li { rt, imm } => op(f, "li", format_args!("r{rt}, {imm}")),
        I::Mr { ra, rs } => op(f, "mr", format_args!("r{ra}, r{rs}")),
        I::Slwi { ra, rs, n } => op(f, "slwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Srwi { ra, rs, n } => op(f, "srwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Clrlwi { ra, rs, n } => op(f, "clrlwi", format_args!("r{ra}, r{rs}, {n}")),
        I::Sldi { ra, rs, n } => op(f, "sldi", format_args!("r{ra}, r{rs}, {n}")),
        I::Srdi { ra, rs, n } => op(f, "srdi", format_args!("r{ra}, r{rs}, {n}")),
        I::Clrldi { ra, rs, n } => op(f, "clrldi", format_args!("r{ra}, r{rs}, {n}")),
        I::Nop => op0(f, "nop"),
        I::CmpwZero { bf, ra } => cmp!(f, "cmpwi", bf, "r{}, {}", ra, 0),

        // -- Superinstructions (shadow-builder only). Rendered as the
        //    two fused halves joined with `; ` so the reader sees real
        //    assembly, not an internal codename. --
        I::LwzCmpwi {
            rt,
            ra_load,
            offset,
            bf,
            cmp_imm,
        } => {
            op(f, "lwz", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            if bf == 0 {
                write!(f, "; cmpwi r{rt}, {cmp_imm}")
            } else {
                write!(f, "; cmpwi cr{bf}, r{rt}, {cmp_imm}")
            }
        }
        I::LiStw {
            rt,
            imm,
            ra_store,
            store_offset,
        } => {
            op(f, "li", format_args!("r{rt}, {imm}"))?;
            write!(f, "; stw r{rt}, {store_offset}(r{ra_store})")
        }
        I::MflrStw {
            rt,
            ra_store,
            store_offset,
        } => {
            op(f, "mflr", format_args!("r{rt}"))?;
            write!(f, "; stw r{rt}, {store_offset}(r{ra_store})")
        }
        I::MflrStd {
            rt,
            ra_store,
            store_offset,
        } => {
            op(f, "mflr", format_args!("r{rt}"))?;
            write!(f, "; std r{rt}, {store_offset}(r{ra_store})")
        }
        I::LwzMtlr {
            rt,
            ra_load,
            offset,
        } => {
            op(f, "lwz", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            write!(f, "; mtlr r{rt}")
        }
        I::LdMtlr {
            rt,
            ra_load,
            offset,
        } => {
            op(f, "ld", format_args!("r{rt}, {offset}(r{ra_load})"))?;
            write!(f, "; mtlr r{rt}")
        }
        I::StdStd {
            rs1,
            rs2,
            ra,
            offset1,
        } => {
            op(f, "std", format_args!("r{rs1}, {offset1}(r{ra})"))?;
            let offset2 = offset1 as i32 + 8;
            write!(f, "; std r{rs2}, {offset2}(r{ra})")
        }
        I::CmpwiBc {
            bf,
            ra,
            imm,
            bo,
            bi,
            target_offset,
        } => {
            cmp!(f, "cmpwi", bf, "r{}, {}", ra, imm)?;
            // The fused bc occupies addr+4; its displacement is
            // relative to its own address.
            let target = branch_target(addr.wrapping_add(4), target_offset as i64, false);
            write!(f, "; bc {bo}, {bi}, 0x{target:x}")
        }
        I::CmpwBc {
            bf,
            ra,
            rb,
            bo,
            bi,
            target_offset,
        } => {
            cmp!(f, "cmpw", bf, "r{}, r{}", ra, rb)?;
            let target = branch_target(addr.wrapping_add(4), target_offset as i64, false);
            write!(f, "; bc {bo}, {bi}, 0x{target:x}")
        }
        I::Consumed => op0(f, ".consumed"),

        // -- Cache / system --
        I::Dcbz { ra, rb } => op(f, "dcbz", format_args!("r{ra}, r{rb}")),
        I::Sc { lev } => {
            if lev == 0 {
                op0(f, "sc")
            } else {
                op(f, "sc", format_args!("{lev}"))
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/fmt_tests.rs"]
mod tests;
