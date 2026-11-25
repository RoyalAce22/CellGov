//! The constants that the code builds in its GPRs, LR, SRR0 and SRR1 on
//! one path or on the paths that join at a word, and the words that end
//! a path.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BIT1, MSR_IR, MSR_PR, MSR_SF, SPR_SRR0, SPR_SRR1};

use super::lost::{join_origins, LostGprs, Origins};
use super::real_mode::{fetch_address, MsrWord, Switch};
use super::toc::FunctionToc;
use crate::caller_census::{surely_written_gprs, written_gprs};
use crate::instruction::{spr_number, PpuInstruction};
use crate::local_cfg::falls_through;

/// The constants that a register can hold on the paths to a word.
///
/// A register holds more than one constant only in these cases:
/// - a path builds it from r2 on a word where r2 holds more than one: a
///   word that functions with two different TOCs reach, or a word whose
///   TOC differs from the constant that the path leaves in r2;
/// - paths that bring different constants join at a branch target.
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
/// one path, or on the paths that join at a word.
///
/// A register that can also hold a constant that the scan dropped is
/// lost. Its mark holds the origins of the dropped constants. A word
/// that the scan does not follow can compute a GPR from a lost
/// GPR, or move a lost GPR through CR or an SPR. That GPR is lost too,
/// and its mark can hold any value that a word computed from a dropped
/// constant.
///
/// An LR that the scan does not know holds the return point of a call,
/// because the bound takes each such `bclr` to return there. A routine can
/// also step a copy of that LR and return to the result, past the return
/// point. An arithmetic, logical or rotate word, or the address update of
/// a load or a store, can compute a GPR from such a copy. That GPR is
/// lost, unless the word copies the value unchanged. A GPR that such a
/// word computes from a lost GPR of this kind is also lost. The usual save
/// and restore of LR stays a plain copy of the return point.
#[derive(Debug, Default)]
pub(super) struct Constants {
    pub(super) gprs: [Option<Value>; 32],
    /// The origins of each lost GPR.
    pub(super) lost_gprs: LostGprs,
    /// The constants in LR, by the address of the word that wrote each
    /// one.
    pub(super) lr: BTreeMap<u64, Value>,
    /// The origins of LR when it is lost.
    pub(super) lr_lost: Option<Origins>,
    /// Whether LR can also hold a value that the scan does not know, when
    /// [`Self::lr`] holds a constant.
    pub(super) lr_unknown: bool,
    /// The GPRs that hold an unstepped copy of an LR that the scan does not
    /// know, as a mask with bit `n` set for GPR `n`.
    pub(super) lr_copies: u32,
    /// The GPRs that hold a stepped copy of an LR that the scan does not
    /// know, as a mask with bit `n` set for GPR `n`.
    ///
    /// A stepped copy is a value that a word computes from such a copy,
    /// other than an unchanged copy. Each of these GPRs is lost.
    pub(super) lr_stepped: u32,
    /// The GPRs that can also hold, on a path, a value for which the scan
    /// keeps no constant and no lost mark.
    ///
    /// The mask has bit `n` set for GPR `n`. A GPR with no constant and no
    /// mark holds such a value, whatever its bit. A copy or a step of the
    /// GPR keeps its bit. Such a value can be a dropped constant that a
    /// word the scan does not follow stepped. So the lost mark of such a
    /// GPR keeps no single step, as [`Self::forget_unheld_steps`] gives it.
    ///
    /// A join or an indirect entry brings such a value beside the
    /// constants and the mark of a GPR. These can also leave one in a GPR:
    /// - the routine of a call;
    /// - an `sc`;
    /// - a word that can write a GPR and that the scan does not follow.
    pub(super) unheld: u32,
    /// Whether LR can also hold, on a path, a value for which the scan
    /// keeps no constant and no lost mark.
    pub(super) lr_unheld: bool,
    srr0: Option<Value>,
    srr1: Option<Value>,
    /// The origins of SRR0 when it is lost.
    pub(super) srr0_lost: Option<Origins>,
    /// The origins of SRR1 when it is lost.
    pub(super) srr1_lost: Option<Origins>,
    /// The origins of a lost value that a word moved into CR or into an
    /// SPR that [`spr_move`] names.
    ///
    /// The scan keeps one mark for all of these registers, so a move out
    /// of any of them takes it, also through an SPR alias. The mark is
    /// stepped, because a move can take only a part of the value.
    pub(super) moved_lost: Option<Origins>,
}

impl Constants {
    /// Steps over the word `raw` at `address`, and returns the constants
    /// it writes into a GPR.
    ///
    /// A word that does not fall through ends the path, so the step
    /// forgets every constant.
    pub(super) fn step(&mut self, address: u64, raw: u32) -> Option<Value> {
        let Ok(decoded) = crate::decode::decode(raw) else {
            *self = Self::default();
            return None;
        };
        let lr_copy = self.lr_copy_write(&decoded);
        let value = self.step_decoded(address, decoded);
        self.after_lr_copy_write(&decoded, lr_copy);
        self.forget_unheld_steps();
        value
    }

    /// Makes the lost mark of each GPR in [`Self::unheld`], and of LR when
    /// [`Self::lr_is_unheld`] holds, keep no single step.
    ///
    /// [`Origins::without_one_step`] gives the new mark.
    fn forget_unheld_steps(&mut self) {
        self.lost_gprs.forget_steps(self.unheld);
        if self.lr_is_unheld() {
            if let Some(origins) = &mut self.lr_lost {
                *origins = origins.without_one_step();
            }
        }
    }

    /// Steps over the decoded word `decoded` at `address`, and returns the
    /// constants it writes into a GPR.
    fn step_decoded(&mut self, address: u64, decoded: PpuInstruction) -> Option<Value> {
        if let Some((gpr, value)) = self.written(decoded) {
            let lost = self.copied_lost(decoded);
            let unheld = self.written_unheld(decoded);
            self.lost_gprs.set(gpr, lost);
            self.unheld = (self.unheld & !gpr_bit(gpr)) | if unheld { gpr_bit(gpr) } else { 0 };
            self.gprs[usize::from(gpr)].clone_from(&value);
            return value;
        }
        match decoded {
            PpuInstruction::Mtlr { rs } => {
                self.lr = self.written_by(rs, address);
                self.lr_lost = self.lost(rs).cloned();
                self.lr_unknown = self.lr_copies & gpr_bit(rs) != 0;
                self.lr_unheld = self.is_unheld(rs);
            }
            PpuInstruction::B { link, .. } => self.after_branch(address, 0b10100, link),
            PpuInstruction::Bc { bo, link, .. }
            | PpuInstruction::Bclr { bo, link, .. }
            | PpuInstruction::Bcctr { bo, link, .. } => self.after_branch(address, bo, link),
            _ if never_falls_through(&decoded) => *self = Self::default(),
            PpuInstruction::Sc { .. } => {
                self.forget_srrs();
                self.forget_written(&decoded);
            }
            PpuInstruction::PpePrivileged { raw } => self.step_privileged(raw, &decoded),
            _ => self.step_unfollowed(&decoded),
        }
        None
    }

    /// Steps over `decoded`, a word that the scan follows no constant
    /// through, and marks the GPR that it writes as lost when the value can
    /// come from a lost GPR.
    ///
    /// The word gives these origins:
    /// - a move into CR or into an SPR of [`spr_move`] joins the origins of
    ///   its source GPRs into [`Self::moved_lost`];
    /// - a move out of such a register gives [`Self::moved_lost`];
    /// - each word of [`computed_from`] gives the origins of its lost
    ///   sources, stepped.
    ///
    /// A GPR that the word does not surely write keeps its own mark too.
    fn step_unfollowed(&mut self, decoded: &PpuInstruction) {
        let lost = match spr_move(decoded) {
            Some(SprMove::Into(sources)) => {
                self.move_into_spr(sources);
                None
            }
            Some(SprMove::OutOf(rt)) => self.moved_lost.clone().map(|origins| (rt, origins)),
            None => self.computed_lost(decoded),
        };
        self.forget_written(decoded);
        if let Some((rt, origins)) = lost {
            self.lost_gprs.join_gpr(rt, &origins);
        }
    }

    /// Joins into [`Self::moved_lost`] the origins of each GPR of
    /// `sources`, a mask with bit `n` set for GPR `n`.
    ///
    /// A copy of an LR that the scan does not know joins a mark that names
    /// no drop: the moved value need not be the return point.
    fn move_into_spr(&mut self, sources: u32) {
        let mut origins = None;
        for gpr in gprs_of(sources) {
            join_origins(&mut origins, self.lost(gpr));
            if (self.lr_copies | self.lr_stepped) & gpr_bit(gpr) != 0 {
                join_origins(&mut origins, Some(&Origins::unnamed()));
            }
        }
        if let Some(origins) = origins {
            join_origins(&mut self.moved_lost, Some(&origins.stepped()));
        }
    }

    /// The GPR that the word `decoded` of [`computed_from`] writes and its
    /// origins, when a source of the word is lost.
    ///
    /// The origins are those of each lost source, stepped. So a mark that
    /// keeps one progression loses it, and a mark of TOCs alone becomes a
    /// mark of any value.
    fn computed_lost(&self, decoded: &PpuInstruction) -> Option<(u8, Origins)> {
        let (rt, sources) = computed_from(decoded)?;
        let mut origins = None;
        for gpr in gprs_of(sources) {
            join_origins(&mut origins, self.lost(gpr));
        }
        origins.map(|origins| (rt, origins.stepped()))
    }

    /// Steps over the privileged word `raw`, and copies a lost mark into or out of SRR0 or SRR1.
    fn step_privileged(&mut self, raw: u32, decoded: &PpuInstruction) {
        match MsrWord::of(raw) {
            Some(MsrWord::MtSrr0 { rs }) => {
                self.srr0 = self.gpr(rs).cloned();
                self.srr0_lost = self.lost(rs).cloned();
            }
            Some(MsrWord::MtSrr1 { rs }) => {
                self.srr1 = self.gpr(rs).cloned();
                self.srr1_lost = self.lost(rs).cloned();
            }
            _ => {
                let lost = self.lost_srr_copy(raw);
                self.step_unfollowed(decoded);
                if let Some((rt, origins)) = lost {
                    self.lost_gprs.set(rt, Some(origins));
                }
            }
        }
    }

    /// Adds to r2 the TOCs that `toc` gives a word.
    ///
    /// `every` holds each TOC that a descriptor gives.
    ///
    /// On a word that a function reaches, r2 holds a TOC or a constant of
    /// the paths. So r2 then holds no value for which the scan keeps no
    /// constant and no lost mark. The lost mark of each register
    /// that can hold such a value then keeps no single step, as
    /// [`Self::forget_unheld_steps`] gives it.
    pub(super) fn enter_toc(&mut self, toc: FunctionToc, every: &Value) {
        self.gprs[2] = toc.in_r2(self.gprs[2].take(), every);
        if toc != FunctionToc::Outside {
            self.unheld &= !gpr_bit(2);
        }
        self.forget_unheld_steps();
    }

    /// Forgets the constants and the lost marks of SRR0 and SRR1.
    ///
    /// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt sets SRR0 and SRR1.
    fn forget_srrs(&mut self) {
        self.srr0 = None;
        self.srr1 = None;
        self.srr0_lost = None;
        self.srr1_lost = None;
    }

    /// The GPR that the word `raw` copies SRR0 or SRR1 into and the
    /// origins of that register, when that register is lost.
    ///
    /// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, and it places the SPR that its SPR field names into RT.
    fn lost_srr_copy(&self, raw: u32) -> Option<(u8, Origins)> {
        if (raw >> 26, (raw >> 1) & 0x3FF) != (31, 339) {
            return None;
        }
        let lost = match spr_number(raw) {
            SPR_SRR0 => self.srr0_lost.as_ref(),
            SPR_SRR1 => self.srr1_lost.as_ref(),
            _ => None,
        };
        lost.map(|origins| (((raw >> 21) & 0x1F) as u8, origins.clone()))
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
        let kinds = fetch_kinds(msr, false);
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

    /// The words that the word `raw` can enter with MSR\[IR\] at 1, when it
    /// is an `rfid` and the scan knows SRR0.
    ///
    /// The fetch address depends on MSR\[SF\] of the new MSR, which `rfid`
    /// sets from SRR1 bits 0 and 1. So the set holds the address of each
    /// SRR0 constant under each MSR\[SF\] that SRR1 can give. An SRR1 that
    /// the scan does not know can set IR with SF at 0 or at 1. An SRR1
    /// that can hold a constant that the scan dropped can do so too.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
    /// [PPC-Book3 p:13 s:2.3] `rfid` fetches from SRR0 0:61 || 0b00 when SF = 1 in the new MSR, and from 32 zeros || SRR0 32:61 || 0b00 when SF = 0.
    pub(super) fn translated_rfid_targets(&self, raw: u32) -> BTreeSet<u64> {
        let (Some(MsrWord::Rfid), Some(srr0)) = (MsrWord::of(raw), &self.srr0) else {
            return BTreeSet::new();
        };
        fetch_kinds(self.known_srr1(), true)
            .into_iter()
            .flat_map(|msr| {
                srr0.constants()
                    .iter()
                    .map(move |&srr0| fetch_address(srr0 & !0b11, msr))
            })
            .collect()
    }

    /// The origins of SRR0 when the word `raw` is an `rfid` that can
    /// leave MSR\[IR\] at 1 with a lost SRR0.
    ///
    /// A lost SRR0 can hold a constant that the scan dropped. The image
    /// need not name that constant, so the `rfid` can enter a word that
    /// the bound does not hold, with MSR\[SF\] at 0 or at 1.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0 under the new MSR.
    pub(super) fn translated_rfid_lost(&self, raw: u32) -> Option<&Origins> {
        if MsrWord::of(raw) != Some(MsrWord::Rfid)
            || fetch_kinds(self.known_srr1(), true).is_empty()
        {
            return None;
        }
        self.srr0_lost.as_ref()
    }

    /// The SRR1 constants, or `None` when SRR1 can also hold a constant
    /// that the scan dropped.
    fn known_srr1(&self) -> Option<&Value> {
        self.srr1.as_ref().filter(|_| self.srr1_lost.is_none())
    }

    /// Where the word `raw` at `address` can put control into 32-bit
    /// mode with MSR\[IR\] at `ir`.
    ///
    /// In 32-bit mode, MSR\[SF\] and MSR bit 1 are 0. An `mtmsr` leaves
    /// both bits as they are, so it gives no address.
    ///
    /// [PPC-Book3 p:13 s:2.3] `rfid` places the OR of SRR1 bits 0 and 1 into MSR\[SF\].
    /// [PPC-Book3 p:22 s:3.4] `mtmsrd` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and MSR\[SF\] to RS bit 0 | RS bit 1.
    /// [PPC-Book3 p:91 s:8.3] `mtmsr` with L = 0 sets MSR\[IR\] to RS\[IR\] | RS\[PR\] and leaves MSR bits 0:31 unchanged.
    pub(super) fn mode32_switch(&self, address: u64, raw: u32, ir: bool) -> Switch {
        let Some((msr, next, true)) = self.msr_write(address, raw) else {
            return Switch::None;
        };
        if !fetch_kinds(msr, ir).contains(&0) {
            return Switch::None;
        }
        let Some(next) = next else {
            return Switch::Unbounded;
        };
        Switch::To(
            next.constants()
                .iter()
                .map(|&next| fetch_address(next, 0))
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
                self.known_srr1(),
                self.srr0.as_ref().map(|srr0| srr0.map(|srr0| srr0 & !0b11)),
                true,
            ),
            MsrWord::Mtmsrd { rs } => (self.gpr(rs), next(), true),
            MsrWord::Mtmsr { rs } => (self.gpr(rs), next(), false),
            MsrWord::MtSrr0 { .. } | MsrWord::MtSrr1 { .. } => return None,
        })
    }

    /// Forgets the constant in each GPR that `decoded` writes.
    ///
    /// A GPR that the word does not surely write can keep a constant that
    /// the scan dropped, so it keeps its mark.
    fn forget_written(&mut self, decoded: &PpuInstruction) {
        let mut bits = written_gprs(decoded);
        self.unheld |= bits;
        while bits != 0 {
            self.gprs[bits.trailing_zeros() as usize] = None;
            bits &= bits - 1;
        }
        self.lost_gprs.forget(surely_written_gprs(decoded));
    }

    /// Whether `gpr` can also hold a constant that the scan dropped.
    #[cfg(test)]
    pub(super) fn is_lost(&self, gpr: u8) -> bool {
        self.lost(gpr).is_some()
    }

    /// The origins of `gpr` when it can also hold a constant that the
    /// scan dropped.
    pub(super) fn lost(&self, gpr: u8) -> Option<&Origins> {
        self.lost_gprs.get(gpr)
    }

    /// The origins of the GPR that `decoded` builds, when its source can
    /// hold a constant that the scan dropped.
    ///
    /// These steps give these origins:
    /// - a word that copies its source unchanged gives the origins of the
    ///   source;
    /// - an add of a constant gives them as [`Origins::added`] states;
    /// - any other step gives them stepped.
    fn copied_lost(&self, decoded: PpuInstruction) -> Option<Origins> {
        if let PpuInstruction::Mflr { .. } = decoded {
            return self.lr_lost.clone();
        }
        let step = GprStep::of(decoded)?;
        let origins = self.lost(step.source?)?;
        Some(match step.step {
            step if step.keeps_value() => origins.clone(),
            Step::Add(add) => origins.added(add),
            Step::Or(_) | Step::RotateAnd { .. } => origins.stepped(),
        })
    }

    /// Whether the GPR that `decoded` builds can hold a value for which
    /// the scan keeps no constant and no lost mark, as [`Self::unheld`]
    /// holds it.
    fn written_unheld(&self, decoded: PpuInstruction) -> bool {
        if let PpuInstruction::Mflr { .. } = decoded {
            return self.lr_is_unheld();
        }
        GprStep::of(decoded)
            .and_then(|step| step.source)
            .is_some_and(|source| self.is_unheld(source))
    }

    /// Whether `gpr` can hold a value for which the scan keeps no
    /// constant and no lost mark.
    fn is_unheld(&self, gpr: u8) -> bool {
        self.unheld & gpr_bit(gpr) != 0 || (self.gpr(gpr).is_none() && self.lost(gpr).is_none())
    }

    /// Whether LR can hold a value for which the scan keeps no constant
    /// and no lost mark.
    pub(super) fn lr_is_unheld(&self) -> bool {
        self.lr_unheld || self.lr_unknown || (self.lr.is_empty() && self.lr_lost.is_none())
    }

    /// The GPRs that can hold a value for which the scan keeps no
    /// constant and no lost mark, as a mask with bit `n` set for GPR `n`.
    pub(super) fn unheld_gprs(&self) -> u32 {
        (0..32u8)
            .filter(|&gpr| self.is_unheld(gpr))
            .fold(0, |mask, gpr| mask | gpr_bit(gpr))
    }

    /// Takes each GPR and LR to hold also a value for which the scan
    /// keeps no constant and no lost mark, as at a word that an indirect
    /// branch can enter from a path that the scan does not follow.
    pub(super) fn enter_unheld(&mut self) {
        self.unheld = u32::MAX;
        self.lr_unheld = true;
    }

    /// The GPR that `decoded` writes and the constant it writes there,
    /// for a word that builds a constant.
    ///
    /// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
    fn written(&self, decoded: PpuInstruction) -> Option<(u8, Option<Value>)> {
        if let PpuInstruction::Mflr { rt } = decoded {
            let lr = self.lr.values().cloned().reduce(|one, two| one.union(&two));
            return Some((rt, lr));
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
    fn written_by(&self, gpr: u8, address: u64) -> BTreeMap<u64, Value> {
        self.gpr(gpr)
            .map(|value| BTreeMap::from([(address, value.clone())]))
            .unwrap_or_default()
    }

    /// The GPR that `decoded` writes from LR or from a copy of an LR that
    /// the scan does not know, and how it writes it.
    ///
    /// A word that computes its result from a stepped copy steps it, even
    /// when the word gives its source unchanged.
    ///
    /// An `mflr` copies an LR that the scan does not know when one of these
    /// is true:
    /// - [`Self::lr`] holds no constant;
    /// - LR can also hold a value that the scan does not know.
    ///
    /// [PPC-Book1 p:82 s:3.3.13] `mflr`, a form of `mfspr`, copies LR into RT.
    fn lr_copy_write(&self, decoded: &PpuInstruction) -> Option<(u8, LrCopyWrite)> {
        if let PpuInstruction::Mflr { rt } = *decoded {
            return (self.lr.is_empty() || self.lr_unknown).then_some((rt, LrCopyWrite::Copy));
        }
        let tracked = self.lr_copies | self.lr_stepped;
        if tracked == 0 {
            return None;
        }
        let (rt, sources) = computed_from(decoded)?;
        if sources & tracked == 0 {
            return None;
        }
        let unchanged = sources & self.lr_stepped == 0
            && (matches!(decoded, PpuInstruction::Mr { .. })
                || GprStep::of(*decoded).is_some_and(|step| step.step.keeps_value()));
        Some((
            rt,
            if unchanged {
                LrCopyWrite::Copy
            } else {
                LrCopyWrite::Stepped
            },
        ))
    }

    /// Updates the copies of an LR that the scan does not know after the
    /// step over `decoded`, and marks a stepped copy as lost.
    ///
    /// `write` is what [`Self::lr_copy_write`] gave before the step. A GPR
    /// that the word surely writes holds no copy and no stepped copy, unless
    /// `write` puts one there.
    fn after_lr_copy_write(&mut self, decoded: &PpuInstruction, write: Option<(u8, LrCopyWrite)>) {
        if self.lr_copies | self.lr_stepped != 0 {
            let written = surely_written_gprs(decoded);
            self.lr_copies &= !written;
            self.lr_stepped &= !written;
        }
        match write {
            Some((rt, LrCopyWrite::Copy)) => self.lr_copies |= gpr_bit(rt),
            Some((rt, LrCopyWrite::Stepped)) => {
                self.lr_stepped |= gpr_bit(rt);
                self.lost_gprs.join_gpr(rt, &Origins::unnamed());
            }
            None => {}
        }
    }

    /// Steps over a branch at `address` with the BO field `bo`, which
    /// writes LR when `link` is set.
    ///
    /// The routine that a branch with `link` calls can write SRR0 and
    /// SRR1, so the step forgets their constants. The routine can also
    /// leave them unchanged, so SRR0, SRR1 and [`Self::moved_lost`] keep
    /// their lost marks.
    ///
    /// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
    /// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR.
    fn after_branch(&mut self, address: u64, bo: u8, link: bool) {
        if !falls_through(bo, link) {
            *self = Self::default();
            return;
        }
        if link {
            // The scan does not join what the routine of the call leaves
            // in the GPRs back into the word after it. The routine can
            // write each GPR.
            self.unheld = u32::MAX;
            self.srr0 = None;
            self.srr1 = None;
            self.lr_lost = None;
            self.lr_unknown = false;
            self.lr_unheld = false;
            self.lr = address
                .checked_add(4)
                .map(|value| BTreeMap::from([(address, Value::One(value))]))
                .unwrap_or_default();
        }
    }
}

/// The MSR\[SF\] and MSR bit 1 of each MSR constant of `msr` that sets
/// MSR\[IR\] to `ir`.
///
/// An MSR value that the scan does not know can set IR to 0 or to 1, with
/// SF at 0 or at 1.
///
/// [PPC-Book3 p:13 s:2.3] `rfid` sets MSR\[IR\] to SRR1\[IR\] | SRR1\[PR\] and fetches the next instruction from SRR0 under the new MSR.
fn fetch_kinds(msr: Option<&Value>, ir: bool) -> BTreeSet<u64> {
    match msr {
        Some(msr) => msr
            .constants()
            .iter()
            .filter(|&&msr| (msr & (MSR_IR | MSR_PR) != 0) == ir)
            .map(|&msr| msr & (MSR_SF | MSR_BIT1))
            .collect(),
        None => BTreeSet::from([0, MSR_SF]),
    }
}

/// Whether the word `raw` ends each path through it.
///
/// These words end a path, as in [`Constants::step`]:
/// - a word that does not decode;
/// - a word that never falls through.
pub(super) fn ends_path(raw: u32) -> bool {
    match crate::decode::decode(raw) {
        Err(_) => true,
        Ok(PpuInstruction::B { link, .. }) => !falls_through(0b10100, link),
        Ok(
            PpuInstruction::Bc { bo, link, .. }
            | PpuInstruction::Bclr { bo, link, .. }
            | PpuInstruction::Bcctr { bo, link, .. },
        ) => !falls_through(bo, link),
        Ok(decoded) => never_falls_through(&decoded),
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

    /// Whether the step gives each value unchanged.
    fn keeps_value(self) -> bool {
        matches!(
            self,
            Self::Add(0)
                | Self::Or(0)
                | Self::RotateAnd {
                    sh: 0,
                    mask: u64::MAX
                }
        )
    }
}

/// How a word writes a GPR from LR or from a copy of an LR that the scan
/// does not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LrCopyWrite {
    /// The word copies the value unchanged.
    Copy,
    /// The word computes a different value from the copy.
    Stepped,
}

/// The mask with bit `gpr` set.
fn gpr_bit(gpr: u8) -> u32 {
    1u32.checked_shl(u32::from(gpr)).unwrap_or(0)
}

/// Each GPR whose bit is set in `mask`, in ascending order.
fn gprs_of(mask: u32) -> impl Iterator<Item = u8> {
    (0..32u8).filter(move |&gpr| mask & gpr_bit(gpr) != 0)
}

/// How a word moves a value between the GPRs and CR or an SPR that the
/// scan does not follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SprMove {
    /// The word moves the GPRs of the mask, with bit `n` set for GPR `n`,
    /// into such a register.
    Into(u32),
    /// The word moves such a register into the GPR.
    OutOf(u8),
}

/// How `decoded` moves a value between the GPRs and CR or an SPR that the
/// scan does not follow, or `None` for a word that moves no such value.
///
/// The scan follows LR, SRR0 and SRR1 on their own. The CTR flow follows
/// a copy of CTR in a GPR only into a later `mtctr`, so a move between CTR
/// and a GPR takes the mark too. These words move a GPR into one of the
/// other registers:
/// - `mtcrf` and `mtocrf`;
/// - `mtctr`, `mtxer`, `mtvrsave` and each other `mtspr`;
/// - `mtmsr` and `mtmsrd`;
/// - `slbmte`, from RS and RB.
///
/// These words move one of them into a GPR:
/// - `mfcr` and `mfocrf`;
/// - `mfctr`, `mfxer`, `mfvrsave`, `mfsprg3`, `mfctrl`, `mftb`, `mftbu`
///   and each other `mfspr`;
/// - `mfmsr`, `slbmfev` and `slbmfee`.
///
/// [PPC-Book1 p:81 s:3.3.13] `mtctr` is `mtspr` of SPR 9, which copies RS into CTR.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr` is `mfspr` of SPR 9, which places CTR into RT.
/// [PPC-Book1 p:83 s:3.3.13] `mtcrf` writes the CR fields that FXM selects from RS, and `mfcr` places CR into RT.
/// [PPC-Book1 p:124 s:5.1.1] `mtocrf` and `mfocrf` move one CR field between CR and a GPR.
/// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
/// [PPC-Book1 p:82 s:3.3.13] `mfspr` places the SPR into RT.
/// [PPC-Book3 p:16 s:3.3.3] Problem-state software may read SPRG3 through a second SPR number.
fn spr_move(decoded: &PpuInstruction) -> Option<SprMove> {
    use PpuInstruction as I;
    Some(match *decoded {
        I::Mtcrf { rs, .. }
        | I::Mtocrf { rs, .. }
        | I::Mtctr { rs }
        | I::Mtxer { rs }
        | I::Mtvrsave { rs } => SprMove::Into(gpr_bit(rs)),
        I::Mfcr { rt }
        | I::Mfocrf { rt, .. }
        | I::Mfctr { rt }
        | I::Mfxer { rt }
        | I::Mfvrsave { rt }
        | I::Mfsprg3 { rt }
        | I::Mfctrl { rt }
        | I::Mftb { rt }
        | I::Mftbu { rt } => SprMove::OutOf(rt),
        I::PpePrivileged { raw } => return privileged_spr_move(raw),
        _ => return None,
    })
}

/// How the privileged word `raw` moves a value between the GPRs and an SPR
/// that the scan does not follow, the MSR or an SLB entry.
///
/// [PPC-Book3 p:19 s:3.4] `mtspr` is primary opcode 31 with extended opcode 467, RS in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
/// [PPC-Book3 p:21 s:3.4] `mfspr` is primary opcode 31 with extended opcode 339, RT in bits 6:10 and the SPR number in bits 11:20 with its halves swapped.
/// [PPC-Book3 p:91 s:8.3] `mtmsr` is primary opcode 31 with extended opcode 146, RS in bits 6:10 and L in bit 15.
/// [PPC-Book3 p:22 s:3.4] `mtmsrd` is primary opcode 31 with extended opcode 178, RS in bits 6:10 and L in bit 15.
/// [PPC-Book3 p:23 s:3.4] `mfmsr` places the MSR into RT.
/// [PPC-Book3 p:51 s:4.11.3.1] `slbmte` takes the ESID from RB and the VSID and flags from RS.
/// [PPC-Book3 p:52 s:4.11] `slbmfev` and `slbmfee` place SLB entry fields into RT.
fn privileged_spr_move(raw: u32) -> Option<SprMove> {
    if raw >> 26 != 31 {
        return None;
    }
    let rt = ((raw >> 21) & 0x1F) as u8;
    let rb = ((raw >> 11) & 0x1F) as u8;
    let followed = matches!(spr_number(raw), SPR_SRR0 | SPR_SRR1);
    Some(match (raw >> 1) & 0x3FF {
        467 if !followed => SprMove::Into(gpr_bit(rt)),
        146 | 178 => SprMove::Into(gpr_bit(rt)),
        402 => SprMove::Into(gpr_bit(rt) | gpr_bit(rb)),
        339 if !followed => SprMove::OutOf(rt),
        83 | 851 | 915 => SprMove::OutOf(rt),
        _ => return None,
    })
}

/// The GPR that the arithmetic, logical, rotate or shift word `decoded`
/// writes, and the GPRs that it computes the result from, as a mask.
///
/// The result is `None` for each other word. A load computes its address
/// from its base GPR but loads its result from storage, so it is such a
/// word. The update form of a load or a store also places that address
/// into RA. For that form the result is RA, which the word computes from
/// RA, and also from RB for an indexed form.
///
/// [PPC-Book1 p:33 s:3.3.2] A load with update places the EA into RA.
/// [PPC-Book1 p:40 s:3.3.3] A store with update places the EA into RA.
/// [PPC-Book1 p:105 s:4.6.2] A floating-point load with update places (RA) + EXTS(D), or (RA) + (RB), into RA.
/// [PPC-Book1 p:107 s:4.6.3] A floating-point store with update writes the EA into RA.
/// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:51 s:3.3.8] An arithmetic instruction places its result into RT.
/// [PPC-Book1 p:65 s:3.3.11] A logical instruction places its result into RA.
/// [PPC-Book1 p:71 s:3.3.12] A rotate or shift instruction returns its result to a GPR, RA.
/// [PPC-Book1 p:76 s:3.3.12.1] `rldimi` inserts the rotated RS into RA under the mask, so the result also holds bits of RA.
fn computed_from(decoded: &PpuInstruction) -> Option<(u8, u32)> {
    use PpuInstruction as I;
    let base = |ra: u8| if ra == 0 { 0 } else { gpr_bit(ra) };
    Some(match *decoded {
        I::Addi { rt, ra, .. } | I::Addis { rt, ra, .. } => (rt, base(ra)),
        I::Subfic { rt, ra, .. }
        | I::Mulli { rt, ra, .. }
        | I::Addic { rt, ra, .. }
        | I::AddicDot { rt, ra, .. }
        | I::Neg { rt, ra, .. }
        | I::Addze { rt, ra, .. }
        | I::Subfze { rt, ra, .. }
        | I::Subfme { rt, ra, .. }
        | I::Addme { rt, ra, .. } => (rt, gpr_bit(ra)),
        I::Add { rt, ra, rb, .. }
        | I::Subf { rt, ra, rb, .. }
        | I::Subfc { rt, ra, rb, .. }
        | I::Addc { rt, ra, rb, .. }
        | I::Subfe { rt, ra, rb, .. }
        | I::Adde { rt, ra, rb, .. }
        | I::Mullw { rt, ra, rb, .. }
        | I::Mulhwu { rt, ra, rb, .. }
        | I::Mulhw { rt, ra, rb, .. }
        | I::Mulhdu { rt, ra, rb, .. }
        | I::Mulhd { rt, ra, rb, .. }
        | I::Mulld { rt, ra, rb, .. }
        | I::Divw { rt, ra, rb, .. }
        | I::Divwu { rt, ra, rb, .. }
        | I::Divd { rt, ra, rb, .. }
        | I::Divdu { rt, ra, rb, .. } => (rt, gpr_bit(ra) | gpr_bit(rb)),
        I::Or { ra, rs, rb, .. }
        | I::Orc { ra, rs, rb, .. }
        | I::And { ra, rs, rb, .. }
        | I::Andc { ra, rs, rb, .. }
        | I::Nor { ra, rs, rb, .. }
        | I::Xor { ra, rs, rb, .. }
        | I::Eqv { ra, rs, rb, .. }
        | I::Nand { ra, rs, rb, .. }
        | I::Slw { ra, rs, rb, .. }
        | I::Srw { ra, rs, rb, .. }
        | I::Sraw { ra, rs, rb, .. }
        | I::Srad { ra, rs, rb, .. }
        | I::Sld { ra, rs, rb, .. }
        | I::Srd { ra, rs, rb, .. }
        | I::Rlwnm { ra, rs, rb, .. }
        | I::Rldcl { ra, rs, rb, .. }
        | I::Rldcr { ra, rs, rb, .. } => (ra, gpr_bit(rs) | gpr_bit(rb)),
        I::Ori { ra, rs, .. }
        | I::Oris { ra, rs, .. }
        | I::Xori { ra, rs, .. }
        | I::Xoris { ra, rs, .. }
        | I::AndiDot { ra, rs, .. }
        | I::AndisDot { ra, rs, .. }
        | I::Extsh { ra, rs, .. }
        | I::Extsb { ra, rs, .. }
        | I::Extsw { ra, rs, .. }
        | I::Cntlzw { ra, rs, .. }
        | I::Cntlzd { ra, rs, .. }
        | I::Popcntb { ra, rs }
        | I::Srawi { ra, rs, .. }
        | I::Sradi { ra, rs, .. }
        | I::Rlwinm { ra, rs, .. }
        | I::RlwinmMask { ra, rs, .. }
        | I::Rldicl { ra, rs, .. }
        | I::Rldicr { ra, rs, .. }
        | I::Rldic { ra, rs, .. }
        | I::Mr { ra, rs }
        | I::Slwi { ra, rs, .. }
        | I::Srwi { ra, rs, .. }
        | I::Clrlwi { ra, rs, .. }
        | I::Clrldi { ra, rs, .. }
        | I::Sldi { ra, rs, .. }
        | I::Srdi { ra, rs, .. } => (ra, gpr_bit(rs)),
        I::Rlwimi { ra, rs, .. } | I::Rldimi { ra, rs, .. } => (ra, gpr_bit(rs) | gpr_bit(ra)),
        I::Lhau { ra, .. }
        | I::Lwzu { ra, .. }
        | I::Lbzu { ra, .. }
        | I::Lhzu { ra, .. }
        | I::Ldu { ra, .. }
        | I::Stwu { ra, .. }
        | I::Stdu { ra, .. }
        | I::Stbu { ra, .. }
        | I::Sthu { ra, .. }
        | I::Lfsu { ra, .. }
        | I::Lfdu { ra, .. }
        | I::Stfsu { ra, .. }
        | I::Stfdu { ra, .. } => (ra, gpr_bit(ra)),
        I::Lwzux { ra, rb, .. }
        | I::Lbzux { ra, rb, .. }
        | I::Lhzux { ra, rb, .. }
        | I::Ldux { ra, rb, .. }
        | I::Lhaux { ra, rb, .. }
        | I::Lwaux { ra, rb, .. }
        | I::Stdux { ra, rb, .. }
        | I::Sthux { ra, rb, .. }
        | I::Stwux { ra, rb, .. }
        | I::Stbux { ra, rb, .. }
        | I::Lfsux { ra, rb, .. }
        | I::Lfdux { ra, rb, .. }
        | I::Stfsux { ra, rb, .. }
        | I::Stfdux { ra, rb, .. } => (ra, gpr_bit(ra) | gpr_bit(rb)),
        _ => return None,
    })
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
