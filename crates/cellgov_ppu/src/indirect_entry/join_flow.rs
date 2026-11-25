//! What the paths to each join word bring in the GPRs and in LR.

use std::collections::BTreeMap;

use super::constants::{ends_path, Constants, Value};
use super::ctr_flow::{calls, links_to_next_word, positions};
use super::toc::WordTocs;
use crate::hypercall_cfg::{branch_target, Code};
use crate::instruction::PpuInstruction;

/// Most constants that the flow keeps for one GPR, or for LR, at a join
/// word.
///
/// When the paths to a join word bring more constants in a register, the
/// join keeps the constants it holds and marks the register as one that
/// can also hold a constant the flow dropped. So a loop that steps a
/// constant adds a finite number of constants, and the flow ends.
///
/// A dropped constant can be the address that a later `bcctr` or `bclr`
/// reaches. So a CTR copied from a marked GPR, and a marked LR at a
/// `bclr`, can reach an address that the scan does not know.
const JOIN_LIMIT: usize = 16;

/// What the paths to one join word bring in the GPRs and in LR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Brought {
    /// The constants of each GPR that holds one, by GPR number in
    /// ascending order.
    gprs: Vec<(u8, Value)>,
    /// Each GPR that can also hold a constant that the flow dropped, as
    /// bit `n` for GPR `n`.
    lost_gprs: u32,
    /// The constants in LR, by the address of the word that wrote each
    /// one.
    lr: BTreeMap<u64, Value>,
    /// Whether LR can also hold a constant that the flow dropped.
    lr_lost: bool,
}

impl Brought {
    /// The constants on the join word, before its TOC enters r2.
    fn entered(&self) -> Constants {
        let mut constants = Constants::default();
        for (gpr, value) in &self.gprs {
            constants.gprs[usize::from(*gpr)] = Some(value.clone());
        }
        constants.lost_gprs = self.lost_gprs;
        constants.lr.clone_from(&self.lr);
        constants.lr_lost = self.lr_lost;
        constants
    }

    /// Joins the constants that one path brings into `self`, and returns
    /// whether `self` changed.
    ///
    /// A join that would leave more than [`JOIN_LIMIT`] constants in a
    /// register keeps the constants of the register and marks it as lost.
    /// A register that the path marks as lost is marked here too.
    fn join(&mut self, path: &Constants) -> bool {
        let mut changed = false;
        let mut lost_gprs = self.lost_gprs | path.lost_gprs;
        for gpr in 0..32u8 {
            let Some(value) = path.gpr(gpr) else {
                continue;
            };
            let joined = match self.gprs.binary_search_by_key(&gpr, |&(own, _)| own) {
                Ok(at) => join_limited(&mut self.gprs[at].1, value),
                Err(at) if value.constants().len() <= JOIN_LIMIT => {
                    self.gprs.insert(at, (gpr, value.clone()));
                    Joined::Grew
                }
                Err(_) => Joined::Overflowed,
            };
            match joined {
                Joined::Held => {}
                Joined::Grew => changed = true,
                Joined::Overflowed => lost_gprs |= 1 << gpr,
            }
        }
        let mut lr_lost = self.lr_lost || path.lr_lost;
        for (&by, value) in &path.lr {
            let own = self.lr.get(&by);
            let joined_len = match own {
                Some(own) if own.holds_all(value) => continue,
                Some(own) => joined_len(own, value),
                None => value.constants().len(),
            };
            let others = self.lr_count() - own.map_or(0, |own| own.constants().len());
            if others + joined_len <= JOIN_LIMIT {
                let joined = own.map_or_else(|| value.clone(), |own| own.clone().union(value));
                self.lr.insert(by, joined);
                changed = true;
            } else {
                lr_lost = true;
            }
        }
        if (lost_gprs, lr_lost) != (self.lost_gprs, self.lr_lost) {
            self.lost_gprs = lost_gprs;
            self.lr_lost = lr_lost;
            changed = true;
        }
        changed
    }

    /// The number of constants in LR, over each word that wrote one.
    fn lr_count(&self) -> usize {
        self.lr.values().map(|value| value.constants().len()).sum()
    }
}

/// What a join did to the constants of one register.
enum Joined {
    /// The register already held each constant.
    Held,
    /// The register took the new constants.
    Grew,
    /// The register kept its constants, because the join would leave
    /// more than [`JOIN_LIMIT`].
    Overflowed,
}

/// Joins `value` into `own` unless the join would leave more than
/// [`JOIN_LIMIT`] constants.
fn join_limited(own: &mut Value, value: &Value) -> Joined {
    if own.holds_all(value) {
        return Joined::Held;
    }
    if joined_len(own, value) > JOIN_LIMIT {
        return Joined::Overflowed;
    }
    *own = own.clone().union(value);
    Joined::Grew
}

/// The number of constants that the join of `own` and `value` holds, or
/// `JOIN_LIMIT + 1` when it holds more than [`JOIN_LIMIT`].
///
/// The count does not build the join, so a join that overflows costs no
/// allocation, and it stops when the join overflows.
fn joined_len(own: &Value, value: &Value) -> usize {
    let over = JOIN_LIMIT + 1;
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
/// no constant of the caller. A `b` or a
/// `bc` that links to the next word writes LR on the taken path too, so
/// its fall-through brings its constants there.
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
/// only grows, to at most [`JOIN_LIMIT`] constants and one lost mark, so
/// the flow ends.
///
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
pub(super) struct JoinFlow<'a> {
    words: &'a JoinWords<'a>,
    tocs: &'a WordTocs,
    every: &'a Value,
    /// What the paths to each join word bring, by the slot of the join
    /// word.
    at: Vec<Brought>,
}

impl<'a> JoinFlow<'a> {
    /// The flow of the code of `words` until no join word changes.
    ///
    /// `tocs` holds the TOC of each word, and `every` holds each TOC that
    /// a descriptor gives.
    pub(super) fn run(words: &'a JoinWords<'a>, tocs: &'a WordTocs, every: &'a Value) -> Self {
        let mut flow = Self {
            words,
            tocs,
            every,
            at: vec![Brought::default(); words.joins.len()],
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
        if self.at[slot].join(constants) {
            work.push(self.words.joins[slot]);
        }
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
