//! Operand classes read from a raw instruction word.

use crate::decode;
use crate::instruction::{Fp63Op, PpuInstruction};

/// The base register of a D-form or DS-form access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BaseClass {
    /// RA = 0, which the access reads as the literal zero of `(RA|0)`.
    Zero,
    /// r1.
    R1,
    /// r2.
    R2,
    /// r13.
    R13,
    /// Any other register.
    Other,
}

impl BaseClass {
    fn of(ra: u8) -> Self {
        match ra {
            0 => Self::Zero,
            1 => Self::R1,
            2 => Self::R2,
            13 => Self::R13,
            _ => Self::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::R1 => "r1",
            Self::R2 => "r2",
            Self::R13 => "r13",
            Self::Other => "other",
        }
    }
}

/// One instruction as the profile counts it: its decoded variant and
/// the operand classes its form has. A class the form lacks is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InsnClass {
    /// The variant the decoder gives the word, or the operation's
    /// mnemonic for a floating-point or vector family variant.
    pub name: &'static str,
    /// The base register of a D-form or DS-form access.
    pub base: Option<BaseClass>,
    /// Whether the immediate of an immediate-form arithmetic, logical or
    /// compare instruction is zero.
    pub imm_zero: Option<bool>,
    /// The Rc bit of a form that has one.
    pub rc: Option<bool>,
    /// Whether the RT or RS field names the same register as RA.
    pub same_reg: Option<bool>,
    /// The BO field of a conditional branch, its hint bits cleared.
    pub bo: Option<u8>,
    /// The AA bit of a branch.
    pub aa: Option<bool>,
    /// The LK bit of a branch.
    pub lk: Option<bool>,
}

/// Two text-adjacent instructions that retired one after the other,
/// and how their operands relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PairClass {
    /// The instruction at the lower address.
    pub first: InsnClass,
    /// The instruction after it.
    pub second: InsnClass,
    /// The second reads the GPR the first writes.
    pub dependent: bool,
    /// Both are D-form or DS-form accesses with one base register.
    pub shared_base: bool,
    /// Both share a base and the second's displacement is the first's
    /// plus the first's access size.
    pub adjacent: bool,
}

/// The fields of a raw word the classes read.
#[derive(Clone, Copy)]
struct Fields {
    op: u32,
    rt: u8,
    ra: u8,
    rb: u8,
    xo: u32,
    word: u32,
}

impl Fields {
    fn of(word: u32) -> Self {
        Self {
            op: word >> 26,
            rt: ((word >> 21) & 31) as u8,
            ra: ((word >> 16) & 31) as u8,
            rb: ((word >> 11) & 31) as u8,
            xo: (word >> 1) & 0x3FF,
            word,
        }
    }

    /// The access size and the displacement of a D-form or DS-form load
    /// or store.
    fn access(self) -> Option<(u8, i16)> {
        let d = self.word as u16 as i16;
        let size = match self.op {
            34 | 35 | 38 | 39 => 1,
            40..=45 => 2,
            32 | 33 | 36 | 37 | 46 | 47 | 48 | 49 | 52 | 53 => 4,
            50 | 51 | 54 | 55 => 8,
            58 if self.word & 3 == 2 => return Some((4, d & !3)),
            58 | 62 => return Some((8, d & !3)),
            _ => return None,
        };
        Some((size, d))
    }

    fn is_store(self) -> bool {
        matches!(self.op, 36..=39 | 44 | 45 | 47 | 52..=55 | 62)
    }

    fn is_branch(self) -> bool {
        matches!(self.op, 16 | 18) || (self.op == 19 && matches!(self.xo, 16 | 528))
    }

    /// The GPR the word writes, for the forms the pair shapes ask about.
    fn gpr_target(self) -> Option<u8> {
        match self.op {
            32..=35 | 40..=43 | 46 | 58 => Some(self.rt),
            7 | 8 | 12..=15 => Some(self.rt),
            20..=30 => Some(self.ra),
            31 if self.is_x_logical() => Some(self.ra),
            31 if self.is_xo_arithmetic() => Some(self.rt),
            // mfcr, mfspr, mftb and the indexed loads.
            31 if matches!(self.xo, 19 | 339 | 371 | 21 | 23 | 87 | 279 | 341 | 343) => {
                Some(self.rt)
            }
            _ => None,
        }
    }

    /// An opcode-31 logical, shift or extend form, which writes RA and
    /// has an Rc bit.
    ///
    /// [PPC-Book1 p:79] sradi is XS-form: its 9-bit XO 413 puts the high
    /// SH bit in raw bit 30, so the 10-bit field reads 826 or 827.
    fn is_x_logical(self) -> bool {
        self.op == 31
            && matches!(
                self.xo,
                24 | 26
                    | 27
                    | 28
                    | 58
                    | 60
                    | 124
                    | 284
                    | 316
                    | 412
                    | 444
                    | 476
                    | 536
                    | 539
                    | 792
                    | 794
                    | 824
                    | 826
                    | 827
                    | 922
                    | 954
                    | 986
            )
    }

    /// An opcode-31 XO-form arithmetic, which writes RT and has an Rc
    /// bit. OE is the top bit of the field.
    ///
    /// [PPC-Book1 p:57 s:3.3.8] mulhd, mulhw, mulhdu and mulhwu have an
    /// Rc bit and no OE bit; [PPC-Book1 p:56 s:3.3.8] mulld has both.
    fn is_xo_arithmetic(self) -> bool {
        self.op == 31
            && matches!(
                self.xo & 0x1FF,
                8 | 9
                    | 10
                    | 11
                    | 40
                    | 73
                    | 75
                    | 104
                    | 136
                    | 138
                    | 200
                    | 202
                    | 232
                    | 233
                    | 234
                    | 235
                    | 266
                    | 457
                    | 459
                    | 489
                    | 491
            )
    }

    /// Whether the word reads GPR `r`.
    fn reads_gpr(self, r: u8) -> bool {
        if self.access().is_some() {
            let base = self.ra != 0 && self.ra == r;
            // A floating-point store's RS field names an FPR.
            let gpr_source = self.is_store() && !matches!(self.op, 52..=55);
            return base || (gpr_source && self.rt == r);
        }
        match self.op {
            7 | 8 | 10..=15 => self.ra == r && !(matches!(self.op, 14 | 15) && self.ra == 0),
            // [PPC-Book1 p:75 s:3.3.12] rlwnm rotates RS by the count in RB.
            23 => self.rt == r || self.rb == r,
            // [PPC-Book1 p:76 s:3.3.12] rldimi merges into the old RA;
            // [PPC-Book1 p:74 s:3.3.12] rldcl and [PPC-Book1 p:75 s:3.3.12]
            // rldcr rotate by RB.
            30 => match (self.word >> 2) & 7 {
                3 => self.rt == r || self.ra == r,
                4 => self.rt == r || self.rb == r,
                _ => self.rt == r,
            },
            20..=30 => self.rt == r || (self.op == 20 && self.ra == r),
            // [PPC-Book1 p:52 s:3.3.8] An XO-form arithmetic writes RT from
            // RA, and from RB where the form has one.
            31 if self.is_xo_arithmetic() => {
                let no_rb = matches!(self.xo & 0x1FF, 104 | 200 | 202 | 232 | 234);
                self.ra == r || (!no_rb && self.rb == r)
            }
            // [PPC-Book1 p:67] An X-form logical writes RA from RS and RB;
            // a shift-immediate or unary form's RB field is no register.
            31 if self.is_x_logical() => {
                let no_rb = matches!(self.xo, 26 | 58 | 824 | 826 | 827 | 922 | 954 | 986);
                self.rt == r || (!no_rb && self.rb == r)
            }
            // The indexed loads write RT and read (RA|0) and RB.
            31 if matches!(self.xo, 21 | 23 | 87 | 279 | 341 | 343) => {
                (self.ra != 0 && self.ra == r) || self.rb == r
            }
            // mfcr, mfspr and mftb read no GPR.
            31 if matches!(self.xo, 19 | 339 | 371) => false,
            // [PPC-Book1 p:81 s:3.3.13] mtspr reads RS, and its SPR number
            // fills the RA and RB slots; [PPC-Book1 p:83 s:3.3.13] mtcrf
            // reads RS, its FXM mask in those slots.
            31 if matches!(self.xo, 144 | 467) => self.rt == r,
            // [PPC-Book1 p:60 s:3.3.9] cmp and cmpl read RA and RB; the RT
            // slot holds BF and L.
            31 if matches!(self.xo, 0 | 32) => self.ra == r || self.rb == r,
            // The rest of opcode 31 counts as reading every register field
            // it names.
            31 => self.rt == r || self.ra == r || self.rb == r,
            _ => false,
        }
    }
}

/// The class of the BO field with its hint bits cleared.
///
/// [PPC-Book1 p:20 s:2.4.1 Branch Instructions Figure 21] BO field
/// encodings: `001at` and `011at` carry two hint bits, `1a00t` and
/// `1a01t` carry two, `0000z` style forms carry one ignored bit, and
/// `1z1zz` branches always.
fn bo_class(bo: u8) -> u8 {
    match (bo & 0x10 != 0, bo & 0x04 != 0) {
        (false, true) => bo & 0b11100,
        (false, false) => bo & 0b11110,
        (true, false) => bo & 0b10110,
        (true, true) => 0b10100,
    }
}

/// The name a class keys on: the variant, or for a variant that holds a
/// whole family of operations, the operation's mnemonic.
fn family_member(insn: &PpuInstruction) -> &'static str {
    match *insn {
        PpuInstruction::Fp59 { op, .. } => op.into(),
        PpuInstruction::Fp63 { op, .. } => op.into(),
        PpuInstruction::Vx { op, .. } => op.into(),
        PpuInstruction::Va { op, .. } => op.into(),
        _ => insn.into(),
    }
}

impl InsnClass {
    /// The class of `word`, or `None` when it has no decode.
    pub fn of(word: u32) -> Option<Self> {
        let insn = decode::decode(word).ok()?;
        Some(Self::from_parts(word, &insn))
    }

    fn from_parts(word: u32, insn: &PpuInstruction) -> Self {
        let f = Fields::of(word);
        let mut class = Self {
            name: family_member(insn),
            base: None,
            imm_zero: None,
            rc: None,
            same_reg: None,
            bo: None,
            aa: None,
            lk: None,
        };
        if f.access().is_some() {
            class.base = Some(BaseClass::of(f.ra));
        }
        if matches!(f.op, 7 | 8 | 10..=15 | 24..=29) {
            class.imm_zero = Some(word & 0xFFFF == 0);
        }
        if matches!(f.op, 20 | 21 | 23 | 30) || f.is_x_logical() || f.is_xo_arithmetic() {
            class.rc = Some(word & 1 != 0);
        }
        match *insn {
            // [PPC-Book1 p:119 s:4.6.7] The floating compares reserve raw
            // bit 31; [PPC-Book1 p:120 s:4.6.8] so does mcrfs.
            PpuInstruction::Fp63 {
                op: Fp63Op::Fcmpu | Fp63Op::Fcmpo | Fp63Op::Mcrfs,
                ..
            } => {}
            PpuInstruction::Fp63 { rc, .. } | PpuInstruction::Fp59 { rc, .. } => {
                class.rc = Some(rc);
            }
            // Only the vector compares carry an Rc bit.
            PpuInstruction::Vx { op, rc, .. } if op.is_vxr_compare() => class.rc = Some(rc),
            _ => {}
        }
        match f.op {
            16 => {
                class.bo = Some(bo_class(f.rt));
                class.aa = Some(word & 2 != 0);
                class.lk = Some(word & 1 != 0);
            }
            18 => {
                class.aa = Some(word & 2 != 0);
                class.lk = Some(word & 1 != 0);
            }
            19 if f.is_branch() => {
                class.bo = Some(bo_class(f.rt));
                class.lk = Some(word & 1 != 0);
            }
            17 | 19 => {}
            _ => class.same_reg = Some(f.rt == f.ra),
        }
        class
    }

    /// The class as one line: the variant, then each class it has.
    pub fn label(&self) -> String {
        let mut out = String::from(self.name);
        let flag = |b: bool| if b { "1" } else { "0" };
        if let Some(base) = self.base {
            out.push_str(" base=");
            out.push_str(base.label());
        }
        if let Some(zero) = self.imm_zero {
            out.push_str(if zero { " imm=0" } else { " imm=nz" });
        }
        if let Some(rc) = self.rc {
            out.push_str(" rc=");
            out.push_str(flag(rc));
        }
        if let Some(same) = self.same_reg {
            out.push_str(if same { " same" } else { " distinct" });
        }
        if let Some(bo) = self.bo {
            out.push_str(&format!(" bo={bo:05b}"));
        }
        if let Some(aa) = self.aa {
            out.push_str(" aa=");
            out.push_str(flag(aa));
        }
        if let Some(lk) = self.lk {
            out.push_str(" lk=");
            out.push_str(flag(lk));
        }
        out
    }
}

impl PairClass {
    /// The class of `first` followed by `second`, or `None` when either
    /// has no decode.
    pub fn of(first: u32, second: u32) -> Option<Self> {
        let (a, b) = (Fields::of(first), Fields::of(second));
        let dependent = a.gpr_target().is_some_and(|t| b.reads_gpr(t));
        let (shared_base, adjacent) = match (a.access(), b.access()) {
            (Some((size, d1)), Some((_, d2))) if a.ra == b.ra => {
                (true, i32::from(d2) == i32::from(d1) + i32::from(size))
            }
            _ => (false, false),
        };
        Some(Self {
            first: InsnClass::of(first)?,
            second: InsnClass::of(second)?,
            dependent,
            shared_base,
            adjacent,
        })
    }

    /// The pair as one line: both classes, then each shape it has.
    pub fn label(&self) -> String {
        let mut out = format!("{} ; {}", self.first.label(), self.second.label());
        if self.dependent {
            out.push_str(" | dependent");
        }
        if self.shared_base {
            out.push_str(" | shared-base");
        }
        if self.adjacent {
            out.push_str(" | adjacent");
        }
        out
    }
}
