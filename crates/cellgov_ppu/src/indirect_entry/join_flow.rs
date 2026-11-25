//! What the paths to each join word bring in the GPRs and in LR.

use std::collections::{BTreeMap, BTreeSet};

use super::constants::{ends_path, Constants, Value};
use super::ctr_flow::{calls, links_to_next_word, positions};
use super::lost::{join_origins, LostGprs, Origins};
use super::toc::WordTocs;
use crate::caller_census::{JoinDrop, JoinRegister};
use crate::hypercall_cfg::{branch_target, Code};
use crate::instruction::PpuInstruction;

/// The largest number of constants that the flow keeps for one GPR, or
/// for LR, at a join word whose bound [`Bounds`] does not raise.
///
/// When the paths to a join word bring more constants than its bound in
/// a register, the join keeps the constants that it holds. It marks the
/// register as lost: the register can also hold a constant that the flow
/// dropped. So a loop that steps a constant adds a finite number of
/// constants, and the flow ends.
///
/// A dropped constant can be the address that a later `bcctr`, `bclr` or
/// `rfid` reaches. So each of these can reach an address that the scan
/// does not know:
/// - a CTR copied from a lost GPR;
/// - a lost LR at a `bclr`;
/// - a lost SRR0 at an `rfid`.
const JOIN_LIMIT: usize = 16;

/// The largest bound that [`Bounds::raise`] gives one register at one
/// join word.
pub(super) const RAISED_LIMIT: usize = 64;

/// What the paths to one join word bring in the GPRs and in LR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Brought {
    /// The constants of each GPR that holds one, by GPR number in
    /// ascending order.
    gprs: Vec<(u8, Value)>,
    /// The origins of each GPR that can also hold a constant that the
    /// flow dropped.
    lost_gprs: LostGprs,
    /// The constants in LR, by the address of the word that wrote each
    /// one.
    lr: BTreeMap<u64, Value>,
    /// The origins of LR when it can also hold a constant that the flow
    /// dropped.
    lr_lost: Option<Origins>,
    /// The origins of SRR0 and of SRR1 when each can also hold a
    /// constant that the flow dropped. The flow brings no SRR0 or SRR1
    /// constant to a join word.
    srrs_lost: [Option<Origins>; 2],
}

impl Brought {
    /// The constants on the join word, before its TOC enters r2.
    fn entered(&self) -> Constants {
        let mut constants = Constants::default();
        for (gpr, value) in &self.gprs {
            constants.gprs[usize::from(*gpr)] = Some(value.clone());
        }
        constants.lost_gprs.clone_from(&self.lost_gprs);
        constants.lr.clone_from(&self.lr);
        constants.lr_lost.clone_from(&self.lr_lost);
        [constants.srr0_lost, constants.srr1_lost] = self.srrs_lost.clone();
        constants
    }

    /// Joins the constants that one path brings into `self`, and returns
    /// whether `self` changed.
    ///
    /// A join that would leave more constants in a register than the
    /// bound of the register keeps the constants of the register. It
    /// records the drop in `drops` and marks the register as lost with
    /// that drop. The join also takes the origins of each register that
    /// the path marks as lost.
    fn join(&mut self, path: &Constants, join: JoinAt<'_>, drops: &mut Drops) -> bool {
        let mut changed = self.lost_gprs.join(&path.lost_gprs);
        for gpr in 0..32u8 {
            let Some(value) = path.gpr(gpr) else {
                continue;
            };
            let register = JoinRegister::Gpr(gpr);
            let limit = join.limit(register);
            let joined = match self.gprs.binary_search_by_key(&gpr, |&(own, _)| own) {
                Ok(at) => join_limited(&mut self.gprs[at].1, value, limit),
                Err(at) if value.constants().len() <= limit => {
                    self.gprs.insert(at, (gpr, value.clone()));
                    Joined::Grew
                }
                Err(_) => Joined::Overflowed,
            };
            match joined {
                Joined::Held => {}
                Joined::Grew => changed = true,
                Joined::Overflowed => {
                    let own = self.gpr(gpr);
                    let dropped = missing(own, value).map(|constant| (0, constant));
                    let origins = drops.record(join.slot, register, dropped);
                    changed |= self.lost_gprs.join_gpr(gpr, &origins);
                }
            }
        }
        changed |= join_origins(&mut self.lr_lost, path.lr_lost.as_ref());
        let limit = join.limit(JoinRegister::Lr);
        let mut dropped = Vec::new();
        for (&by, value) in &path.lr {
            let own = self.lr.get(&by);
            let joined_len = match own {
                Some(own) if own.holds_all(value) => continue,
                Some(own) => joined_len(own, value, limit),
                None => value.constants().len(),
            };
            let others = self.lr_count() - own.map_or(0, |own| own.constants().len());
            if others + joined_len <= limit {
                let joined = own.map_or_else(|| value.clone(), |own| own.clone().union(value));
                self.lr.insert(by, joined);
                changed = true;
            } else {
                dropped.extend(missing(own, value).map(|constant| (by, constant)));
            }
        }
        if !dropped.is_empty() {
            let origins = drops.record(join.slot, JoinRegister::Lr, dropped.into_iter());
            changed |= join_origins(&mut self.lr_lost, Some(&origins));
        }
        changed |= join_origins(&mut self.srrs_lost[0], path.srr0_lost.as_ref());
        changed |= join_origins(&mut self.srrs_lost[1], path.srr1_lost.as_ref());
        changed
    }

    /// The constants of `gpr`, when it holds any.
    fn gpr(&self, gpr: u8) -> Option<&Value> {
        let at = self.gprs.binary_search_by_key(&gpr, |&(own, _)| own).ok()?;
        Some(&self.gprs[at].1)
    }

    /// The number of constants in LR, over each word that wrote one.
    fn lr_count(&self) -> usize {
        self.lr.values().map(|value| value.constants().len()).sum()
    }

    /// Whether `register` holds `item`: a constant of a GPR keyed by 0,
    /// or a constant of LR keyed by the word that wrote it.
    fn holds(&self, register: JoinRegister, (by, constant): (u64, u64)) -> bool {
        let value = match register {
            JoinRegister::Gpr(gpr) => self.gpr(gpr),
            JoinRegister::Lr => self.lr.get(&by),
        };
        value.is_some_and(|value| value.constants().binary_search(&constant).is_ok())
    }

    /// The number of constants that `register` holds, as [`Self::holds`]
    /// counts them.
    fn count(&self, register: JoinRegister) -> usize {
        match register {
            JoinRegister::Gpr(gpr) => self.gpr(gpr).map_or(0, |value| value.constants().len()),
            JoinRegister::Lr => self.lr_count(),
        }
    }
}

/// The constants of `value` that `own` does not hold.
fn missing<'v>(own: Option<&'v Value>, value: &'v Value) -> impl Iterator<Item = u64> + 'v {
    value
        .constants()
        .iter()
        .filter(move |constant| {
            own.is_none_or(|own| own.constants().binary_search(constant).is_err())
        })
        .copied()
}

/// The join word that a path joins into: its slot, and the bounds that
/// [`Bounds`] raises there.
#[derive(Clone, Copy)]
struct JoinAt<'b> {
    slot: usize,
    raised: Option<&'b BTreeMap<JoinRegister, usize>>,
}

impl JoinAt<'_> {
    /// The most constants that the join word keeps in `register`.
    fn limit(self, register: JoinRegister) -> usize {
        self.raised
            .and_then(|raised| raised.get(&register))
            .copied()
            .unwrap_or(JOIN_LIMIT)
    }
}

/// What a join did to the constants of one register.
enum Joined {
    /// The register already held each constant.
    Held,
    /// The register took the new constants.
    Grew,
    /// The register kept its constants, because the join would leave
    /// more constants than its bound.
    Overflowed,
}

/// Joins `value` into `own` unless the join would leave more than
/// `limit` constants.
fn join_limited(own: &mut Value, value: &Value, limit: usize) -> Joined {
    if own.holds_all(value) {
        return Joined::Held;
    }
    if joined_len(own, value, limit) > limit {
        return Joined::Overflowed;
    }
    *own = own.clone().union(value);
    Joined::Grew
}

/// The number of constants that the join of `own` and `value` holds, or
/// `limit + 1` when it holds more than `limit`.
///
/// The count does not build the join, so a join that overflows costs no
/// allocation. The count stops when the join overflows.
fn joined_len(own: &Value, value: &Value, limit: usize) -> usize {
    let over = limit + 1;
    let room = over.saturating_sub(own.constants().len());
    let added = value
        .constants()
        .iter()
        .filter(|constant| own.constants().binary_search(constant).is_err())
        .take(room)
        .count();
    (own.constants().len() + added).min(over)
}

/// The target of the word `raw` at `address` when it is a direct branch
/// that brings the GPRs and LR to its target unchanged.
///
/// The flow does not follow a call into its routine, so the routine gets
/// no constant of the caller. A `b` or a `bc` that links to the next word
/// writes LR on the taken path too. Its target is the next word, so its
/// fall-through brings the constants there.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
/// [PPC-Book1 p:24 s:2.4] A branch alters only CTR and LR, so r2 at the target is r2 at the branch.
fn taken(address: u64, raw: u32) -> Option<u64> {
    let (displacement, absolute) = match crate::decode::decode(raw) {
        Ok(PpuInstruction::B { offset, aa, .. }) => (i64::from(offset), aa),
        Ok(PpuInstruction::Bc { offset, aa, .. }) => (i64::from(offset), aa),
        _ => return None,
    };
    if calls(address, raw) || links_to_next_word(address, raw) {
        return None;
    }
    Some(branch_target(address, displacement, absolute))
}

/// What the paths to each join word bring in the GPRs and in LR.
///
/// A join word is a word at the real address of the target of a direct
/// branch that [`taken`] gives. A path goes from a word to these words:
/// - the next word, when the word can fall through;
/// - each join word of the target of the word, as [`taken`] gives it.
///
/// The flow joins the constants of each path to a join word, and
/// continues until no join word changes. Each register of a join word
/// only grows, to at most its bound of constants and a lost mark with at
/// most one origin for each drop, so the flow ends.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
pub(super) struct JoinFlow<'a> {
    words: &'a JoinWords<'a>,
    tocs: &'a WordTocs,
    every: &'a Value,
    bounds: &'a Bounds,
    /// What the paths to each join word bring, by the slot of the join
    /// word.
    at: Vec<Brought>,
    drops: Drops,
}

impl<'a> JoinFlow<'a> {
    /// The flow of the code of `words` until no join word changes.
    ///
    /// `tocs` holds the TOC of each word, and `every` holds each TOC that
    /// a descriptor gives. `bounds` holds the join words where a register
    /// keeps more than [`JOIN_LIMIT`] constants.
    ///
    /// With `counting`, the flow keeps the constants that each drop
    /// dropped, so [`Self::drops`] can count them. The flow is the same
    /// with or without `counting`, and so are the drop ids.
    pub(super) fn run(
        words: &'a JoinWords<'a>,
        tocs: &'a WordTocs,
        every: &'a Value,
        bounds: &'a Bounds,
        counting: bool,
    ) -> Self {
        let mut flow = Self {
            words,
            tocs,
            every,
            bounds,
            at: vec![Brought::default(); words.joins.len()],
            drops: Drops {
                counting,
                ..Drops::default()
            },
        };
        let mut work = Worklist::new(&words.steps);
        for (index, steps) in words.steps.iter().enumerate().rev() {
            for word in (0..steps.len()).rev() {
                if word == 0 || steps[word].join.is_some() || steps[word - 1].ends {
                    work.push((index, word));
                }
            }
        }
        while let Some(position) = work.pop() {
            flow.walk(position, &mut work);
        }
        flow
    }

    /// The constants on the word at `position` when it is a join word,
    /// before its TOC enters r2.
    pub(super) fn entered(&self, position: (usize, usize)) -> Option<Constants> {
        let (index, word) = position;
        self.words.steps[index][word]
            .join
            .map(|slot| self.at[slot].entered())
    }

    /// Steps from the word at `start` to the next join word or to the end
    /// of the path, and joins what the path brings into each join word on
    /// the way.
    fn walk(&mut self, start: (usize, usize), work: &mut Worklist) {
        let words = self.words;
        let (index, first) = start;
        let segment = &words.code.segments()[index];
        let steps = &words.steps[index];
        let mut constants = self.entered(start).unwrap_or_default();
        let raws = segment.words.get(first..).unwrap_or_default();
        let tocs = self.tocs.segment(index).get(first..).unwrap_or_default();
        for (word, (&raw, &toc)) in (first..).zip(raws.iter().zip(tocs)) {
            let step = &steps[word];
            if let Some(slot) = step.join.filter(|_| word != first) {
                self.join_into(slot, &constants, work);
                return;
            }
            constants.enter_toc(toc, self.every);
            for &slot in &step.targets {
                self.join_into(slot, &constants, work);
            }
            if step.ends {
                return;
            }
            constants.step(segment.vaddr + (word as u64) * 4, raw);
        }
    }

    /// Joins `constants` into the join word at `slot`, and queues the word
    /// when what it gets changes.
    fn join_into(&mut self, slot: usize, constants: &Constants, work: &mut Worklist) {
        let at = JoinAt {
            slot,
            raised: self.bounds.0.get(&slot),
        };
        if self.at[slot].join(constants, at, &mut self.drops) {
            work.push(self.words.joins[slot]);
        }
    }

    /// Each drop of the flow, by its id, with the slot of its join word.
    ///
    /// The count of a drop holds the constants that the paths brought to
    /// the register at the join word: those it kept and those it dropped.
    /// The count is `None` past [`RAISED_LIMIT`], and in a run without
    /// `counting`.
    pub(super) fn drops(&self) -> Vec<(usize, JoinDrop)> {
        self.drops
            .records
            .iter()
            .map(|record| {
                let brought = &self.at[record.slot];
                let count = brought.count(record.register)
                    + record
                        .dropped
                        .iter()
                        .filter(|&&item| !brought.holds(record.register, item))
                        .count();
                let (index, word) = self.words.joins[record.slot];
                let segment = &self.words.code.segments()[index];
                let drop = JoinDrop {
                    join: segment.vaddr + (word as u64) * 4,
                    register: record.register,
                    brought: Some(count)
                        .filter(|&count| self.drops.counting && count <= RAISED_LIMIT),
                };
                (record.slot, drop)
            })
            .collect()
    }
}

/// The drops of one [`JoinFlow`] run.
#[derive(Debug, Default)]
struct Drops {
    /// The id of the drop of each register at each join word, by the
    /// slot of the join word.
    ids: BTreeMap<(usize, JoinRegister), u32>,
    /// Each drop, by its id.
    records: Vec<DropRecord>,
    /// Whether each record keeps its dropped constants.
    counting: bool,
}

/// The constants that the joins at one join word dropped from one
/// register.
#[derive(Debug)]
struct DropRecord {
    slot: usize,
    register: JoinRegister,
    /// The dropped constants, each keyed as [`Brought::holds`] keys it.
    /// The record keeps at most `RAISED_LIMIT + 1` of them, which is
    /// enough to show that the count is past [`RAISED_LIMIT`].
    dropped: BTreeSet<(u64, u64)>,
}

impl Drops {
    /// Records that the join at `slot` dropped the constants `dropped`
    /// of `register`, and returns the origins of the mark that the drop
    /// sets.
    ///
    /// A drop past the last id that a `u32` holds gets no id: its mark
    /// names no drop and has more, so the register stays lost.
    fn record(
        &mut self,
        slot: usize,
        register: JoinRegister,
        dropped: impl Iterator<Item = (u64, u64)>,
    ) -> Origins {
        let id = match self.ids.get(&(slot, register)) {
            Some(&id) => id,
            None => {
                let Ok(id) = u32::try_from(self.records.len()) else {
                    return Origins::unnamed();
                };
                self.ids.insert((slot, register), id);
                self.records.push(DropRecord {
                    slot,
                    register,
                    dropped: BTreeSet::new(),
                });
                id
            }
        };
        if self.counting {
            let record = &mut self.records[id as usize];
            for item in dropped {
                if record.dropped.len() > RAISED_LIMIT {
                    break;
                }
                record.dropped.insert(item);
            }
        }
        Origins::of(id)
    }
}

/// The bound of each register at each join word where it is more than
/// [`JOIN_LIMIT`], by the slot of the join word.
#[derive(Debug, Default)]
pub(super) struct Bounds(BTreeMap<usize, BTreeMap<JoinRegister, usize>>);

impl Bounds {
    /// The bound of `register` at the join word at `slot`.
    #[cfg(test)]
    pub(super) fn bound(&self, slot: usize, register: JoinRegister) -> usize {
        let raised = self.0.get(&slot);
        JoinAt { slot, raised }.limit(register)
    }

    /// Raises the bound of each drop that `origins` names, when its count
    /// is at most [`RAISED_LIMIT`], and returns whether a bound rose.
    ///
    /// The new bound is the count or twice the old bound, whichever is
    /// more, and at most [`RAISED_LIMIT`]. A loop that steps a constant
    /// brings one more constant than each bound, so the doubling takes
    /// it past [`RAISED_LIMIT`] in two raises.
    ///
    /// `drops` holds each drop of the run that gave `origins`, by its id,
    /// as [`JoinFlow::drops`] gives it. A drop happens only when the count
    /// is past the bound, so each raise is strict.
    pub(super) fn raise<'o>(
        &mut self,
        origins: impl Iterator<Item = &'o Origins>,
        drops: &[(usize, JoinDrop)],
    ) -> bool {
        let mut raised = false;
        for &id in origins.flat_map(Origins::drops) {
            let Some(&(slot, drop)) = drops.get(id as usize) else {
                continue;
            };
            let Some(count) = drop.brought else {
                continue;
            };
            let bound = self.0.entry(slot).or_default().entry(drop.register);
            let bound = bound.or_insert(JOIN_LIMIT);
            let to = count.max(*bound * 2).min(RAISED_LIMIT);
            if to > *bound {
                *bound = to;
                raised = true;
            }
        }
        raised
    }
}

/// The join words of the code, and what each word does to the paths
/// through it.
///
/// These depend on the code alone, so the scan builds them once, and each
/// round of [`JoinFlow::run`] reads them.
pub(super) struct JoinWords<'a> {
    code: &'a Code,
    /// What each word does to the paths through it, in one vector for
    /// each code segment.
    steps: Vec<Vec<Step>>,
    /// The index of the segment and of the word there of each join word,
    /// by its slot.
    joins: Vec<(usize, usize)>,
}

/// What one word does to the paths through it.
#[derive(Debug)]
struct Step {
    /// The slot of the word when it is a join word: its index in
    /// [`JoinWords::joins`].
    join: Option<usize>,
    /// Whether the word ends each path through it, as [`ends_path`] gives
    /// it.
    ends: bool,
    /// The slot of each join word at a real alias of the target that
    /// [`taken`] gives for the word.
    targets: Box<[usize]>,
}

/// A direct branch that [`taken`] gives, and the join words at the real
/// aliases of its target.
struct Branch {
    /// The index of the segment and of the word there of the branch.
    from: (usize, usize),
    /// The index of the segment and of the word there of each join word,
    /// in the order of the real aliases.
    to: Vec<(usize, usize)>,
}

impl<'a> JoinWords<'a> {
    /// The join words of `code`, and what each word does to the paths
    /// through it.
    pub(super) fn new(code: &'a Code) -> Self {
        let branches: Vec<Branch> = code
            .segments()
            .iter()
            .enumerate()
            .flat_map(|(index, segment)| {
                segment
                    .addressed()
                    .enumerate()
                    .filter_map(move |(word, (address, raw))| {
                        let target = taken(address, raw)?;
                        let to = code
                            .real_aliases(target)
                            .flat_map(|alias| positions(code, alias))
                            .collect();
                        Some(Branch {
                            from: (index, word),
                            to,
                        })
                    })
            })
            .collect();
        let mut steps: Vec<Vec<Step>> = code
            .segments()
            .iter()
            .map(|segment| {
                segment
                    .words
                    .iter()
                    .map(|&raw| Step {
                        join: None,
                        ends: ends_path(raw),
                        targets: Box::default(),
                    })
                    .collect()
            })
            .collect();
        // Marks each join word. The loop after this one gives each its slot.
        for &(index, word) in branches.iter().flat_map(|branch| &branch.to) {
            steps[index][word].join = Some(0);
        }
        let mut joins = Vec::new();
        for (index, words) in steps.iter_mut().enumerate() {
            for (word, step) in words.iter_mut().enumerate() {
                if step.join.is_some() {
                    step.join = Some(joins.len());
                    joins.push((index, word));
                }
            }
        }
        for branch in branches {
            // Each target is a join word, so each has a slot.
            let targets = branch
                .to
                .iter()
                .filter_map(|&(index, word)| steps[index][word].join)
                .collect();
            let (index, word) = branch.from;
            steps[index][word].targets = targets;
        }
        Self { code, steps, joins }
    }
}

/// The words that [`JoinFlow::run`] has yet to step from, by the index of
/// the segment and of the word there.
struct Worklist {
    /// Whether each word is in [`Self::pending`], in one vector for each
    /// code segment.
    queued: Vec<Vec<bool>>,
    pending: Vec<(usize, usize)>,
}

impl Worklist {
    /// The empty list for the words of `steps`.
    fn new(steps: &[Vec<Step>]) -> Self {
        Self {
            queued: steps.iter().map(|steps| vec![false; steps.len()]).collect(),
            pending: Vec::new(),
        }
    }

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
