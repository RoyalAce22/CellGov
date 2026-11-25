//! The TOCs that r2 can hold on each word, from the function descriptors
//! and the functions that reach the word.

use std::collections::{BTreeMap, BTreeSet};

use super::bound::FileSegment;
use super::constants::{never_falls_through, Value};
use crate::hypercall_cfg::{branch_target, Code};
use crate::instruction::PpuInstruction;
use crate::local_cfg::falls_through;

/// The TOCs that the function descriptors in `files` give each word of
/// `code` that they name as a function's first word.
///
/// A descriptor counts only when its environment is zero, as each LV2
/// descriptor's is. Data that only looks like a descriptor also counts,
/// because the scan cannot tell it from a descriptor. An extra TOC can
/// only add words to the bound.
///
/// [OFW-4.93 lv2_kernel a:0x80000000002765e4-0x8000000000276600] A call through a descriptor loads LR from its first doubleword and r2 from its second.
pub(super) fn function_tocs(
    files: &[FileSegment<'_>],
    code: &Code,
) -> BTreeMap<u64, BTreeSet<u64>> {
    let mut tocs: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    for [entry, toc, environment] in files.iter().flat_map(FileSegment::descriptors) {
        if environment == 0 && code.holds(entry) {
            tocs.entry(entry).or_default().insert(toc);
        }
    }
    tocs
}

/// The TOCs that r2 can hold on one word, from the functions that reach
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FunctionToc {
    /// No function that a descriptor names reaches the word.
    Outside,
    /// Each function that reaches the word has this TOC.
    Toc(u64),
    /// Functions with two different TOCs reach the word, so r2 can hold
    /// each TOC that a descriptor gives.
    Every,
}

impl FunctionToc {
    /// The TOC of a word that the functions of `self` and of `other`
    /// reach.
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Outside, toc) | (toc, Self::Outside) => toc,
            (Self::Toc(one), Self::Toc(other)) if one == other => self,
            _ => Self::Every,
        }
    }

    /// The constants that r2 can hold on a word with this TOC.
    ///
    /// `path` holds the constants of r2 that the scan keeps on the paths
    /// to the word: on one straight-line path, or on the paths that join
    /// at a branch target. `None` says that the scan keeps no constant of
    /// r2 there. On a word that a function reaches, r2 can also hold the
    /// function's TOC. So the result adds to `path` the TOC of
    /// [`Self::Toc`], or each TOC of `every` for [`Self::Every`].
    /// [`Self::Outside`] gives `path`. `every` holds each TOC that a
    /// descriptor gives.
    pub(super) fn in_r2(self, path: Option<Value>, every: &Value) -> Option<Value> {
        let tocs = match self {
            Self::Outside => return path,
            Self::Toc(toc) => Value::One(toc),
            Self::Every => every.clone(),
        };
        Some(match path {
            Some(path) => path.union(&tocs),
            None => tocs,
        })
    }
}

/// The [`FunctionToc`] of each word of the code, in one vector for each
/// code segment.
#[derive(Debug)]
pub(super) struct WordTocs {
    segments: Vec<(u64, Vec<FunctionToc>)>,
}

impl WordTocs {
    fn new(code: &Code) -> Self {
        Self {
            segments: code
                .segments()
                .iter()
                .map(|segment| {
                    (
                        segment.vaddr,
                        vec![FunctionToc::Outside; segment.words.len()],
                    )
                })
                .collect(),
        }
    }

    /// The TOC of each word of the code segment at `index`.
    pub(super) fn segment(&self, index: usize) -> &[FunctionToc] {
        &self.segments[index].1
    }

    /// The TOC that each segment keeps for its word at `address`.
    fn at(&mut self, address: u64) -> impl Iterator<Item = &mut FunctionToc> + '_ {
        self.segments.iter_mut().filter_map(move |(vaddr, tocs)| {
            let offset = address.checked_sub(*vaddr)?;
            if offset % 4 != 0 {
                return None;
            }
            tocs.get_mut(usize::try_from(offset / 4).ok()?)
        })
    }

    /// The TOC of the word at `address`.
    fn get(&mut self, address: u64) -> FunctionToc {
        self.at(address)
            .fold(FunctionToc::Outside, |toc, known| toc.join(*known))
    }

    /// Joins `toc` into the word at `address`, and returns whether the
    /// word's TOC changed.
    fn join(&mut self, address: u64, toc: FunctionToc) -> bool {
        let mut changed = false;
        for known in self.at(address) {
            let joined = known.join(toc);
            changed |= joined != *known;
            *known = joined;
        }
        changed
    }
}

/// The TOC of each word of the code that is in a function that a
/// descriptor names.
///
/// The words of a function are the words that control can reach from
/// its first word with the steps of [`FunctionSteps`]. The steps also
/// enter the first word of another function. So a word in the middle of
/// a function keeps that function's TOC when a descriptor names it.
///
/// The walk errs wide: a word that gets a TOC in error can only add
/// words to the bound. An indirect branch reaches each named word, so
/// the walk continues as the named words grow.
pub(super) struct FunctionWords<'a> {
    code: &'a Code,
    pub(super) tocs: WordTocs,
    /// The words whose TOC changed and whose steps the walk has not
    /// taken since.
    pending: Vec<u64>,
    /// The join of the TOCs of the indirect branches that the walk
    /// reached.
    indirect: FunctionToc,
    /// The named words that already have [`Self::indirect`].
    carried: BTreeSet<u64>,
}

impl<'a> FunctionWords<'a> {
    /// The walk before its first step, with the TOCs that `functions`
    /// gives each function's first word.
    pub(super) fn new(code: &'a Code, functions: &BTreeMap<u64, BTreeSet<u64>>) -> Self {
        let mut tocs = WordTocs::new(code);
        for (&first, first_tocs) in functions {
            for &toc in first_tocs {
                tocs.join(first, FunctionToc::Toc(toc));
            }
        }
        Self {
            code,
            tocs,
            pending: functions.keys().copied().collect(),
            indirect: FunctionToc::Outside,
            carried: BTreeSet::new(),
        }
    }

    /// Walks the steps from each word whose TOC changed, with an
    /// indirect branch that reaches each word of `named`.
    pub(super) fn grow(&mut self, named: &BTreeSet<u64>) {
        self.carry(named);
        while let Some(at) = self.pending.pop() {
            let toc = self.tocs.get(at);
            let code = self.code;
            for raw in code.words_at(at) {
                let steps = FunctionSteps::of(at, raw);
                if let Some(next) = at.checked_add(4).filter(|_| steps.next) {
                    self.reach(next, toc);
                }
                if let Some(target) = steps.target {
                    self.reach(target, toc);
                }
                if steps.indirect && self.indirect.join(toc) != self.indirect {
                    self.indirect = self.indirect.join(toc);
                    self.carried.clear();
                    self.carry(named);
                }
            }
        }
    }

    /// Gives [`Self::indirect`] to each word of `named` that does not
    /// have it yet.
    fn carry(&mut self, named: &BTreeSet<u64>) {
        if self.indirect == FunctionToc::Outside {
            return;
        }
        for &word in named {
            if self.carried.insert(word) {
                self.reach(word, self.indirect);
            }
        }
    }

    /// Joins `toc` into the word at `address`.
    fn reach(&mut self, address: u64, toc: FunctionToc) {
        if self.tocs.join(address, toc) {
            self.pending.push(address);
        }
    }
}

/// The steps that control can take from one word with r2 unchanged.
///
/// A direct branch that links steps to its target and to the next word,
/// where the call returns. The target gets the TOC of its caller, as
/// well as its own TOC when it is the first word of a function.
///
/// [PPC-Book1 p:24 s:2.4] The registers that a branch can alter are CTR and LR, so r2 at the target of a call is r2 at the call.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FunctionSteps {
    /// Control can go to the next word.
    next: bool,
    /// The target of a direct branch.
    pub(super) target: Option<u64>,
    /// The word is an indirect branch that does not link.
    indirect: bool,
}

impl FunctionSteps {
    /// The steps from the word `raw` at `address`.
    ///
    /// A word that does not decode can cause an exception that a handler
    /// emulates, so control can go to the next word.
    pub(super) fn of(address: u64, raw: u32) -> Self {
        let Ok(decoded) = crate::decode::decode(raw) else {
            return Self::to_next(true);
        };
        let direct = |displacement: i64, aa: bool| Some(branch_target(address, displacement, aa));
        match decoded {
            PpuInstruction::B { offset, aa, link } => Self {
                next: link,
                target: direct(i64::from(offset), aa),
                indirect: false,
            },
            PpuInstruction::Bc {
                bo,
                offset,
                aa,
                link,
                ..
            } => Self {
                next: falls_through(bo, link),
                target: direct(i64::from(offset), aa),
                indirect: false,
            },
            PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. } => {
                Self {
                    next: falls_through(bo, link),
                    target: None,
                    indirect: !link,
                }
            }
            _ => Self::to_next(!never_falls_through(&decoded)),
        }
    }

    /// The steps of a word that is not a branch.
    fn to_next(next: bool) -> Self {
        Self {
            next,
            target: None,
            indirect: false,
        }
    }
}
