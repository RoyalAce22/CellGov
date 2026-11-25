//! What the paths to each word bring in CTR, in the GPRs that hold a copy
//! of CTR, and in the stack slots that hold one.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_BIT2;

use super::constants::{never_falls_through, GprStep, Step, Value};
use super::reach::Reach;
use super::slot_set::{Slot, SlotSet};
use super::toc::FunctionSteps;
use crate::caller_census::surely_written_gprs;
use crate::hypercall_cfg::{branch_target, Code, CodeSegment};
use crate::instruction::PpuInstruction;
use crate::local_cfg::falls_through;

/// Whether the word `raw` at `address` is a branch that links, other
/// than a `b` or a `bc` that links to the next word.
///
/// A `bc` that links to the next word, such as `bcl 20,31,$+4`, reaches
/// the next word on the taken and on the fall-through path, so it runs
/// no routine.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:24 s:2.4] A taken `bc` sets NIA to its target, and a `bc` sets LR whether or not it branches.
pub(super) fn calls(address: u64, raw: u32) -> bool {
    match crate::decode::decode(raw) {
        Ok(PpuInstruction::B { link, .. } | PpuInstruction::Bc { link, .. }) => {
            link && !links_to_next_word(address, raw)
        }
        Ok(PpuInstruction::Bclr { link, .. } | PpuInstruction::Bcctr { link, .. }) => link,
        _ => false,
    }
}

/// Whether the word `raw` at `address` is a `b` or a `bc` that links to
/// the next word.
pub(super) fn links_to_next_word(address: u64, raw: u32) -> bool {
    matches!(
        crate::decode::decode(raw),
        Ok(PpuInstruction::B { link: true, .. } | PpuInstruction::Bc { link: true, .. })
    ) && FunctionSteps::of(address, raw).target == address.checked_add(4)
}

/// The address of the word after each `b` or `bc` of `code` that links
/// to the next word.
fn after_next_word_links(code: &Code) -> impl Iterator<Item = u64> + '_ {
    code.segments()
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(address, raw)| links_to_next_word(address, raw))
        .filter_map(|(address, _)| address.checked_add(4))
}

/// The address of the word after each call of `code`, where the `bclr`
/// of its routine returns.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn after_calls(code: &Code) -> impl Iterator<Item = u64> + '_ {
    code.segments()
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(address, raw)| calls(address, raw))
        .filter_map(|(address, _)| address.checked_add(4))
}

/// What the paths to one word bring in CTR.
///
/// A path can also bring a CTR that no word of the scan writes, such as
/// a value that the code loads. The bound takes such a CTR to hold a
/// named address, or in real mode a word of
/// [`IndirectBound::real_named`]. So the state does not record it.
///
/// [`IndirectBound::real_named`]: super::IndirectBound::real_named
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ctr {
    /// The constants that the paths bring in CTR, by the address of the
    /// word that wrote each one.
    known: BTreeMap<u64, Value>,
    /// Whether a path brings a value that the scan cannot prove.
    lost: bool,
}

impl Ctr {
    /// CTR after the word at `by` writes the constants `value`.
    fn written(by: u64, value: Value) -> Self {
        Self {
            known: BTreeMap::from([(by, value)]),
            lost: false,
        }
    }

    /// CTR after a step that can leave it at any value.
    fn lost() -> Self {
        Self {
            known: BTreeMap::new(),
            lost: true,
        }
    }

    /// CTR after a step that keeps no constant of it.
    ///
    /// Each constant becomes a value that the scan cannot prove. A CTR
    /// that no word of the scan writes stays out of the state.
    fn unproved(&self) -> Self {
        Self {
            known: BTreeMap::new(),
            lost: self.lost || !self.known.is_empty(),
        }
    }

    /// A copy of CTR after a word applies `step` to each of its constants.
    ///
    /// A step that changes a constant leaves a value that the scan cannot
    /// prove, because a loop can run the step more than once.
    fn stepped(&self, step: Step) -> Self {
        let unchanged = self
            .known
            .values()
            .all(|value| value.map(|constant| step.apply(constant)) == *value);
        if unchanged {
            self.clone()
        } else {
            self.unproved()
        }
    }

    /// A copy of CTR after `part` changes each of its constants.
    ///
    /// The change is not a step of a loop, so each constant stays a
    /// constant.
    fn part(&self, part: impl Fn(u64) -> u64) -> Self {
        Self {
            known: self
                .known
                .iter()
                .map(|(&by, value)| (by, value.map(&part)))
                .collect(),
            lost: self.lost,
        }
    }

    /// CTR after the word at `by` copies this copy of CTR back into CTR.
    ///
    /// The result records the word at `by` as the word that wrote each
    /// constant.
    fn rewritten(&self, by: u64) -> Self {
        let known = self
            .known
            .values()
            .cloned()
            .reduce(|one, two| one.union(&two));
        Self {
            known: known
                .map(|value| BTreeMap::from([(by, value)]))
                .unwrap_or_default(),
            lost: self.lost,
        }
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let changed = other.lost && !self.lost;
        self.lost |= other.lost;
        join_known(&mut self.known, &other.known) || changed
    }
}

/// The constants of one copy of CTR, by the address of the word that
/// wrote each one.
type Known = BTreeMap<u64, Value>;

/// Joins the constants `other` into `own`, and returns whether `own`
/// changed.
fn join_known(own: &mut Known, other: &Known) -> bool {
    let mut changed = false;
    for (&by, value) in other {
        match own.get_mut(&by) {
            Some(held) if held.holds_all(value) => {}
            Some(held) => {
                *held = held.clone().union(value);
                changed = true;
            }
            None => {
                own.insert(by, value.clone());
                changed = true;
            }
        }
    }
    changed
}

/// The mask with bit `n` set for GPR `n`.
fn bit(gpr: u8) -> u32 {
    1u32.checked_shl(u32::from(gpr)).unwrap_or(0)
}

/// What the paths bring in the GPRs that hold a copy of CTR.
///
/// A GPR with no entry in `known` and no bit in `lost` holds no copy that
/// the state records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GprCopies {
    /// The constants of each copy that holds one, by GPR number.
    known: BTreeMap<u8, Known>,
    /// Each GPR whose copy can hold a value that the scan cannot prove,
    /// as bit `n` for GPR `n`.
    lost: u32,
}

impl GprCopies {
    /// The copy in `gpr`, or `None` when the GPR holds no copy.
    fn get(&self, gpr: u8) -> Option<Ctr> {
        let lost = self.lost & bit(gpr) != 0;
        let known = self.known.get(&gpr);
        (lost || known.is_some()).then(|| Ctr {
            known: known.cloned().unwrap_or_default(),
            lost,
        })
    }

    /// The GPRs that hold a copy, as a mask with bit `n` set for GPR `n`.
    fn mask(&self) -> u32 {
        self.known
            .keys()
            .fold(self.lost, |gprs, &gpr| gprs | bit(gpr))
    }

    /// Puts `copy` into `gpr` in place of the copy there.
    fn set(&mut self, gpr: u8, copy: Ctr) {
        if copy.known.is_empty() {
            self.known.remove(&gpr);
        } else {
            self.known.insert(gpr, copy.known);
        }
        if copy.lost {
            self.lost |= bit(gpr);
        } else {
            self.lost &= !bit(gpr);
        }
    }

    /// Joins `copy` into the copy in `gpr`.
    fn join_at(&mut self, gpr: u8, copy: &Ctr) {
        if copy.lost {
            self.lost |= bit(gpr);
        }
        if !copy.known.is_empty() {
            join_known(self.known.entry(gpr).or_default(), &copy.known);
        }
    }

    /// Removes the copy of each GPR in the mask `written`.
    fn remove(&mut self, written: u32) {
        self.known.retain(|&gpr, _| written & bit(gpr) == 0);
        self.lost &= !written;
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = other.lost & !self.lost != 0;
        self.lost |= other.lost;
        for (&gpr, known) in &other.known {
            changed |= join_known(self.known.entry(gpr).or_default(), known);
        }
        changed
    }
}

/// What the paths bring in the stack slots that hold a copy of CTR or a
/// part of one.
///
/// A slot with no entry in `known` that is not in `lost` holds no copy
/// that the state records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SlotCopies {
    /// The constants of each copy that holds one, by slot.
    known: BTreeMap<Slot, Known>,
    /// Each slot whose copy can hold a value that the scan cannot prove.
    lost: SlotSet,
}

impl SlotCopies {
    /// The slots that hold a copy.
    fn slots(&self) -> SlotSet {
        let mut slots = SlotSet::of(self.known.keys().copied());
        slots.join(&self.lost);
        slots
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = self.lost.join(&other.lost);
        for (&slot, known) in &other.known {
            changed |= join_known(self.known.entry(slot).or_default(), known);
        }
        changed
    }

    /// The copy that a load of `slot` gets from the slots that hold a
    /// copy, and that `load` extends to 64 bits.
    ///
    /// A load whose bytes all lie in one slot gets the part of the copy
    /// there. A load that reads some bytes of a slot and some other bytes
    /// gets a value that the scan cannot prove.
    ///
    /// [CBE-Handbook p:45 s:1.3.3] The CBE processor stores the most-significant byte at the lowest address.
    fn loaded(&self, (offset, width): Slot, load: Load) -> Ctr {
        let end = offset + i64::from(width);
        let mut loaded = Ctr::default();
        for (&(held, held_width), known) in &self.known {
            let held_end = held + i64::from(held_width);
            if end <= held || held_end <= offset {
                continue;
            }
            if held <= offset && end <= held_end {
                let shift = 8 * (held_end - end);
                let part = known
                    .iter()
                    .map(|(&by, value)| (by, value.map(|value| load.extend(value >> shift, width))))
                    .collect();
                join_known(&mut loaded.known, &part);
            } else {
                loaded.lost = true;
            }
        }
        loaded.lost |= self.lost.overlaps(offset, end);
        loaded
    }

    /// Writes `slot` with `copy`, or with a value that holds no copy when
    /// `copy` is `None`.
    ///
    /// The store removes each slot that it writes in full. A slot that it
    /// writes in part holds a value that the scan cannot prove. The new
    /// slot holds the low-order bytes of `copy`.
    ///
    /// [PPC-Book1 p:42 s:3.3.3] `stw` stores RS 32:63 into the word at EA.
    /// [PPC-Book1 p:43 s:3.3.3] `std` stores RS into the doubleword at EA.
    fn store(&mut self, (offset, width): Slot, copy: Option<&Ctr>) {
        let end = offset + i64::from(width);
        let mut part_written = Vec::new();
        self.known.retain(|&(held, held_width), _| {
            let held_end = held + i64::from(held_width);
            if end <= held || held_end <= offset {
                return true;
            }
            if held < offset || end < held_end {
                part_written.push((held, held_width));
            }
            false
        });
        self.lost = self.lost.without_inside(offset, end);
        for slot in part_written {
            self.lost.insert(slot);
        }
        if let Some(copy) = copy {
            let low = copy.part(|value| Load::Zero.extend(value, width));
            if low.lost {
                self.lost.insert((offset, width));
            }
            if !low.known.is_empty() {
                self.known.insert((offset, width), low.known);
            }
        }
    }

    /// The copies after r1 moves by `by`, without each slot that the move
    /// takes out of [`SLOT_REACH`].
    fn moved(&self, by: i64) -> Self {
        let known = self
            .known
            .iter()
            .filter_map(|(&(offset, width), known)| {
                let offset = offset.checked_sub(by)?;
                SLOT_REACH
                    .contains(&offset)
                    .then(|| ((offset, width), known.clone()))
            })
            .collect();
        Self {
            known,
            lost: self.lost.moved(by, &SLOT_REACH),
        }
    }
}

/// The offsets from r1 that a displacement can reach.
///
/// A move of r1 drops each slot that it takes out of this range, so a
/// loop that moves r1 keeps a finite set of slots.
///
/// [PPC-Book1 p:37 s:3.3.2] `lwz` computes EA as (RA|0) + EXTS(D), with D a 16-bit field.
const SLOT_REACH: std::ops::Range<i64> = -(1 << 15)..(1 << 15);

/// What the paths to one word bring in CTR, in the GPRs that hold a copy
/// of CTR, and in the stack slots that hold one.
///
/// A copy that the scan cannot prove is a bit of a mask or a slot of a
/// [`SlotSet`], with no entry of its own. So a state that holds such a
/// copy in each slot of [`SLOT_REACH`] stays small. The states of two
/// words that get the same lost slots can share one [`SlotSet`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CtrState {
    ctr: Ctr,
    copies: GprCopies,
    slots: SlotCopies,
}

impl CtrState {
    /// A state with no CTR, and a copy that the scan cannot prove in each
    /// GPR of `gprs` and each slot of `slots`.
    ///
    /// `gprs` is a mask with bit `n` set for GPR `n`.
    fn lost_copies(gprs: u32, slots: &SlotSet) -> Self {
        Self {
            ctr: Ctr::default(),
            copies: GprCopies {
                known: BTreeMap::new(),
                lost: gprs,
            },
            slots: SlotCopies {
                known: BTreeMap::new(),
                lost: slots.clone(),
            },
        }
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let ctr = self.ctr.join(&other.ctr);
        let copies = self.copies.join(&other.copies);
        self.slots.join(&other.slots) || ctr || copies
    }

    /// The copies in GPRs and in stack slots after the word `decoded`.
    ///
    /// These words copy CTR or a copy into a GPR:
    /// - `mfctr`, which copies CTR;
    /// - a word that the scan follows a constant through, from a GPR
    ///   that holds a copy;
    /// - a load at a constant offset from r1, from the slots that hold a
    ///   copy there.
    ///
    /// Each other word that surely writes a GPR removes its copy. A word can
    /// also copy into a GPR that it does not surely write, such as an
    /// invalid form of a load. That word joins the copy into the copy of
    /// the GPR. A store at a constant offset from r1 writes its slot, as
    /// [`SlotCopies::store`] gives. A word that moves r1 by a constant
    /// moves each slot, and each other word that surely writes r1 removes
    /// each slot.
    ///
    /// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
    fn after(&self, decoded: &PpuInstruction) -> (GprCopies, SlotCopies) {
        let mut copied: Vec<(u8, Ctr)> = match *decoded {
            PpuInstruction::Mfctr { rt } => vec![(rt, self.ctr.clone())],
            _ => GprStep::of(*decoded)
                .and_then(|step| {
                    let copy = self.copies.get(step.source?)?;
                    Some((step.rt, copy.stepped(step.step)))
                })
                .into_iter()
                .collect(),
        };
        let mut slots = self.slots.clone();
        let access = StackAccess::of(decoded);
        if let Some(access) = &access {
            for &(slot, gpr) in &access.parts {
                if access.load == Load::None {
                    let copy = gpr.and_then(|gpr| self.copies.get(gpr));
                    slots.store(slot, copy.as_ref());
                } else {
                    copied.extend(gpr.map(|gpr| (gpr, self.slots.loaded(slot, access.load))));
                }
            }
        }
        let written = surely_written_gprs(decoded);
        let mut copies = self.copies.clone();
        copies.remove(written);
        for (gpr, copy) in copied {
            if copy == Ctr::default() {
                continue;
            }
            if written & bit(gpr) == 0 {
                copies.join_at(gpr, &copy);
            } else {
                copies.set(gpr, copy);
            }
        }
        if written & 1 << 1 != 0 {
            slots = match r1_move(decoded, access.as_ref()) {
                Some(by) => slots.moved(by),
                None => SlotCopies::default(),
            };
        }
        (copies, slots)
    }
}

/// The constant that `decoded` adds to r1, for a word that moves r1 by a
/// constant.
///
/// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:40 s:3.3.3] A store with update places EA into RA.
fn r1_move(decoded: &PpuInstruction, access: Option<&StackAccess>) -> Option<i64> {
    if let Some(by) = access.and_then(|access| access.moves_r1) {
        return Some(by);
    }
    match GprStep::of(*decoded)? {
        GprStep {
            rt: 1,
            source: Some(1),
            step: Step::Add(by),
        } => Some(by),
        _ => None,
    }
}

/// How a load fills the bytes of RT that it does not load, or
/// [`Load::None`] for a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    /// A store, which loads no GPR.
    None,
    /// A load that sets the other bytes to 0.
    Zero,
    /// A load that copies the high-order bit of the loaded bytes into the
    /// other bytes.
    Sign,
}

impl Load {
    /// The 64-bit value of the low-order `width` bytes of `value`.
    fn extend(self, value: u64, width: u8) -> u64 {
        let bits = 8 * u32::from(width);
        if bits >= 64 {
            return value;
        }
        let low = value & ((1 << bits) - 1);
        if self == Self::Sign && low >> (bits - 1) & 1 != 0 {
            low | (u64::MAX << bits)
        } else {
            low
        }
    }
}

/// A load or a store that a word makes at a constant offset from r1.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StackAccess {
    load: Load,
    /// The slot of each GPR that the word loads or stores, and the GPR.
    /// The GPR is `None` for the FPR of a floating-point load or store.
    parts: Vec<(Slot, Option<u8>)>,
    /// The constant that the update form adds to r1.
    moves_r1: Option<i64>,
}

impl StackAccess {
    /// The access of `decoded`, or `None` for a word that makes no load
    /// or store at a constant offset from r1.
    ///
    /// [PPC-Book1 p:33 s:3.3.2] A load writes RT, and its update form also writes the EA into RA.
    /// [PPC-Book1 p:36 s:3.3.2] `lha` loads the halfword at EA and extends its sign into RT.
    /// [PPC-Book1 p:38 s:3.3.2] `lwa` loads the word at EA and extends its sign into RT.
    /// [PPC-Book1 p:40 s:3.3.3] A store writes no GPR, and its update form writes the EA into RA.
    /// [PPC-Book1 p:46 s:3.3.5] `lmw` and `stmw` load or store the low-order words of the GPRs from RT or RS through 31.
    /// [PPC-Book1 p:104 s:4.6.2] `lfsu` loads an FPR and places the EA into RA.
    /// [PPC-Book1 p:105 s:4.6.2] `lfdu` loads an FPR and places the EA into RA.
    /// [PPC-Book1 p:107 s:4.6.3] `stfs` stores a word at EA.
    /// [PPC-Book1 p:108 s:4.6.3] `stfd` stores a doubleword at EA.
    fn of(decoded: &PpuInstruction) -> Option<Self> {
        use PpuInstruction as I;
        let (load, ra, imm, width, gpr, update) = match *decoded {
            I::Lbz { rt, ra, imm } => (Load::Zero, ra, imm, 1, Some(rt), false),
            I::Lbzu { rt, ra, imm } => (Load::Zero, ra, imm, 1, Some(rt), true),
            I::Lhz { rt, ra, imm } => (Load::Zero, ra, imm, 2, Some(rt), false),
            I::Lhzu { rt, ra, imm } => (Load::Zero, ra, imm, 2, Some(rt), true),
            I::Lha { rt, ra, imm } => (Load::Sign, ra, imm, 2, Some(rt), false),
            I::Lhau { rt, ra, imm } => (Load::Sign, ra, imm, 2, Some(rt), true),
            I::Lwz { rt, ra, imm } => (Load::Zero, ra, imm, 4, Some(rt), false),
            I::Lwzu { rt, ra, imm } => (Load::Zero, ra, imm, 4, Some(rt), true),
            I::Lwa { rt, ra, imm } => (Load::Sign, ra, imm, 4, Some(rt), false),
            I::Ld { rt, ra, imm } => (Load::Zero, ra, imm, 8, Some(rt), false),
            I::Ldu { rt, ra, imm } => (Load::Zero, ra, imm, 8, Some(rt), true),
            I::Lfsu { ra, imm, .. } => (Load::Zero, ra, imm, 4, None, true),
            I::Lfdu { ra, imm, .. } => (Load::Zero, ra, imm, 8, None, true),
            I::Stb { rs, ra, imm } => (Load::None, ra, imm, 1, Some(rs), false),
            I::Stbu { rs, ra, imm } => (Load::None, ra, imm, 1, Some(rs), true),
            I::Sth { rs, ra, imm } => (Load::None, ra, imm, 2, Some(rs), false),
            I::Sthu { rs, ra, imm } => (Load::None, ra, imm, 2, Some(rs), true),
            I::Stw { rs, ra, imm } => (Load::None, ra, imm, 4, Some(rs), false),
            I::Stwu { rs, ra, imm } => (Load::None, ra, imm, 4, Some(rs), true),
            I::Std { rs, ra, imm } => (Load::None, ra, imm, 8, Some(rs), false),
            I::Stdu { rs, ra, imm } => (Load::None, ra, imm, 8, Some(rs), true),
            I::Stfs { ra, imm, .. } => (Load::None, ra, imm, 4, None, false),
            I::Stfsu { ra, imm, .. } => (Load::None, ra, imm, 4, None, true),
            I::Stfd { ra, imm, .. } => (Load::None, ra, imm, 8, None, false),
            I::Stfdu { ra, imm, .. } => (Load::None, ra, imm, 8, None, true),
            I::Lmw { rt, ra, imm } => return Self::multiple(Load::Zero, rt, ra, imm),
            I::Stmw { rs, ra, imm } => return Self::multiple(Load::None, rs, ra, imm),
            _ => return None,
        };
        let offset = i64::from(imm);
        (ra == 1).then(|| Self {
            load,
            parts: vec![((offset, width), gpr)],
            moves_r1: update.then_some(offset),
        })
    }

    /// The access of an `lmw` or an `stmw` of the GPRs from `first`
    /// through 31 at `imm` from RA.
    fn multiple(load: Load, first: u8, ra: u8, imm: i16) -> Option<Self> {
        (ra == 1).then(|| Self {
            load,
            parts: (first..32)
                .map(|gpr| ((i64::from(imm) + 4 * i64::from(gpr - first), 4), Some(gpr)))
                .collect(),
            moves_r1: None,
        })
    }
}

/// What the paths to each word of the code bring in CTR.
///
/// A path goes from a word to these words:
/// - the next word, when the word can fall through;
/// - each word at the real address of the target of a direct branch
///   that is not a call;
/// - from a `bclr`, each word at the real address of the word after a
///   `b` or a `bc` that links to the next word.
///
/// The flow does not follow CTR or a copy of it to these words:
/// - the first word of the routine that a call runs;
/// - the target of an indirect branch, an `rfid` or an interrupt.
///
/// A path to one of these words brings a CTR that the bound takes to
/// hold a named address. The word after a call gets a CTR that the scan
/// cannot prove. The word after an `sc` gets the CTR before the `sc`, so
/// the flow takes the handler to return with CTR unchanged.
///
/// A `b` or a `bc` that links to the next word calls no routine, so the
/// next word gets the CTR before the branch. A `bclr` can return there
/// later with another CTR, so that word also gets what each `bclr`
/// brings.
///
/// The flow also follows each copy of CTR in a GPR on the same paths. An
/// `mtctr` of a GPR that holds a copy puts the copy back into CTR, so a
/// CTR that the scan cannot prove stays so through the copy.
///
/// The flow also follows each copy through a stack slot at a constant
/// offset from r1, as [`CtrState::after`] gives.
///
/// The word after a call gets these copies:
/// - each copy before the call, because its routine can return the GPR
///   or the slot unchanged;
/// - a copy of a CTR that the scan cannot prove, in each GPR and slot
///   that holds a copy at a `bclr`, because its routine can make that
///   copy.
///
/// The flow joins the CTR of each path to a word, and continues until no
/// word changes. Each state only grows, from finite sets of constants,
/// so the flow ends.
///
/// The flow steps only the words of a [`Reach`] set. A word outside the
/// set gets no state, and its `bcctr` reaches no target.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
pub(super) struct CtrFlow<'a> {
    code: &'a Code,
    reach: &'a Reach<'a>,
    /// The constants that each `mtctr` copies, as
    /// [`Bound::code_constants`] keeps them.
    ///
    /// [`Bound::code_constants`]: super::bound::Bound::code_constants
    writes: &'a BTreeMap<(usize, u64), Value>,
    /// Each `mtctr` that can also copy a constant that the scan dropped,
    /// so its CTR is one the scan cannot prove.
    lost_writes: &'a BTreeSet<(usize, u64)>,
    /// What the paths to each word bring in CTR and in its copies, in one
    /// vector for each code segment.
    at: Vec<Vec<CtrState>>,
}

impl<'a> CtrFlow<'a> {
    /// The flow before its first step, with no CTR on any word. The flow
    /// steps the words that `reach` holds.
    pub(super) fn new(
        code: &'a Code,
        writes: &'a BTreeMap<(usize, u64), Value>,
        lost_writes: &'a BTreeSet<(usize, u64)>,
        reach: &'a Reach<'a>,
    ) -> Self {
        Self {
            code,
            reach,
            writes,
            lost_writes,
            at: code
                .segments()
                .iter()
                .map(|segment| vec![CtrState::default(); segment.words.len()])
                .collect(),
        }
    }

    /// Steps from each word until no word changes.
    ///
    /// The flow joins what each `bclr` brings into one state. When that
    /// state changes, the flow joins it into the word after each link to
    /// the next word. So each `bclr` costs one join.
    ///
    /// The flow also keeps the GPRs and the stack slots that hold a copy
    /// at a `bclr`. When that set grows, the flow joins
    /// [`CtrState::lost_copies`] of it into the word after each call. The
    /// set has at most 32 GPRs and the slots in [`SLOT_REACH`], so it
    /// grows a finite number of times.
    pub(super) fn run(&mut self) {
        let code = self.code;
        let real_positions = |address: u64| {
            code.real_aliases(address)
                .flat_map(move |alias| positions(code, alias))
        };
        let returns: Vec<(usize, usize)> = after_next_word_links(code)
            .flat_map(real_positions)
            .collect();
        let call_returns: Vec<(usize, usize)> =
            after_calls(code).flat_map(real_positions).collect();
        let mut returned = CtrState::default();
        let mut returned_gprs = 0;
        let mut returned_slots = SlotSet::default();
        let mut injected = SlotSet::default();
        let mut work = Worklist::new(code, self.reach);
        while let Some((index, word)) = work.pop() {
            let segment = &code.segments()[index];
            let address = segment.vaddr + (word as u64) * 4;
            let raw = segment.words[word];
            let (next, taken) = self.steps((index, word), address, raw);
            let next = next.and_then(|ctr| Some((address.checked_add(4)?, ctr)));
            let taken = taken.into_iter().flat_map(|(target, ctr)| {
                code.real_aliases(target)
                    .map(move |alias| (alias, ctr.clone()))
            });
            for (to, ctr) in next.into_iter().chain(taken) {
                for position in positions(code, to) {
                    self.join_into(position, &ctr, &mut work);
                }
            }
            let Some(brought) = self.returned((index, word), raw) else {
                continue;
            };
            let gprs = brought.copies.mask();
            let slots = brought.slots.slots();
            let new_slot = !slots.is_subset(&returned_slots);
            if gprs & !returned_gprs != 0 || new_slot {
                returned_gprs |= gprs;
                returned_slots.join(&slots);
                let copies = CtrState::lost_copies(returned_gprs, &returned_slots);
                for &position in &call_returns {
                    self.inject(position, &copies, &injected, &mut work);
                }
                injected = returned_slots.clone();
            }
            if returned.join(&brought) {
                for &position in &returns {
                    self.join_into(position, &returned, &mut work);
                }
            }
        }
    }

    /// Joins `copies`, the lost copies that the returns bring, into the
    /// word at `position` after a call, and queues the word when its state
    /// changes.
    ///
    /// `injected` holds the lost slots of the last injection into each
    /// word after a call. The caller keeps each slot of `injected` in
    /// `copies`. A word whose lost slots share their runs with `injected`
    /// takes the lost slots of `copies` without a set comparison.
    fn inject(
        &mut self,
        position: (usize, usize),
        copies: &CtrState,
        injected: &SlotSet,
        work: &mut Worklist,
    ) {
        let (index, word) = position;
        if !self.reach.contains(index, word) {
            return;
        }
        let state = &mut self.at[index][word];
        let changed = if state.slots.lost.shares(injected) {
            let grew = !copies.slots.lost.shares(injected);
            state.slots.lost = copies.slots.lost.clone();
            state.copies.join(&copies.copies) || grew
        } else {
            state.join(copies)
        };
        if changed {
            work.push(position);
        }
    }

    /// Joins `state` into the word at `position`, and queues the word
    /// when its state changes.
    fn join_into(&mut self, position: (usize, usize), state: &CtrState, work: &mut Worklist) {
        let (index, word) = position;
        if self.reach.contains(index, word) && self.at[index][word].join(state) {
            work.push(position);
        }
    }

    /// The state that the word `raw` at `position` brings to the word
    /// where LR points, or `None` when the word is not a `bclr`.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
    /// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
    fn returned(&self, position: (usize, usize), raw: u32) -> Option<CtrState> {
        let Ok(PpuInstruction::Bclr { bo, .. }) = crate::decode::decode(raw) else {
            return None;
        };
        let (index, word) = position;
        let state = &self.at[index][word];
        let ctr = if bo & PPC_BO_BIT2 == 0 {
            state.ctr.unproved()
        } else {
            state.ctr.clone()
        };
        Some(CtrState {
            ctr,
            copies: state.copies.clone(),
            slots: state.slots.clone(),
        })
    }

    /// The state that the word `raw` at `address` passes to the next word,
    /// and the target of a direct branch with the state that the branch
    /// passes there.
    ///
    /// `position` holds the index of the segment and of the word there. A
    /// word that does not decode passes no state, as the scan of the code
    /// constants forgets each constant there. A call passes no copy to its
    /// target.
    fn steps(
        &self,
        position: (usize, usize),
        address: u64,
        raw: u32,
    ) -> (Option<CtrState>, Option<(u64, CtrState)>) {
        let (index, word) = position;
        let state = &self.at[index][word];
        let Ok(decoded) = crate::decode::decode(raw) else {
            return (None, None);
        };
        let (next, taken) = self.ctr_steps(index, address, raw, decoded, state);
        let (copies, slots) = state.after(&decoded);
        let with = |ctr| CtrState {
            ctr,
            copies: copies.clone(),
            slots: slots.clone(),
        };
        let taken = taken.map(|(target, ctr)| {
            let state = if calls(address, raw) {
                CtrState::default()
            } else {
                with(ctr)
            };
            (target, state)
        });
        (next.map(with), taken)
    }

    /// The CTR that the word `decoded` at `address` passes to the next
    /// word, and the target of a direct branch with the CTR that the
    /// branch passes there. The word is `raw`, in the segment at `index`,
    /// and `state` is what the paths to it bring.
    ///
    /// An `mtctr` of a GPR that holds a copy of CTR joins the copy into
    /// the constants that `writes` holds for the `mtctr`.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
    fn ctr_steps(
        &self,
        index: usize,
        address: u64,
        raw: u32,
        decoded: PpuInstruction,
        state: &CtrState,
    ) -> (Option<Ctr>, Option<(u64, Ctr)>) {
        let ctr = &state.ctr;
        match decoded {
            PpuInstruction::Mtctr { rs } => {
                let written = self.writes.get(&(index, address)).cloned();
                let mut ctr =
                    written.map_or_else(Ctr::default, |value| Ctr::written(address, value));
                if let Some(copy) = state.copies.get(rs) {
                    ctr.join(&copy.rewritten(address));
                }
                if self.lost_writes.contains(&(index, address)) {
                    ctr.join(&Ctr::lost());
                }
                (Some(ctr), None)
            }
            PpuInstruction::B { offset, aa, link } => {
                let target = branch_target(address, i64::from(offset), aa);
                let next = branched(address, raw, 0b10100, link, ctr);
                (next, Some((target, taken(address, raw, 0b10100, ctr))))
            }
            PpuInstruction::Bc {
                bo,
                offset,
                aa,
                link,
                ..
            } => {
                let target = branch_target(address, i64::from(offset), aa);
                let next = branched(address, raw, bo, link, ctr);
                (next, Some((target, taken(address, raw, bo, ctr))))
            }
            PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. } => {
                (branched(address, raw, bo, link, ctr), None)
            }
            _ if never_falls_through(&decoded) => (None, None),
            _ => (Some(ctr.clone()), None),
        }
    }

    /// Adds the target of each `bcctr` to `targets`, by the word that
    /// wrote each constant that the paths to it bring in CTR, and returns
    /// the [`IndirectBound::lost_targets`].
    ///
    /// [`IndirectBound::lost_targets`]: super::IndirectBound::lost_targets
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    pub(super) fn bcctr_targets(
        &self,
        targets: &mut BTreeMap<u64, BTreeSet<u64>>,
    ) -> BTreeSet<u64> {
        let mut lost = BTreeSet::new();
        for (index, (segment, states)) in self.code.segments().iter().zip(&self.at).enumerate() {
            for (word, ((address, raw), state)) in segment.addressed().zip(states).enumerate() {
                if !self.reach.contains(index, word)
                    || !matches!(crate::decode::decode(raw), Ok(PpuInstruction::Bcctr { .. }))
                {
                    continue;
                }
                let ctr = &state.ctr;
                for (&by, value) in &ctr.known {
                    targets
                        .entry(by)
                        .or_default()
                        .extend(value.constants().iter().map(|&ctr| ctr & !0b11));
                }
                if ctr.lost {
                    lost.insert(address);
                }
            }
        }
        lost
    }
}

/// The CTR that a direct branch at `address` passes to its target. The
/// branch is the word `raw`, with the BO field `bo`, and `ctr` is what
/// the paths to the branch bring.
///
/// A decrement leaves CTR at a value that the scan cannot prove, because
/// a loop can run it more than once. The flow does not follow CTR into
/// the routine that a call runs. The bound takes the routine to start
/// with a CTR that no word of the scan writes, as at an indirect entry.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
fn taken(address: u64, raw: u32, bo: u8, ctr: &Ctr) -> Ctr {
    if calls(address, raw) {
        return Ctr::default();
    }
    if bo & PPC_BO_BIT2 == 0 {
        ctr.unproved()
    } else {
        ctr.clone()
    }
}

/// The CTR that a branch at `address` passes to the next word, or `None`
/// when it cannot fall through. The branch is the word `raw`, with the
/// BO field `bo`, and it writes LR when `link` is set. `ctr` is what the
/// paths to the branch bring.
///
/// A `bc` with BO 1a00t that does not link falls through only when the
/// decremented CTR is 0. A `bcctr` then reaches address 0 in either
/// mode, and the `bc` is the word that wrote CTR. The taken path brings
/// its own CTR to the target.
///
/// Each other decrement leaves CTR at a value that the scan cannot prove.
/// A call leaves CTR at any value, because its routine can write CTR
/// before it returns.
///
/// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t decrements CTR, then branches if CTR M:63 is not 0, with M = 0 in 64-bit mode and M = 32 in 32-bit mode.
/// [PPC-Book1 p:20 s:2.4.1] In 32-bit mode a branch sets the high-order 32 bits of its target address to 0.
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
fn branched(address: u64, raw: u32, bo: u8, link: bool, ctr: &Ctr) -> Option<Ctr> {
    if !falls_through(bo, link) {
        return None;
    }
    Some(if calls(address, raw) {
        Ctr::lost()
    } else if bo & PPC_BO_BIT2 != 0 {
        ctr.clone()
    } else if bo & 0b10110 == 0b10000 && !link {
        Ctr::written(address, Value::One(0))
    } else {
        ctr.unproved()
    })
}

/// The words that [`CtrFlow::run`] has yet to step from, by the index of
/// the segment and of the word there.
struct Worklist {
    /// Whether each word is in `pending`, in one vector for each segment.
    queued: Vec<Vec<bool>>,
    pending: Vec<(usize, usize)>,
}

impl Worklist {
    /// A worklist that holds each word of `code` that `reach` holds, with
    /// the first word next.
    fn new(code: &Code, reach: &Reach<'_>) -> Self {
        let queued: Vec<Vec<bool>> = code
            .segments()
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                (0..segment.words.len())
                    .map(|word| reach.contains(index, word))
                    .collect()
            })
            .collect();
        let pending = queued
            .iter()
            .enumerate()
            .flat_map(|(index, words)| {
                words
                    .iter()
                    .enumerate()
                    .filter(|&(_, &queued)| queued)
                    .map(move |(word, _)| (index, word))
            })
            .rev()
            .collect();
        Self { queued, pending }
    }

    /// Takes the next word.
    fn pop(&mut self) -> Option<(usize, usize)> {
        let (index, word) = self.pending.pop()?;
        self.queued[index][word] = false;
        Some((index, word))
    }

    /// Queues the word at `position`, unless it is in the list.
    fn push(&mut self, position: (usize, usize)) {
        let (index, word) = position;
        if !self.queued[index][word] {
            self.queued[index][word] = true;
            self.pending.push(position);
        }
    }
}

/// The index of the segment and of the word of each segment that holds a
/// word at `address`.
pub(super) fn positions(code: &Code, address: u64) -> impl Iterator<Item = (usize, usize)> + '_ {
    code.segments()
        .iter()
        .enumerate()
        .filter_map(move |(index, segment)| {
            let offset = address.checked_sub(segment.vaddr)?;
            if offset % 4 != 0 {
                return None;
            }
            let word = usize::try_from(offset / 4).ok()?;
            (word < segment.words.len()).then_some((index, word))
        })
}

#[cfg(test)]
#[path = "../tests/indirect_entry_ctr_flow_bound_tests.rs"]
mod bound_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_ctr_flow_inject_tests.rs"]
mod inject_tests;
