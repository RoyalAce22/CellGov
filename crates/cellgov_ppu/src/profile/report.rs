//! What a profile says about a text image: static counts, coverage of a
//! candidate set, and the JSON a run writes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::class::{InsnClass, PairClass};
use super::counts::OperandProfile;
use crate::decode;
use crate::instruction::PpuInstruction;
use crate::shadow::PredecodedShadow;
use crate::PpuExecutionUnit;

/// What a candidate set covers of a profile's retirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// Every retirement the profile counted.
    pub retired: u64,
    /// The retirements a quickened form or a fused pair runs.
    pub covered: u64,
    /// The dispatches the fused pairs remove: one per fused retirement.
    pub dispatches_removed: u64,
}

/// The words of `text` that decode, counted by class.
pub fn static_counts(text: &[u8]) -> BTreeMap<InsnClass, u64> {
    let mut counts = BTreeMap::new();
    for word in text.chunks_exact(4) {
        let word = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        if let Some(class) = InsnClass::of(word) {
            *counts.entry(class).or_insert(0) += 1;
        }
    }
    counts
}

/// The word of `text` at `pc`, where `text` starts at `base`.
fn word_at(base: u64, text: &[u8], pc: u64) -> Option<u32> {
    let off = usize::try_from(pc.checked_sub(base)?).ok()?;
    let bytes = text.get(off..off.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// What `covers` and `fuses` would cover of `profile`'s retirements in
/// `text`, which starts at `base`.
///
/// `covers` names the instructions a quickening rewrites. `fuses` names
/// the pairs a fused form runs. Fusion walks the text from its start,
/// and the first pair of an overlapping run wins, as the shadow builds.
/// A pair removes one dispatch for each retirement of its head.
pub fn coverage(
    profile: &OperandProfile,
    base: u64,
    text: &[u8],
    covers: impl Fn(&PpuInstruction) -> bool,
    fuses: impl Fn(&PpuInstruction, &PpuInstruction) -> bool,
) -> Coverage {
    let decoded = |pc: u64| word_at(base, text, pc).and_then(|w| decode::decode(w).ok());
    let links = |pc: u64| match (decoded(pc), decoded(pc.wrapping_add(4))) {
        (Some(a), Some(b)) => fuses(&a, &b),
        _ => false,
    };
    // A slot heads a pair when the links of fusable pairs that run up
    // to it are even in number: the walk took every other link.
    let heads = |pc: u64| {
        if !links(pc) {
            return false;
        }
        let mut before = 0u64;
        let mut at = pc;
        while at >= base.saturating_add(4) && links(at - 4) {
            before += 1;
            at -= 4;
        }
        before.is_multiple_of(2)
    };
    let mut out = Coverage {
        retired: profile.retired(),
        ..Coverage::default()
    };
    for (&pc, &n) in &profile.pcs {
        if heads(pc) {
            out.covered += 2 * n;
            out.dispatches_removed += n;
        } else if !(pc >= 4 && heads(pc - 4)) && decoded(pc).is_some_and(|insn| covers(&insn)) {
            // A pair's second slot runs its stored word when a branch
            // reaches it, so neither form covers it.
            out.covered += n;
        }
    }
    out
}

/// What the shadow's own quickened forms and fused pairs cover of
/// `profile`'s retirements.
///
/// A slot counts as quickened when the shadow holds a variant other than
/// the decode of the word that `mem` holds there.
pub fn shipped_coverage(
    profile: &OperandProfile,
    shadow: &PredecodedShadow,
    mem: &[u8],
) -> Coverage {
    let mut out = Coverage {
        retired: profile.retired(),
        ..Coverage::default()
    };
    for (&pc, &n) in &profile.pcs {
        let Some(slot) = shadow.get(pc) else {
            continue;
        };
        if slot.is_super_pair() {
            out.covered += 2 * n;
            out.dispatches_removed += n;
            continue;
        }
        if matches!(slot, PpuInstruction::Consumed) {
            continue;
        }
        let raw = word_at(0, mem, pc).and_then(|w| decode::decode(w).ok());
        if raw.is_some_and(|raw| raw != slot) {
            out.covered += n;
        }
    }
    out
}

/// The report of `units`' profiles summed, over the text the first
/// unit's shadow covers in `mem`.
///
/// The units share one text image: a unit of another address space
/// counts its addresses against this one's text.
pub fn report_units<'a>(
    units: impl IntoIterator<Item = &'a PpuExecutionUnit>,
    mem: &[u8],
) -> ProfileReport {
    let mut profile = OperandProfile::default();
    let mut shadow = None;
    for unit in units {
        profile.merge(unit.operand_profile());
        shadow = shadow.or(unit.instruction_shadow());
    }
    let Some(shadow) = shadow else {
        return ProfileReport::new(&profile, mem, &[], None);
    };
    let text = usize::try_from(shadow.base())
        .ok()
        .zip(usize::try_from(shadow.end()).ok())
        .and_then(|(lo, hi)| mem.get(lo..hi))
        .unwrap_or_default();
    let shipped = shipped_coverage(&profile, shadow, mem);
    ProfileReport::new(&profile, mem, text, Some(shipped))
}

/// A run's profile ranked for the shadow candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileReport {
    /// Every retirement the profile counted.
    pub retired: u64,
    /// Each class with its retirements and its words in the text, most
    /// retired first.
    pub insns: Vec<(InsnClass, u64, u64)>,
    /// Each pair class with its retirements, most retired first.
    pub pairs: Vec<(PairClass, u64)>,
    /// What the shadow's quickened forms and fused pairs cover.
    pub shipped: Option<Coverage>,
}

impl ProfileReport {
    /// Ranks `profile` by the words `mem` holds at its addresses, and
    /// counts the words of `text`.
    pub fn new(
        profile: &OperandProfile,
        mem: &[u8],
        text: &[u8],
        shipped: Option<Coverage>,
    ) -> Self {
        let (dynamic, pair_counts) = profile.classes(|pc| word_at(0, mem, pc));
        let mut rows: BTreeMap<InsnClass, (u64, u64)> = BTreeMap::new();
        for (class, n) in dynamic {
            rows.entry(class).or_default().0 += n;
        }
        for (class, n) in static_counts(text) {
            rows.entry(class).or_default().1 += n;
        }
        let mut insns: Vec<_> = rows.into_iter().map(|(c, (d, s))| (c, d, s)).collect();
        insns.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
        let mut pairs: Vec<_> = pair_counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Self {
            retired: profile.retired(),
            insns,
            pairs,
            shipped,
        }
    }

    /// The report as JSON: `retired`, then `insns` rows of `class`,
    /// `dynamic` and `static`, `pairs` rows of `pair` and `dynamic`, and
    /// `shipped` coverage or `null`.
    ///
    /// The label keys each row, so a caller can sum the reports of several
    /// runs row by row.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        let _ = write!(out, "{{\n  \"retired\": {},\n  \"insns\": [", self.retired);
        for (i, (class, dynamic, stat)) in self.insns.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(
                out,
                "{sep}\n    {{\"class\": \"{}\", \"dynamic\": {dynamic}, \"static\": {stat}}}",
                escape(&class.label())
            );
        }
        out.push_str("\n  ],\n  \"pairs\": [");
        for (i, (pair, dynamic)) in self.pairs.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(
                out,
                "{sep}\n    {{\"pair\": \"{}\", \"dynamic\": {dynamic}}}",
                escape(&pair.label())
            );
        }
        out.push_str("\n  ],\n  \"shipped\": ");
        match self.shipped {
            Some(c) => {
                let _ = write!(
                    out,
                    "{{\"retired\": {}, \"covered\": {}, \"dispatches_removed\": {}}}",
                    c.retired, c.covered, c.dispatches_removed
                );
            }
            None => out.push_str("null"),
        }
        out.push_str("\n}\n");
        out
    }
}

/// `s` with its JSON string metacharacters escaped.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
