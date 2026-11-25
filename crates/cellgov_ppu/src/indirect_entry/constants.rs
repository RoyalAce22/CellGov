//! The constants that the code builds in its GPRs, LR, SRR0 and SRR1 on
//! one straight-line path, and the words that end such a path.

use std::collections::BTreeSet;
use std::rc::Rc;

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_IR, MSR_PR, MSR_SF};

use super::real_mode::{fetch_address, MsrWord, Switch};
use crate::caller_census::written_gprs;
use crate::instruction::PpuInstruction;
use crate::local_cfg::falls_through;

/// The constants that a register can hold on a straight-line path.
///
/// A register holds more than one constant only when the path builds it
/// from r2 on a word where r2 holds more than one: a word that functions
/// with two different TOCs reach, or a word whose TOC differs from the
/// constant that the path leaves in r2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Value {
    /// One constant.
    One(u64),
    /// The constants, in ascending order, when there are none or two or
    /// more.
    Many(Rc<[u64]>),
}

impl Value {
    /// The value that holds each constant of `values`.
    ///
    /// A value from an empty `values` holds no constant.
    pub(super) fn sorted(mut values: Vec<u64>) -> Self {
        values.sort_unstable();
        values.dedup();
        match *values.as_slice() {
            [one] => Self::One(one),
            _ => Self::Many(values.into()),
        }
    }

    /// The constants, in ascending order.
    pub(super) fn constants(&self) -> &[u64] {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Many(many) => many,
        }
    }

    /// The value that holds `step` of each constant.
    pub(super) fn map(&self, step: impl Fn(u64) -> u64) -> Self {
        match self {
            Self::One(one) => Self::One(step(*one)),
            Self::Many(many) => Self::sorted(many.iter().map(|&constant| step(constant)).collect()),
        }
    }

    /// The value that holds each constant of `self` and of `other`.
    pub(super) fn union(self, other: &Self) -> Self {
        if self.holds_all(other) {
            self
        } else if other.holds_all(&self) {
            other.clone()
        } else {
            Self::sorted([self.constants(), other.constants()].concat())
        }
    }

    /// Whether `self` holds each constant of `other`.
    pub(super) fn holds_all(&self, other: &Self) -> bool {
        if let (Self::Many(one), Self::Many(two)) = (self, other) {
            if Rc::ptr_eq(one, two) {
                return true;
            }
        }
        other
            .constants()
            .iter()
            .all(|constant| self.constants().binary_search(constant).is_ok())
    }
}

/// The constants that the code builds in its GPRs, LR, SRR0 and SRR1 on
/// one straight-line path.
#[derive(Debug, Default)]
pub(super) struct Constants {
    pub(super) gprs: [Option<Value>; 32],
    pub(super) lr: Option<Written>,
    srr0: Option<Value>,
    srr1: Option<Value>,
}

/// A constant in LR.
#[derive(Debug, Clone)]
pub(super) struct Written {
    pub(super) value: Value,
    /// The address of the word that wrote the constant.
    pub(super) by: u64,
}

impl Constants {
    /// Steps over the word `raw` at `address`, and returns the constants
    /// it writes into a GPR.
    ///
    /// A word that does not fall through ends the straight-line path, so
    /// the step forgets every constant.
    pub(super) fn step(&mut self, address: u64, raw: u32) -> Option<Value> {
        let Ok(decoded) = crate::decode::decode(raw) else {
            *self = Self::default();
            return None;
        };
        if let Some((gpr, value)) = self.written(decoded) {
            self.gprs[usize::from(gpr)].clone_from(&value);
            return value;
        }
        match decoded {
            PpuInstruction::Mtlr { rs } => self.lr = self.written_by(rs, address),
            PpuInstruction::B { link, .. } => self.after_branch(address, 0b10100, link),
            PpuInstruction::Bc { bo, link, .. }
            | PpuInstruction::Bclr { bo, link, .. }
            | PpuInstruction::Bcctr { bo, link, .. } => self.after_branch(address, bo, link),
            _ if never_falls_through(&decoded) => *self = Self::default(),
            PpuInstruction::Sc { .. } => {
                self.forget_srrs();
                self.forget_written(&decoded);
            }
            PpuInstruction::PpePrivileged { raw } => match MsrWord::of(raw) {
                Some(MsrWord::MtSrr0 { rs }) => self.srr0 = self.gpr(rs).cloned(),
                Some(MsrWord::MtSrr1 { rs }) => self.srr1 = self.gpr(rs).cloned(),
                _ => self.forget_written(&decoded),
            },
            _ => self.forget_written(&decoded),
        }
        None
    }

    /// Forgets the constants in SRR0 and SRR1.
    ///
    /// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
    fn forget_srrs(&mut self) {
        self.srr0 = None;
        self.srr1 = None;
    }

    /// Where the word `raw` at `address` can put control into real mode.
    ///
    /// These words give no address:
    /// - an `mtmsr` or `mtmsrd` with L = 1, which sets only MSR\[EE\]
    ///   and MSR\[RI\];
    /// - an `hrfid`, because the scan does not follow HSRR0 and HSRR1.
    ///
    /// The address depends on the MSR value only through MSR\[SF\] and
    /// MSR bit 1, so the step uses one MSR value of each kind. An MSR
    /// value that the scan does not know can clear IR and PR with SF at 0
    /// or at 1.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0 under the new MSR.
    /// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
    /// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and leaves MSR bits 0:31 unchanged.
    pub(super) fn real_mode_switch(&self, address: u64, raw: u32) -> Switch {
        let Some((msr, next, sets_sf)) = self.msr_write(address, raw) else {
            return Switch::None;
        };
        let kinds: BTreeSet<u64> = match msr {
            Some(msr) => msr
                .constants()
                .iter()
                .filter(|&&msr| msr & (MSR_IR | MSR_PR) == 0)
                .map(|&msr| msr & (MSR_SF | MSR_BIT1))
                .collect(),
            None => BTreeSet::from([0, MSR_SF]),
        };
        if kinds.is_empty() {
            return Switch::None;
        }
        let Some(next) = next else {
            return Switch::Unbounded;
        };
        Switch::To(
            kinds
                .iter()
                .flat_map(|&msr| {
                    next.constants().iter().map(move |&next| {
                        if sets_sf {
                            fetch_address(next, msr)
                        } else {
                            next
                        }
                    })
                })
                .collect(),
        )
    }

    /// What the word `raw` at `address` does to MSR and to the next fetch.
    ///
    /// The result is `None` for a word that does not set MSR\[IR\]. Else
    /// it holds:
    /// - the MSR constants that the word sets MSR\[IR\] from;
    /// - the address of the next fetch;
    /// - whether the word also sets MSR\[SF\].
    ///
    /// The MSR constants or the address are `None` when the scan does
    /// not know them.
    fn msr_write(&self, address: u64, raw: u32) -> Option<(Option<&Value>, Option<Value>, bool)> {
        let next = || address.checked_add(4).map(Value::One);
        Some(match MsrWord::of(raw)? {
            MsrWord::Rfid => (
                self.srr1.as_ref(),
                self.srr0.as_ref().map(|srr0| srr0.map(|srr0| srr0 & !0b11)),
                true,
            ),
            MsrWord::Mtmsrd { rs } => (self.gpr(rs), next(), true),
            MsrWord::Mtmsr { rs } => (self.gpr(rs), next(), false),
            MsrWord::MtSrr0 { .. } | MsrWord::MtSrr1 { .. } => return None,
        })
    }

    /// Forgets the constant in each GPR that `decoded` writes.
    fn forget_written(&mut self, decoded: &PpuInstruction) {
        let written = written_gprs(decoded);
        for (gpr, value) in self.gprs.iter_mut().enumerate() {
            if written >> gpr & 1 == 1 {
                *value = None;
            }
        }
    }

    /// The GPR that `decoded` writes and the constant it writes there,
    /// for a word that builds a constant.
    ///
    /// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
    fn written(&self, decoded: PpuInstruction) -> Option<(u8, Option<Value>)> {
        if let PpuInstruction::Mflr { rt } = decoded {
            return Some((rt, self.lr.as_ref().map(|lr| lr.value.clone())));
        }
        let GprStep { rt, source, step } = GprStep::of(decoded)?;
        let value = match source {
            Some(gpr) => self
                .gpr(gpr)
                .map(|value| value.map(|value| step.apply(value))),
            None => Some(Value::One(step.apply(0))),
        };
        Some((rt, value))
    }

    pub(super) fn gpr(&self, gpr: u8) -> Option<&Value> {
        self.gprs.get(usize::from(gpr))?.as_ref()
    }

    /// The constants in `gpr`, which the word at `address` copies into LR.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
    fn written_by(&self, gpr: u8, address: u64) -> Option<Written> {
        self.gpr(gpr).map(|value| Written {
            value: value.clone(),
            by: address,
        })
    }

    /// Steps over a branch at `address` with the BO field `bo`, which
    /// writes LR when `link` is set.
    ///
    /// The routine that a branch with `link` calls can write SRR0 and
    /// SRR1, so the step forgets them.
    ///
    /// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
    fn after_branch(&mut self, address: u64, bo: u8, link: bool) {
        if !falls_through(bo, link) {
            *self = Self::default();
            return;
        }
        if link {
            self.forget_srrs();
            self.lr = address.checked_add(4).map(|value| Written {
                value: Value::One(value),
                by: address,
            });
        }
    }
}

/// A word that the scan follows a constant through: it writes `rt`
/// from one source GPR, or from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GprStep {
    pub(super) rt: u8,
    /// The source GPR, or `None` when the word steps 0.
    pub(super) source: Option<u8>,
    pub(super) step: Step,
}

/// What a [`GprStep`] does to the value of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// Adds the value.
    Add(i64),
    /// ORs the value in.
    Or(u64),
    /// Rotates left by `sh` bits, then ANDs with `mask`.
    RotateAnd { sh: u32, mask: u64 },
}

impl GprStep {
    /// The step of `decoded`, or `None` for a word that the scan does not
    /// follow a constant through.
    ///
    /// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
    /// [PPC-Book1 p:66 s:3.3.11] `ori` and `oris` OR RS with UI or UI || 0x0000 and place the result into RA.
    /// [PPC-Book1 p:67 s:3.3.11] `or` ORs RS with RB and places the result into RA.
    /// [PPC-Book1 p:72 s:3.3.12.1] `rldicl` and `rldicr` rotate RS left by SH and AND it with MASK(MB, 63) or MASK(0, ME).
    pub(super) fn of(decoded: PpuInstruction) -> Option<Self> {
        let base = |ra: u8| (ra != 0).then_some(ra);
        let (rt, source, step) = match decoded {
            PpuInstruction::Addi { rt, ra, imm } => (rt, base(ra), Step::Add(i64::from(imm))),
            PpuInstruction::Addis { rt, ra, imm } => {
                (rt, base(ra), Step::Add(i64::from(imm) << 16))
            }
            PpuInstruction::Ori { ra, rs, imm } => (ra, Some(rs), Step::Or(u64::from(imm))),
            PpuInstruction::Oris { ra, rs, imm } => (ra, Some(rs), Step::Or(u64::from(imm) << 16)),
            PpuInstruction::Rldicl { ra, rs, sh, mb, .. } => (
                ra,
                Some(rs),
                Step::RotateAnd {
                    sh: u32::from(sh),
                    mask: u64::MAX >> mb,
                },
            ),
            PpuInstruction::Rldicr { ra, rs, sh, me, .. } => (
                ra,
                Some(rs),
                Step::RotateAnd {
                    sh: u32::from(sh),
                    mask: u64::MAX << (63 - me),
                },
            ),
            PpuInstruction::Or { ra, rs, rb, .. } if rs == rb => (ra, Some(rs), Step::Or(0)),
            _ => return None,
        };
        Some(Self { rt, source, step })
    }
}

impl Step {
    pub(super) fn apply(self, value: u64) -> u64 {
        match self {
            Self::Add(add) => value.wrapping_add_signed(add),
            Self::Or(or) => value | or,
            Self::RotateAnd { sh, mask } => value.rotate_left(sh) & mask,
        }
    }
}

/// Whether control never goes from `decoded` to the next word, for a
/// word that is not a branch. These words never fall through:
/// - `rfid` and `hrfid`;
/// - a trap that traps for all operand values.
///
/// The bound errs wide, so a word that causes an exception which a
/// handler can emulate keeps the path. A word that the PPE does not
/// implement is such a word.
///
/// An `rfid` with SRR0 at the next word falls through. The scan forgets
/// each constant there. That loses no named word, because no image read
/// shows a constant that crosses an `rfid`.
///
/// [PPC-Book3 p:119 s:Appendix G] `rfid` and `hrfid` are XL-form words with primary opcode 19 and extended opcodes 18 and 274.
/// [PPC-Book3 p:13 s:2.3] `rfid` and `hrfid` fetch the next instruction from SRR0 or HSRR0.
/// [OFW-4.93 lv2_kernel a:0x8000000000000720-0x8000000000000744] This `rfid` returns to the next word, which r5 names, and the words after it build no constant.
/// [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement `popcntb`.
/// [CBE-Handbook p:739 s:A.2.4.1] An attempt to execute `popcntb`, `mcrxr` or another word that the PPE does not implement causes an illegal instruction exception.
/// [PPC-Book3 p:92 s:8.4] An `lq` with RT = RA invokes the illegal instruction error handler.
pub(super) fn never_falls_through(decoded: &PpuInstruction) -> bool {
    match *decoded {
        PpuInstruction::PpePrivileged { raw } => {
            matches!((raw >> 26, (raw >> 1) & 0x3FF), (19, 18 | 274))
        }
        PpuInstruction::Tw { to, ra, rb } | PpuInstruction::Td { to, ra, rb } => {
            traps_always(to, ra == rb)
        }
        PpuInstruction::Twi { to, .. } | PpuInstruction::Tdi { to, .. } => traps_always(to, false),
        _ => false,
    }
}

/// Whether a trap with the TO field `to` traps for all operand values.
///
/// `same_operands` is set when the trap compares a register with itself.
/// Two operands are equal, or else each comparison puts them in one
/// order. So the trap traps always when TO has the equal bit and one of
/// these is true:
/// - the operands are the same register;
/// - TO has both signed order bits;
/// - TO has both unsigned order bits.
///
/// [PPC-Book1 p:62 s:3.3.10] TO bits 0 to 4 select signed less than, signed greater than, equal, unsigned less than and unsigned greater than.
fn traps_always(to: u8, same_operands: bool) -> bool {
    let equal = to & 0b00100 != 0;
    let signed_orders = to & 0b11000 == 0b11000;
    let unsigned_orders = to & 0b00011 == 0b00011;
    equal && (same_operands || signed_orders || unsigned_orders)
}
