//! What the paths to each word bring in CTR, in the GPRs that hold a copy
//! of CTR, and in the stack slots that hold one.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_BIT2;

use super::cells::{cell_writes, read_cells, Cells};
use super::constants::{ctr_flow_spr_move, never_falls_through, GprStep, SprMove, Step, Value};
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

/// The address of the word after each call of `code`, where its routine
/// returns.
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
/// The default state holds no path. It is the state of a word before a
/// path reaches it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ctr {
    /// The constants that the paths bring in CTR, by the address of the
    /// word that wrote each one.
    known: BTreeMap<u64, Value>,
    /// Whether a path brings a value that the scan cannot prove.
    lost: bool,
    /// Whether a path brings a value of a class whose words the bound
    /// names, from an `mtctr` of [`CtrWrites::ranged`].
    ///
    /// Only CTR itself holds such a value. A copy of it is lost, so a
    /// copy that a word changes cannot leave the class.
    ranged: bool,
    /// Whether a path brings a CTR that no word of the scan writes, such
    /// as a value that the code loads.
    ///
    /// The bound takes such a CTR to hold a named address, or in real
    /// mode a word of [`IndirectBound::real_named`]. So a `bcctr` of it
    /// adds no target. A step that changes it gives a value that the
    /// scan cannot prove.
    ///
    /// Only CTR itself holds such a value. The state records no copy of
    /// it.
    ///
    /// [`IndirectBound::real_named`]: super::IndirectBound::real_named
    unwritten: bool,
}

impl Ctr {
    /// CTR that no word of the scan writes.
    fn unwritten() -> Self {
        Self {
            unwritten: true,
            ..Self::default()
        }
    }

    /// CTR after the word at `by` writes the constants `value`.
    fn written(by: u64, value: Value) -> Self {
        Self {
            known: BTreeMap::from([(by, value)]),
            ..Self::default()
        }
    }

    /// CTR after a step that can leave it at any value.
    fn lost() -> Self {
        Self {
            lost: true,
            ..Self::default()
        }
    }

    /// CTR after an `mtctr` that copies a value of a class whose words
    /// the bound names.
    fn ranged() -> Self {
        Self {
            ranged: true,
            ..Self::default()
        }
    }

    /// CTR after a step that keeps no constant of it.
    ///
    /// Each of these values of a path becomes a value that the scan
    /// cannot prove:
    /// - a constant;
    /// - a value of a class;
    /// - a CTR that no word of the scan writes.
    ///
    /// A state that holds no path stays so.
    fn unproved(&self) -> Self {
        Self {
            lost: self.lost || self.ranged || self.unwritten || !self.known.is_empty(),
            ..Self::default()
        }
    }

    /// A copy of CTR in a GPR.
    ///
    /// The copy holds a value of a class as a value that the scan cannot
    /// prove.
    fn copied(&self) -> Self {
        Self {
            known: self.known.clone(),
            lost: self.lost || self.ranged,
            ..Self::default()
        }
    }

    /// A copy of CTR after a word applies `step` to each of its constants.
    ///
    /// A step that changes a constant leaves a value that the scan cannot
    /// prove, because a loop can run the step more than once.
    fn stepped(&self, step: Step) -> Self {
        let unchanged = !self.ranged
            && self
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
            lost: self.lost || self.ranged,
            ..Self::default()
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
            ranged: self.ranged,
            unwritten: self.unwritten,
        }
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let changed = (other.lost && !self.lost)
            || (other.ranged && !self.ranged)
            || (other.unwritten && !self.unwritten);
        self.lost |= other.lost;
        self.ranged |= other.ranged;
        self.unwritten |= other.unwritten;
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
            ..Ctr::default()
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

/// What the paths to one word bring in CTR and in the registers and stack
/// slots that hold a copy of CTR.
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
    /// The copy of CTR in LR.
    lr: Ctr,
    /// The cells of CR, XER and the SPRs of [`ctr_flow_spr_move`] that can
    /// hold a part of a copy.
    ///
    /// A move out of a register takes the cells that it reads. A move can
    /// take only a part of the value, so a move out gives a copy that the
    /// scan cannot prove. A cell keeps no constant, so a copy with
    /// constants that the scan knows also sets it.
    moved: Cells,
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
            ..Self::default()
        }
    }

    /// Joins `other` into `self`, and returns whether `self` changed.
    fn join(&mut self, other: &Self) -> bool {
        let ctr = self.ctr.join(&other.ctr);
        let copies = self.copies.join(&other.copies);
        let lr = self.lr.join(&other.lr);
        let moved = !other.moved.without(self.moved).is_empty();
        self.moved = self.moved.union(other.moved);
        self.slots.join(&other.slots) || ctr || copies || lr || moved
    }

    /// The copies in the registers and in the stack slots after the word
    /// `decoded`, with no CTR.
    ///
    /// These words copy CTR or a copy into a GPR:
    /// - `mfctr`, which copies CTR;
    /// - `mflr`, which copies the copy in LR;
    /// - a word that the scan follows a constant through, from a GPR
    ///   that holds a copy;
    /// - a move out of CR, XER or an SPR, as [`Self::moved_out`] gives;
    /// - a load at a constant offset from r1, from the slots that hold a
    ///   copy there.
    ///
    /// Each other word that surely writes a GPR removes its copy. A word can
    /// also copy into a GPR that it does not surely write, such as an
    /// invalid form of a load. That word joins the copy into the copy of
    /// the GPR. A store at a constant offset from r1 writes its slot, as
    /// [`SlotCopies::store`] gives. A word that moves r1 by a constant
    /// moves each slot, and each other word that surely writes r1 removes
    /// each slot. [`Self::lr_after`] gives LR, and [`Self::moved_after`]
    /// gives the cells.
    ///
    /// [PPC-Book1 p:82 s:3.3.13] `mflr` and `mfctr`, forms of `mfspr`, copy LR or CTR into RT.
    fn after(&self, decoded: &PpuInstruction) -> Self {
        let mut copied: Vec<(u8, Ctr)> = match *decoded {
            PpuInstruction::Mfctr { rt } => vec![(rt, self.ctr.copied())],
            PpuInstruction::Mflr { rt } => vec![(rt, self.lr.clone())],
            _ => match GprStep::of(*decoded) {
                Some(step) => step
                    .source
                    .and_then(|source| self.copies.get(source))
                    .map(|copy| (step.rt, copy.stepped(step.step)))
                    .into_iter()
                    .collect(),
                None => self.moved_out(decoded).into_iter().collect(),
            },
        };
        let mut slots = self.slots.clone();
        let access = StackAccess::of(decoded);
        if let Some(access) = &access {
            self.stack_step(access, &mut slots, &mut copied);
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
        Self {
            ctr: Ctr::default(),
            copies,
            slots,
            lr: self.lr_after(decoded),
            moved: self.moved_after(decoded),
        }
    }

    /// The copy that `decoded` gives a GPR when it moves a cell that holds
    /// a copy out of CR, XER or an SPR of [`ctr_flow_spr_move`], or `None`
    /// for any other word.
    ///
    /// The copy is one that the scan cannot prove. A word that computes a
    /// GPR from a copy gives no copy, so it drops a copy that the scan
    /// cannot prove. The flow keeps no such result, because the return
    /// injection would put it after every call of the kernel.
    fn moved_out(&self, decoded: &PpuInstruction) -> Option<(u8, Ctr)> {
        match ctr_flow_spr_move(decoded) {
            Some(SprMove::OutOf(rt)) if self.moved.meets(read_cells(decoded)) => {
                Some((rt, Ctr::lost()))
            }
            _ => None,
        }
    }

    /// The copy of CTR in LR after the word `decoded`.
    ///
    /// An `mtlr` puts the copy of its GPR into LR, or no copy. A branch
    /// that links writes the address of the next word into LR, so LR then
    /// holds no copy.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtlr` and `mtctr`, forms of `mtspr`, copy RS into LR or CTR.
    /// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
    fn lr_after(&self, decoded: &PpuInstruction) -> Ctr {
        match *decoded {
            PpuInstruction::Mtlr { rs } => self.copies.get(rs).unwrap_or_default(),
            PpuInstruction::B { link: true, .. }
            | PpuInstruction::Bc { link: true, .. }
            | PpuInstruction::Bclr { link: true, .. }
            | PpuInstruction::Bcctr { link: true, .. } => Ctr::default(),
            _ => self.lr.clone(),
        }
    }

    /// The cells that hold a copy after the word `decoded`.
    ///
    /// Each write of [`cell_writes`] sets its cells when it reads a GPR
    /// that holds a copy, or a cell that holds one. A write of each bit
    /// of a cell from no copy clears the cell.
    ///
    /// A copy with constants that the scan knows also sets a cell. The
    /// image need not name such a constant, for example a part of CTR
    /// that a load takes back from the stack, so the bound does not hold
    /// it as an entry.
    ///
    /// [PPC-Book1 p:81 s:3.3.13] `mtspr` copies RS into the SPR that its SPR field names.
    fn moved_after(&self, decoded: &PpuInstruction) -> Cells {
        let copies = self.copies.mask();
        cell_writes(decoded, ctr_flow_spr_move(decoded))
            .iter()
            .fold(self.moved, |moved, write| {
                let set = write.gprs & copies != 0 || self.moved.meets(write.from);
                moved.written(write, set)
            })
    }

    /// Writes into `slots` each slot that the store `access` writes, or
    /// adds to `copied` the copy that the load `access` gives each GPR.
    fn stack_step(
        &self,
        access: &StackAccess,
        slots: &mut SlotCopies,
        copied: &mut Vec<(u8, Ctr)>,
    ) {
        for part in &access.parts {
            match (part.gpr, access.load) {
                (None, Load::None) => slots.store(part.slot, None),
                (None, _) => {}
                (Some(gpr), Load::None) => {
                    let copy = self.copies.get(gpr).map(|copy| part.stored(copy));
                    slots.store(part.slot, copy.as_ref());
                }
                (Some(gpr), load) => {
                    copied.push((gpr, part.loaded(self.slots.loaded(part.slot, load))));
                }
            }
        }
    }
}

/// The registers and the stack slots that hold a copy at a return, which
/// the flow injects into the word after each call.
#[derive(Debug, Default)]
struct Returned {
    /// The GPRs that hold a copy, as a mask with bit `n` set for GPR `n`.
    gprs: u32,
    /// The slots that hold a copy, as [`SlotSet::widened`] gives them.
    slots: SlotSet,
    /// Whether LR holds a copy.
    lr: bool,
    /// The cells of [`CtrState::moved`] that hold a copy.
    moved: Cells,
}

impl Returned {
    /// Adds the registers and the slots that hold a copy in `brought`, the
    /// state of a return, and returns whether the set grew.
    fn grow(&mut self, brought: &CtrState) -> bool {
        let gprs = brought.copies.mask();
        let slots = brought.slots.slots();
        let lr = brought.lr != Ctr::default();
        let grew = gprs & !self.gprs != 0
            || !slots.is_subset(&self.slots)
            || (lr && !self.lr)
            || !brought.moved.without(self.moved).is_empty();
        if grew {
            self.gprs |= gprs;
            self.slots = self.slots.widened(&slots, &SLOT_REACH);
            self.lr |= lr;
            self.moved = self.moved.union(brought.moved);
        }
        grew
    }

    /// A state with no CTR, and a copy that the scan cannot prove in each
    /// register and slot of the set.
    fn lost_copies(&self) -> CtrState {
        CtrState {
            lr: if self.lr { Ctr::lost() } else { Ctr::default() },
            moved: self.moved,
            ..CtrState::lost_copies(self.gprs, &self.slots)
        }
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

/// The slot of one register that a word loads or stores at a constant
/// offset from r1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Part {
    slot: Slot,
    /// The GPR, or `None` for the FPR of a floating-point load or store.
    gpr: Option<u8>,
    /// The number of low-order bits of the GPR below the bytes that the
    /// word moves.
    ///
    /// The value is 0 for each word other than a string word. A string
    /// word moves the high-order bytes of the low-order word of its last
    /// GPR.
    shift: u32,
}

impl Part {
    /// The bytes that a store of the copy `copy` in the GPR writes, as
    /// the low-order bytes of a copy.
    fn stored(&self, copy: Ctr) -> Ctr {
        if self.shift == 0 {
            copy
        } else {
            copy.part(|value| value >> self.shift)
        }
    }

    /// The GPR after a load puts the bytes `loaded` into it.
    ///
    /// `loaded` holds the bytes as the low-order bytes of a copy.
    fn loaded(&self, loaded: Ctr) -> Ctr {
        if self.shift == 0 {
            loaded
        } else {
            loaded.part(|value| value << self.shift)
        }
    }
}

/// A load or a store that a word makes at a constant offset from r1.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StackAccess {
    load: Load,
    /// The slot of each register that the word loads or stores.
    parts: Vec<Part>,
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
            I::Lswi { rt, ra, nb } => return Self::string(Load::Zero, rt, ra, nb),
            I::Stswi { rs, ra, nb } => return Self::string(Load::None, rs, ra, nb),
            _ => return None,
        };
        let offset = i64::from(imm);
        (ra == 1).then(|| Self {
            load,
            parts: vec![Part {
                slot: (offset, width),
                gpr,
                shift: 0,
            }],
            moves_r1: update.then_some(offset),
        })
    }

    /// The access of an `lmw` or an `stmw` of the GPRs from `first`
    /// through 31 at `imm` from RA.
    fn multiple(load: Load, first: u8, ra: u8, imm: i16) -> Option<Self> {
        (ra == 1).then(|| Self {
            load,
            parts: (first..32)
                .map(|gpr| Part {
                    slot: (i64::from(imm) + 4 * i64::from(gpr - first), 4),
                    gpr: Some(gpr),
                    shift: 0,
                })
                .collect(),
            moves_r1: None,
        })
    }

    /// The access of an `lswi` or an `stswi` of `nb` bytes from the GPRs
    /// from `first` up at 0 from RA.
    ///
    /// Each GPR takes four bytes, and the last GPR can take fewer. A GPR
    /// moves the high-order bytes of its low-order word, from the left.
    ///
    /// [PPC-Book1 p:48 s:3.3.6] `lswi` loads n bytes from EA = (RA|0) into the low-order words of RT onward, left to right, with GPR 0 after GPR 31, and sets the high-order word and the unfilled bytes to 0.
    /// [PPC-Book1 p:49 s:3.3.6] `stswi` stores n bytes from the low-order words of RS onward, left to right, to EA = (RA|0).
    fn string(load: Load, first: u8, ra: u8, nb: u8) -> Option<Self> {
        let bytes = if nb == 0 { 32 } else { u32::from(nb) };
        (ra == 1).then(|| Self {
            load,
            parts: (0..bytes.div_ceil(4))
                .map(|n| {
                    let width = (bytes - 4 * n).min(4);
                    Part {
                        slot: (i64::from(4 * n), width as u8),
                        gpr: Some(((u32::from(first) + n) % 32) as u8),
                        shift: 8 * (4 - width),
                    }
                })
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
/// - from a return, each word at the real address of the word after a
///   `b` or a `bc` that links to the next word.
///
/// A return is a `bclr` or a `bcctr` that does not link, as
/// [`CtrFlow::returned`] gives.
///
/// The flow does not follow CTR or a copy of it to these words:
/// - the first word of the routine that a call runs;
/// - the target of an indirect branch, an `rfid` or an interrupt.
///
/// A path to one of these words brings a CTR that no word of the scan
/// writes, which the bound takes to hold a named address. So each such
/// word, and each word that [`Reach::started`] gives, starts a path with
/// that CTR. The word after a call gets a CTR that the scan cannot
/// prove. The word after an `sc` gets the CTR before the `sc`, so the
/// flow takes the handler to return with CTR unchanged.
///
/// A `b` or a `bc` that links to the next word calls no routine, so the
/// next word gets the CTR before the branch. A return can reach that
/// word later with another CTR, so that word also gets what each return
/// brings.
///
/// The flow also follows each copy of CTR in a GPR on the same paths. An
/// `mtctr` of a GPR that holds a copy puts the copy back into CTR, so a
/// CTR that the scan cannot prove stays so through the copy.
///
/// The flow also follows each copy through these registers and slots, as
/// [`CtrState::after`] gives:
/// - a stack slot at a constant offset from r1;
/// - LR, from an `mtlr` into a `bclr` or an `mflr`;
/// - the cells of CR, XER and the SPRs of [`ctr_flow_spr_move`], which
///   keep each copy as one that the scan cannot prove.
///
/// The word after a call gets these copies:
/// - each copy before the call, because its routine can return the
///   register or the slot unchanged;
/// - a copy of a CTR that the scan cannot prove, in each register and
///   slot that holds a copy at a return, because its routine can make
///   that copy. The slots can also include other slots of the same width
///   and residue, as [`CtrFlow::run`] gives.
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
    writes: CtrWrites<'a>,
    /// What the paths to each word bring in CTR and in its copies, in one
    /// vector for each code segment.
    at: Vec<Vec<CtrState>>,
}

/// What each `mtctr` copies into CTR, as [`Bound::code_constants`] keeps
/// it, by segment index and address.
///
/// [`Bound::code_constants`]: super::bound::Bound::code_constants
#[derive(Clone, Copy)]
pub(super) struct CtrWrites<'a> {
    /// The constants that each `mtctr` copies.
    pub(super) constants: &'a BTreeMap<(usize, u64), Value>,
    /// Each `mtctr` that can also copy a constant that the scan dropped,
    /// so its CTR is one the scan cannot prove.
    pub(super) lost: &'a BTreeSet<(usize, u64)>,
    /// Each `mtctr` that can also copy a value of a class whose words the
    /// bound names.
    pub(super) ranged: &'a BTreeSet<(usize, u64)>,
}

impl<'a> CtrFlow<'a> {
    /// The flow before its first step. The flow steps the words that
    /// `reach` holds.
    ///
    /// Each word that [`Reach::started`] gives starts with a CTR that no
    /// word of the scan writes. Each other word starts with no path.
    pub(super) fn new(code: &'a Code, writes: CtrWrites<'a>, reach: &'a Reach<'a>) -> Self {
        let at = code
            .segments()
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                (0..segment.words.len())
                    .map(|word| CtrState {
                        ctr: if reach.started(index, word) {
                            Ctr::unwritten()
                        } else {
                            Ctr::default()
                        },
                        ..CtrState::default()
                    })
                    .collect()
            })
            .collect();
        Self {
            code,
            reach,
            writes,
            at,
        }
    }

    /// Steps from each word until no word changes, and returns the number
    /// of words that it stepped.
    ///
    /// The flow joins what each return brings into one state. When that
    /// state changes, the flow joins it into the word after each link to
    /// the next word. So each return costs one join.
    ///
    /// The flow also keeps the registers and the stack slots that hold a
    /// copy at a return, as [`Returned`] gives them. When that set grows,
    /// the flow joins [`Returned::lost_copies`] of it into the word after
    /// each call.
    ///
    /// A move of r1 between the word after a call and a return brings the
    /// injected slots back at new offsets, one move at a time. So the flow
    /// keeps the slots as [`SlotSet::widened`] gives them. The set grows at
    /// most 32 times for the GPRs, once for LR, once for each of the 11
    /// cells, and at most four times for each width
    /// and residue of a slot in [`SLOT_REACH`]. An `lmw` or an `stmw` near
    /// the top of the displacements can put a slot past the end of
    /// [`SLOT_REACH`], and each such slot can add one more growth. The
    /// displacement and the 32 GPRs bound those slots, so the set grows a
    /// finite number of times.
    ///
    /// [PPC-Book1 p:46 s:3.3.5] `lmw` and `stmw` move consecutive words from EA up, with EA = (RA|0) + EXTS(D).
    pub(super) fn run(&mut self) -> usize {
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
        let mut returned_copies = Returned::default();
        let mut injected = SlotSet::default();
        let mut work = Worklist::new(code, self.reach);
        let mut stepped = 0;
        while let Some((index, word)) = work.pop() {
            stepped += 1;
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
            if returned_copies.grow(&brought) {
                let copies = returned_copies.lost_copies();
                for &position in &call_returns {
                    self.inject(position, &copies, &injected, &mut work);
                }
                injected = returned_copies.slots.clone();
            }
            if returned.join(&brought) {
                for &position in &returns {
                    self.join_into(position, &returned, &mut work);
                }
            }
        }
        stepped
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
            let gprs = state.copies.join(&copies.copies);
            let lr = state.lr.join(&copies.lr);
            let moved = !copies.moved.without(state.moved).is_empty();
            state.moved = state.moved.union(copies.moved);
            gprs || lr || moved || grew
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

    /// The state that the word `raw` at `position` brings when it returns
    /// from a routine, or `None` when the word cannot return.
    ///
    /// These words can return:
    /// - a `bclr`;
    /// - a `bcctr` that does not link, because a routine can copy LR into
    ///   CTR and branch there.
    ///
    /// The flow cannot tell which `bcctr` holds a return point in CTR, so
    /// it takes each one that does not link as a return.
    ///
    /// A `bclr` brings no copy in LR. Its target is the address in LR, and
    /// that address is the word that it returns to.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00, and `bcctr` branches to CTR 0:61 || 0b00.
    /// [PPC-Book1 p:24 s:2.4] A `bc` with BO bit 2 at 0 decrements CTR.
    fn returned(&self, position: (usize, usize), raw: u32) -> Option<CtrState> {
        let (bo, through_lr) = match crate::decode::decode(raw) {
            Ok(PpuInstruction::Bclr { bo, .. }) => (bo, true),
            Ok(PpuInstruction::Bcctr {
                bo, link: false, ..
            }) => (bo, false),
            _ => return None,
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
            lr: if through_lr {
                Ctr::default()
            } else {
                state.lr.clone()
            },
            moved: state.moved,
        })
    }

    /// The state that the word `raw` at `address` passes to the next word,
    /// and the target of a direct branch with the state that the branch
    /// passes there.
    ///
    /// `position` holds the index of the segment and of the word there. A
    /// word that does not decode passes no state, as the scan of the code
    /// constants forgets each constant there. A call passes to its target
    /// a CTR that no word of the scan writes, and no copy in a GPR, a slot
    /// or LR. It passes the cells of [`CtrState::moved`], because CR, XER
    /// and the SPRs keep their values across the branch.
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
        let after = state.after(&decoded);
        let with = |ctr| CtrState {
            ctr,
            ..after.clone()
        };
        let taken = taken.map(|(target, ctr)| {
            let state = if calls(address, raw) {
                CtrState {
                    ctr,
                    moved: after.moved,
                    ..CtrState::default()
                }
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
    /// the constants that `writes` holds for the `mtctr`. An `mtctr` with
    /// no constant, no copy and no lost or ranged value copies a value
    /// that no word of the scan writes.
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
                let written = self.writes.constants.get(&(index, address)).cloned();
                let mut ctr =
                    written.map_or_else(Ctr::default, |value| Ctr::written(address, value));
                if let Some(copy) = state.copies.get(rs) {
                    ctr.join(&copy.rewritten(address));
                }
                if self.writes.lost.contains(&(index, address)) {
                    ctr.join(&Ctr::lost());
                }
                if self.writes.ranged.contains(&(index, address)) {
                    ctr.join(&Ctr::ranged());
                }
                if ctr == Ctr::default() {
                    ctr = Ctr::unwritten();
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
        self.register_targets(
            targets,
            |decoded| matches!(decoded, PpuInstruction::Bcctr { .. }),
            |state| &state.ctr,
        )
    }

    /// Adds the target of each `bclr` that the copy of CTR in LR gives to
    /// `targets`, by the word that wrote each constant, and returns the
    /// address of each `bclr` whose copy the scan cannot prove.
    ///
    /// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
    pub(super) fn bclr_targets(&self, targets: &mut BTreeMap<u64, BTreeSet<u64>>) -> BTreeSet<u64> {
        self.register_targets(
            targets,
            |decoded| matches!(decoded, PpuInstruction::Bclr { .. }),
            |state| &state.lr,
        )
    }

    /// Adds to `targets` the constants that `register` gives at each
    /// branch for which `branch` holds, by the word that wrote each one,
    /// and returns the address of each such branch whose value the scan
    /// cannot prove.
    fn register_targets(
        &self,
        targets: &mut BTreeMap<u64, BTreeSet<u64>>,
        branch: fn(&PpuInstruction) -> bool,
        register: fn(&CtrState) -> &Ctr,
    ) -> BTreeSet<u64> {
        let mut lost = BTreeSet::new();
        for (index, (segment, states)) in self.code.segments().iter().zip(&self.at).enumerate() {
            for (word, ((address, raw), state)) in segment.addressed().zip(states).enumerate() {
                if !self.reach.contains(index, word)
                    || !crate::decode::decode(raw).is_ok_and(|decoded| branch(&decoded))
                {
                    continue;
                }
                let ctr = register(state);
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
        return Ctr::unwritten();
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

#[cfg(test)]
#[path = "../tests/indirect_entry_ctr_flow_ranged_tests.rs"]
mod ranged_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_ctr_flow_widen_tests.rs"]
mod widen_tests;

#[cfg(test)]
#[path = "../tests/indirect_entry_ctr_flow_unwritten_tests.rs"]
mod unwritten_tests;
