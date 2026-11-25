//! Resolves the number of each LV1 hypercall site over the control-flow
//! graph of its segment.
//!
//! The walk goes backward from `sc` along every path that can reach it.
//! Each path must end at an `li r11` that loads the same constant. Some
//! paths end where the walk cannot see r11:
//! - a word that may write r11;
//! - a call, which may change r11 before it returns.
//!
//! Each of these leaves the site unresolved. A path also ends at a word
//! that no direct branch or fall-through reaches, such as a `nop` after
//! `blr`. That path loads no constant. The walk does not see an indirect
//! branch or a branch from another segment.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

use crate::caller_census::{maybe_clobbers_r11, CallerSite};
use crate::instruction::PpuInstruction;
use crate::local_cfg::{is_unconditional, local_branch_index};

/// Most words the walk of one site visits before it leaves the site
/// unresolved.
const HYPERCALL_WALK_LIMIT: usize = 1024;

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

/// The sites in `words`, which start at `base`: each word that is
/// exactly `sc 1`, with the number every path to it loads into r11.
pub(crate) fn scan(base: u64, words: &[u32]) -> Vec<CallerSite> {
    let branches = branch_sources(base, words);
    words
        .iter()
        .enumerate()
        .filter(|&(_, &raw)| raw == PPC_SC_1)
        .map(|(index, _)| CallerSite {
            address: base + (index as u64) * 4,
            ordinal: resolve(words, &branches, index),
        })
        .collect()
}

/// Each word index that a direct branch of `words` reaches, with the
/// indices of the branches that reach it. A target outside `words` is
/// dropped.
///
/// [PPC-Book1 p:24 s:2.4] `b` and `bc` reach CIA + the displacement, or the displacement itself when AA = 1.
fn branch_sources(base: u64, words: &[u32]) -> BTreeMap<usize, Vec<usize>> {
    let mut sources: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, raw) in words.iter().copied().enumerate() {
        let (displacement, absolute) = match crate::decode::decode(raw) {
            Ok(PpuInstruction::B { offset, aa, .. }) => (i64::from(offset), aa),
            Ok(PpuInstruction::Bc { offset, aa, .. }) => (i64::from(offset), aa),
            _ => continue,
        };
        if let Some(target) = local_branch_index(base, index, displacement, absolute, words.len()) {
            sources.entry(target).or_default().push(index);
        }
    }
    sources
}

/// The constant that every path to the word at `site` loads into r11,
/// or `None` when the walk cannot prove one or no path loads one.
fn resolve(words: &[u32], branches: &BTreeMap<usize, Vec<usize>>, site: usize) -> Option<u16> {
    let mut number = None;
    let mut seen = BTreeSet::from([site]);
    let mut pending = vec![site];
    let mut visited = 0;
    while let Some(at) = pending.pop() {
        if visited == HYPERCALL_WALK_LIMIT {
            return None;
        }
        visited += 1;
        let fall = at
            .checked_sub(1)
            .map_or(Flow::Stops, |before| flow(words[before]));
        let mut next = branches.get(&at).cloned().unwrap_or_default();
        match fall {
            Flow::Stops => {}
            Flow::Keeps => next.push(at - 1),
            Flow::Loads(loaded) => {
                if number
                    .replace(loaded)
                    .is_some_and(|earlier| earlier != loaded)
                {
                    return None;
                }
            }
            Flow::Unknown => return None,
        }
        for index in next {
            if seen.insert(index) {
                pending.push(index);
            }
        }
    }
    number
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
