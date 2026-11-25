//! Resolves the number of each LV1 hypercall site over the control-flow
//! graph of all the executable segments of one ELF.
//!
//! The walk goes backward from `sc` along every path that can reach it.
//! A path steps back over a fall-through or over a direct branch, and a
//! direct branch may come from another segment. Each path must end at an
//! `li r11` that loads the same constant. Some paths end where the walk
//! cannot see r11:
//! - a word that may write r11;
//! - a call, which may change r11 before it returns;
//! - a word that control can enter in another way, which
//!   [`crate::indirect_entry`] bounds.
//!
//! Each of these leaves the site unresolved. A path also ends at a word
//! that nothing enters, such as a `nop` after `blr`. That path loads no
//! constant.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

use crate::caller_census::{maybe_clobbers_r11, CallerSite};
use crate::instruction::PpuInstruction;
use crate::local_cfg::is_unconditional;

/// Most words the walk of one site visits before it leaves the site
/// unresolved.
const HYPERCALL_WALK_LIMIT: usize = 1024;

/// The words of one executable segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeSegment {
    /// The address of the first word.
    pub(crate) vaddr: u64,
    /// The big-endian words of the segment's file bytes.
    pub(crate) words: Vec<u32>,
}

impl CodeSegment {
    /// Each word, with its address.
    pub(crate) fn addressed(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        self.words
            .iter()
            .enumerate()
            .map(|(index, &raw)| (self.vaddr + (index as u64) * 4, raw))
    }

    fn word_at(&self, address: u64) -> Option<u32> {
        let offset = address.checked_sub(self.vaddr)?;
        if offset % 4 != 0 {
            return None;
        }
        self.words.get(usize::try_from(offset / 4).ok()?).copied()
    }
}

/// The executable segments of one ELF, in program-header order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Code {
    segments: Vec<CodeSegment>,
}

impl Code {
    pub(crate) fn new(segments: Vec<CodeSegment>) -> Self {
        Self { segments }
    }

    pub(crate) fn segments(&self) -> &[CodeSegment] {
        &self.segments
    }

    /// The word at `address` in each segment that holds one there.
    pub(crate) fn words_at(&self, address: u64) -> impl Iterator<Item = u32> + '_ {
        self.segments
            .iter()
            .filter_map(move |segment| segment.word_at(address))
    }

    /// Whether a segment holds a word at `address`.
    pub(crate) fn holds(&self, address: u64) -> bool {
        self.words_at(address).next().is_some()
    }
}

/// What one word does to r11 on the path that falls through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// No path falls through the word.
    Stops,
    /// The word leaves r11 as it was.
    Keeps,
    /// The word loads this constant into r11.
    Loads(u16),
    /// The word may change r11 in a way the walk cannot follow.
    Unknown,
}

/// The sites in `code`: each word that is exactly `sc 1`, with the
/// number every path to it loads into r11.
///
/// `entries` holds the words that control may enter other than by a
/// direct branch or a fall-through.
pub(crate) fn scan(code: &Code, entries: &BTreeSet<u64>) -> Vec<CallerSite> {
    let branches = branch_sources(code);
    code.segments
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(_, raw)| raw == PPC_SC_1)
        .map(|(address, _)| CallerSite {
            address,
            ordinal: resolve(code, &branches, entries, address),
        })
        .collect()
}

/// Each address that a direct branch of `code` reaches, with the
/// addresses of the branches that reach it. A target that no segment
/// holds is dropped.
fn branch_sources(code: &Code) -> BTreeMap<u64, Vec<u64>> {
    let mut sources: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for (address, raw) in code.segments.iter().flat_map(CodeSegment::addressed) {
        let (displacement, absolute) = match crate::decode::decode(raw) {
            Ok(PpuInstruction::B { offset, aa, .. }) => (i64::from(offset), aa),
            Ok(PpuInstruction::Bc { offset, aa, .. }) => (i64::from(offset), aa),
            _ => continue,
        };
        let target = branch_target(address, displacement, absolute);
        if code.holds(target) {
            sources.entry(target).or_default().push(address);
        }
    }
    sources
}

/// The address that a direct branch at `address` reaches in 64-bit mode.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + EXTS(displacement), or EXTS(displacement) when AA = 1.
pub(crate) fn branch_target(address: u64, displacement: i64, absolute: bool) -> u64 {
    let base = if absolute { 0 } else { address };
    base.wrapping_add_signed(displacement)
}

/// The constant that every path to the word at `site` loads into r11,
/// or `None` when the walk cannot prove one or no path loads one.
fn resolve(
    code: &Code,
    branches: &BTreeMap<u64, Vec<u64>>,
    entries: &BTreeSet<u64>,
    site: u64,
) -> Option<u16> {
    let mut number = None;
    let mut seen = BTreeSet::from([site]);
    let mut pending = vec![site];
    let mut visited = 0;
    while let Some(at) = pending.pop() {
        if visited == HYPERCALL_WALK_LIMIT || entries.contains(&at) {
            return None;
        }
        visited += 1;
        let mut next = branches.get(&at).cloned().unwrap_or_default();
        match flow_into(code, at) {
            Flow::Stops => {}
            Flow::Keeps => next.push(at - 4),
            Flow::Loads(loaded) if number.is_some_and(|earlier| earlier != loaded) => return None,
            Flow::Loads(loaded) => number = Some(loaded),
            Flow::Unknown => return None,
        }
        for address in next {
            if seen.insert(address) {
                pending.push(address);
            }
        }
    }
    number
}

/// What the word before `at` does to r11 on the path that falls through
/// it into `at`.
///
/// When two segments hold different words there, the walk cannot know
/// which one runs, so the flow is unknown.
fn flow_into(code: &Code, at: u64) -> Flow {
    let Some(before) = at.checked_sub(4) else {
        return Flow::Stops;
    };
    let mut flows = code.words_at(before).map(flow);
    let Some(first) = flows.next() else {
        return Flow::Stops;
    };
    if flows.all(|other| other == first) {
        first
    } else {
        Flow::Unknown
    }
}

/// What the word `raw` does to r11 on the path that falls through it.
///
/// A taken branch writes no GPR, so this covers only the fall-through.
fn flow(raw: u32) -> Flow {
    let Ok(decoded) = crate::decode::decode(raw) else {
        return Flow::Unknown;
    };
    match decoded {
        PpuInstruction::B { link, .. } => after_branch(true, link),
        PpuInstruction::Bc { bo, link, .. }
        | PpuInstruction::Bclr { bo, link, .. }
        | PpuInstruction::Bcctr { bo, link, .. } => after_branch(is_unconditional(bo), link),
        PpuInstruction::Sc { .. } => Flow::Unknown,
        PpuInstruction::Addi { rt: 11, ra: 0, imm } => {
            u16::try_from(imm).map_or(Flow::Unknown, Flow::Loads)
        }
        PpuInstruction::Addi { rt: 11, .. }
        | PpuInstruction::Addis { rt: 11, .. }
        | PpuInstruction::Or { ra: 11, .. }
        | PpuInstruction::Lmw { .. }
        | PpuInstruction::Lswi { .. }
        | PpuInstruction::Lswx { .. }
        | PpuInstruction::PpeIllegal { .. }
        | PpuInstruction::PpePrivileged { .. } => Flow::Unknown,
        _ if maybe_clobbers_r11(raw) => Flow::Unknown,
        _ => Flow::Keeps,
    }
}

/// The fall-through of a branch that branches always when
/// `unconditional` is set, and that writes LR when `link` is set.
///
/// A branch that links falls through only when its callee returns, and
/// the callee may change r11.
fn after_branch(unconditional: bool, link: bool) -> Flow {
    match (unconditional, link) {
        (_, true) => Flow::Unknown,
        (true, false) => Flow::Stops,
        (false, false) => Flow::Keeps,
    }
}

#[cfg(test)]
#[path = "tests/hypercall_cfg_tests.rs"]
mod tests;
