//! Resolves the number of each LV1 hypercall site over the control-flow
//! graph of all the executable segments of one ELF.
//!
//! The walk goes backward from `sc` along every path that can reach it.
//! A path steps back over a fall-through or over a direct branch, and a
//! direct branch may come from another segment. An absolute branch that
//! may run in real mode also reaches the word at the real address of its
//! target. Each path must end at an `li r11` that loads the same
//! constant. Some paths end where the walk cannot see r11:
//! - a word that may write r11;
//! - a call, which may change r11 before it returns;
//! - a word that control can enter in another way, which
//!   [`crate::indirect_entry`] bounds.
//!
//! Each of these leaves the site unresolved. A `bcctr` whose target the
//! bound lost leaves every site unresolved. A path also ends at a word
//! that nothing enters, such as a `nop` after `blr`. That path loads no
//! constant.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;
use cellgov_ps3_abi::hw::ppu_mmu::REAL_MODE_IGNORED_EA;

use crate::caller_census::{maybe_clobbers_r11, CallerSite};
use crate::indirect_entry::IndirectBound;
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

    /// The address of each word that a segment holds at the real address
    /// of `address`.
    ///
    /// The real address of a word is its address without EA bits 0 and 1.
    /// The kernel run loads each kernel segment at that real address.
    ///
    /// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
    /// [PPC-Book3 p:30 s:4.2.6.2] The offset real mode mechanism compares EA bits 2:63 with the RMLR limit and ORs them with the RMOR offset.
    pub(crate) fn real_aliases(&self, address: u64) -> impl Iterator<Item = u64> + '_ {
        let real = address & !REAL_MODE_IGNORED_EA;
        (0..4u64)
            .map(move |bits| real | (bits << REAL_MODE_IGNORED_EA.trailing_zeros()))
            .filter(|&alias| self.holds(alias))
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
/// `bound` holds the words that control may enter other than by a
/// direct branch or a fall-through. `real_mode_starts` holds the words
/// where control enters in real mode at the ELF entry or at an
/// interrupt. The scan adds the words that the real-mode switches of
/// `bound` enter.
///
/// An unbounded switch of `bound` can put control into real mode at any
/// word, so then every word may run in real mode. A `bcctr` or `bclr`
/// that may run in real mode can reach each word that
/// [`real_indirect_words`] gives. A `bcctr` whose target `bound` lost
/// can enter any word in either mode, so it leaves every site
/// unresolved.
pub(crate) fn scan(
    code: &Code,
    bound: &IndirectBound,
    real_mode_starts: &BTreeSet<u64>,
) -> Vec<CallerSite> {
    let real_mode = if bound.unbounded_switches.is_empty() {
        let mut starts = real_mode_starts.clone();
        starts.extend(switched_words(code, bound));
        real_mode_words(code, &starts, bound)
    } else {
        RealMode::Every
    };
    let lost = !bound.lost_targets.is_empty();
    let branches = branch_sources(code, &real_mode);
    let entries = entered(code, bound, &real_mode);
    code.segments
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(_, raw)| raw == PPC_SC_1)
        .map(|(address, _)| CallerSite {
            address,
            ordinal: if lost {
                None
            } else {
                resolve(code, &branches, &entries, address)
            },
        })
        .collect()
}

/// The words that control may enter other than by a direct branch or a
/// fall-through:
/// - the entries of `bound`;
/// - each target of `bound` that a segment holds;
/// - each word at the real address of a target of `bound` whose writer
///   is in `real_mode`;
/// - each word at the real address of a real-mode switch of `bound`;
/// - when a `bcctr` or `bclr` may run in real mode, each word that
///   [`real_indirect_words`] gives.
///
/// An indirect branch in real mode and a real-mode switch reach the word
/// at the real address of their target. The entries of `bound` miss that
/// word when no segment holds the target address itself. They also miss
/// some targets, such as the word after a call, where a `bclr` returns.
///
/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book3 p:29 s:4.2.6] Real addressing mode ignores EA bit 1, and with MSR\[HV\] = 0 and LPES1 = 1 the offset real mode mechanism controls the access.
fn entered(code: &Code, bound: &IndirectBound, real_mode: &RealMode) -> BTreeSet<u64> {
    let mut entries = bound.entries.clone();
    entries.extend(switched_words(code, bound));
    if real_mode.branches_indirectly() {
        entries.extend(real_indirect_words(code, &bound.entries, &bound.targets));
    }
    for (by, targets) in &bound.targets {
        let real = real_mode.contains(*by);
        for &target in targets {
            if code.holds(target) {
                entries.insert(target);
            }
            if real {
                entries.extend(code.real_aliases(target));
            }
        }
    }
    entries
}

/// The words of the code that may run in real mode.
#[derive(Debug)]
enum RealMode {
    Every,
    /// The words that [`real_mode_words`] gives.
    Words {
        words: BTreeSet<u64>,
        /// Whether one of `words` is a `bcctr` or a `bclr`.
        indirect: bool,
    },
}

impl RealMode {
    /// Whether the word at `address` may run in real mode.
    fn contains(&self, address: u64) -> bool {
        match self {
            Self::Every => true,
            Self::Words { words, .. } => words.contains(&address),
        }
    }

    /// Whether a `bcctr` or a `bclr` may run in real mode.
    ///
    /// Under [`Self::Every`], an unbounded switch can also reach each
    /// address that the bound holds.
    fn branches_indirectly(&self) -> bool {
        match self {
            Self::Every => true,
            Self::Words { indirect, .. } => *indirect,
        }
    }
}

/// The words that a `bcctr` or `bclr` may reach in real mode.
///
/// The set holds the word at the real address of each of these:
/// - a word of `entries`, for a branch that no CTR or LR write of
///   `targets` reaches;
/// - a target of `targets`, also one whose writer did not run in real
///   mode;
/// - the word after a branch that links, with the LR that the branch
///   wrote.
///
/// `targets` keys each target by the word that wrote CTR or LR. The scan
/// cannot tell which branch reaches a target, so it keeps every target.
pub(crate) fn real_indirect_words<'a>(
    code: &'a Code,
    entries: &'a BTreeSet<u64>,
    targets: &'a BTreeMap<u64, BTreeSet<u64>>,
) -> impl Iterator<Item = u64> + 'a {
    entries
        .iter()
        .chain(targets.values().flatten())
        .copied()
        .chain(return_points(code))
        .flat_map(|address| code.real_aliases(address))
}

/// The address of the word after each branch of `code` that links.
///
/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` and `bcctr` with LK = 1 also set LR to CIA + 4.
fn return_points(code: &Code) -> impl Iterator<Item = u64> + '_ {
    code.segments
        .iter()
        .flat_map(CodeSegment::addressed)
        .filter(|&(_, raw)| links(raw))
        .filter_map(|(address, _)| address.checked_add(4))
}

/// Whether the word `raw` is a branch that writes LR.
fn links(raw: u32) -> bool {
    matches!(
        crate::decode::decode(raw),
        Ok(PpuInstruction::B { link: true, .. }
            | PpuInstruction::Bc { link: true, .. }
            | PpuInstruction::Bclr { link: true, .. }
            | PpuInstruction::Bcctr { link: true, .. })
    )
}

/// Whether the word `raw` is a `bcctr` or a `bclr`.
fn is_indirect_branch(raw: u32) -> bool {
    matches!(
        crate::decode::decode(raw),
        Ok(PpuInstruction::Bcctr { .. } | PpuInstruction::Bclr { .. })
    )
}

/// Each word of `code` at the real address of a real-mode switch of
/// `bound`, where control runs in real mode.
fn switched_words<'a>(code: &'a Code, bound: &'a IndirectBound) -> impl Iterator<Item = u64> + 'a {
    bound
        .real_mode_switches
        .iter()
        .flat_map(|&address| code.real_aliases(address))
}

/// Each address that a direct branch of `code` reaches, with the
/// addresses of the branches that reach it. The map omits a target that
/// no segment holds.
///
/// `real_mode` holds the words that may run in real mode.
fn branch_sources(code: &Code, real_mode: &RealMode) -> BTreeMap<u64, Vec<u64>> {
    let mut sources: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for (address, raw) in code.segments.iter().flat_map(CodeSegment::addressed) {
        for target in direct_targets(code, address, raw, real_mode.contains(address)) {
            sources.entry(target).or_default().push(address);
        }
    }
    sources
}

/// The words of `code` that the direct branch `raw` at `address` can
/// reach, or no words when `raw` is not a direct branch.
///
/// When `real_mode` is set, the branch may run in real mode. An absolute
/// branch there also reaches each word whose real address is the real
/// address of its target.
fn direct_targets(code: &Code, address: u64, raw: u32, real_mode: bool) -> BTreeSet<u64> {
    let (displacement, absolute) = match crate::decode::decode(raw) {
        Ok(PpuInstruction::B { offset, aa, .. }) => (i64::from(offset), aa),
        Ok(PpuInstruction::Bc { offset, aa, .. }) => (i64::from(offset), aa),
        _ => return BTreeSet::new(),
    };
    let target = branch_target(address, displacement, absolute);
    let mut targets: BTreeSet<u64> = code.holds(target).then_some(target).into_iter().collect();
    if absolute && real_mode {
        targets.extend(code.real_aliases(target));
    }
    targets
}

/// The words of `code` that may run in real mode: each word that control
/// can reach from a word of `starts` with these steps:
/// - a fall-through;
/// - a direct branch;
/// - from a word that writes CTR or LR, an indirect branch to each
///   address that `bound` gives the word as a target;
/// - from a `bcctr` or a `bclr`, an indirect branch to each word that
///   [`real_indirect_words`] gives.
///
/// Real mode ignores EA bits 0 and 1, so an indirect branch reaches each
/// word at the real address of its target. The set does not hold a word
/// that runs in real mode only after one of these:
/// - an `hrfid`;
/// - an `rfid`, `mtmsr` or `mtmsrd` that is an unbounded switch, which
///   [`scan`] handles.
///
/// [PPC-Book1 p:25 s:2.4] The registers that `bclr` and `bcctr` alter are CTR and LR, so the MSR at the target is the MSR at the branch.
fn real_mode_words(code: &Code, starts: &BTreeSet<u64>, bound: &IndirectBound) -> RealMode {
    let mut reached = BTreeSet::new();
    let mut indirect = false;
    let mut pending: Vec<u64> = starts.iter().copied().collect();
    while let Some(at) = pending.pop() {
        if !code.holds(at) || !reached.insert(at) {
            continue;
        }
        for raw in code.words_at(at) {
            if flow(raw) != Flow::Stops {
                pending.extend(at.checked_add(4));
            }
            pending.extend(direct_targets(code, at, raw, true));
            if !indirect && is_indirect_branch(raw) {
                indirect = true;
                pending.extend(real_indirect_words(code, &bound.entries, &bound.targets));
            }
        }
        for &target in bound.targets.get(&at).into_iter().flatten() {
            pending.extend(code.real_aliases(target));
        }
    }
    RealMode::Words {
        words: reached,
        indirect,
    }
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
/// When two segments hold words there with different flows, the walk
/// cannot know which flow runs, so the flow is unknown.
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
