//! What a profile says about a text image: static counts, coverage of a
//! candidate set, and the JSON a run writes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

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

/// Where a report reads the word at a retired address.
pub trait WordSource {
    /// The word at `pc`, or `None` when nothing holds all four bytes.
    fn word(&self, pc: u64) -> Option<u32>;
}

/// A flat image whose first byte is at address 0.
impl WordSource for [u8] {
    fn word(&self, pc: u64) -> Option<u32> {
        word_at(0, self, pc)
    }
}

/// Guest memory: each word comes from the region that holds it. The read
/// logs no provisional read, because the report is a host observer.
impl WordSource for GuestMemory {
    fn word(&self, pc: u64) -> Option<u32> {
        let range = ByteRange::new(GuestAddr::new(pc), 4)?;
        self.with_reads_unlogged(|mem| {
            let b = mem.read(range)?;
            <[u8; 4]>::try_from(&*b).ok().map(u32::from_be_bytes)
        })
    }
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
pub fn shipped_coverage<M: WordSource + ?Sized>(
    profile: &OperandProfile,
    shadow: &PredecodedShadow,
    mem: &M,
) -> Coverage {
    let mut out = Coverage {
        retired: profile.retired(),
        ..Coverage::default()
    };
    for_each_shipped(profile, shadow, mem, |slot, n| {
        if slot.is_super_pair() {
            out.covered += 2 * n;
            out.dispatches_removed += n;
        } else {
            out.covered += n;
        }
    });
    out
}

/// The retirements of each quickened form and fused pair the shadow runs
/// for `profile`, by variant name, most retired first.
///
/// A fused pair counts the retirements of its head.
pub fn shipped_forms<M: WordSource + ?Sized>(
    profile: &OperandProfile,
    shadow: &PredecodedShadow,
    mem: &M,
) -> Vec<(String, u64)> {
    let mut forms: BTreeMap<String, u64> = BTreeMap::new();
    for_each_shipped(profile, shadow, mem, |slot, n| {
        *forms.entry(variant_name(slot)).or_default() += n;
    });
    rank(forms)
}

/// Calls `each` with every slot of `profile` that the shadow runs as a
/// quickened form or the head of a fused pair, and its retirements.
///
/// A slot is quickened as [`shipped_coverage`] says. The walk skips a
/// consumed slot, because a branch that reaches it runs its stored word.
fn for_each_shipped<M: WordSource + ?Sized>(
    profile: &OperandProfile,
    shadow: &PredecodedShadow,
    mem: &M,
    mut each: impl FnMut(&PpuInstruction, u64),
) {
    for (&pc, &n) in &profile.pcs {
        let Some(slot) = shadow.get(pc) else {
            continue;
        };
        if matches!(slot, PpuInstruction::Consumed) {
            continue;
        }
        if !slot.is_super_pair() {
            let raw = mem.word(pc).and_then(|w| decode::decode(w).ok());
            if !raw.is_some_and(|raw| raw != slot) {
                continue;
            }
        }
        each(&slot, n);
    }
}

fn variant_name(insn: &PpuInstruction) -> String {
    <&'static str>::from(insn).to_string()
}

/// The report of `units`' profiles summed, over the text the first
/// unit's shadow covers in `mem`.
///
/// The units must run in the address space whose memory is `mem`. For
/// units of several spaces, the caller reports each space alone and
/// merges the reports with [`ProfileReport::merge`]. The report reads
/// each retired word from the region that holds it.
pub fn report_units<'a>(
    units: impl IntoIterator<Item = &'a PpuExecutionUnit>,
    mem: &GuestMemory,
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
    let text = text_bytes(mem, shadow.base(), shadow.end());
    let shipped = shipped_coverage(&profile, shadow, mem);
    let mut report = ProfileReport::new(&profile, mem, &text, Some(shipped));
    report.shipped_forms = shipped_forms(&profile, shadow, mem);
    report
}

/// The words of `[base, end)` that a region of `mem` holds, region by
/// region. Each piece starts and ends on a word boundary of the text.
///
/// The boot shadow starts at 0, and the boot unmaps the memory below the
/// user text floor, so `mem` refuses a read of the whole range. The
/// region views leave out those bytes, which hold no code.
fn text_bytes(mem: &GuestMemory, base: u64, end: u64) -> Vec<u8> {
    let mut text = Vec::new();
    for view in mem.region_views() {
        let lo = view.base.max(base);
        let Some(lo) = lo.checked_add((4 - (lo - base) % 4) % 4) else {
            continue;
        };
        let hi = end.min(view.base.saturating_add(view.bytes.len() as u64));
        if lo >= hi {
            continue;
        }
        let len = (hi - lo) & !3;
        let from = usize::try_from(lo - view.base).ok();
        let to = from
            .zip(usize::try_from(len).ok())
            .and_then(|(f, n)| f.checked_add(n));
        if let Some(piece) = from.zip(to).and_then(|(f, t)| view.bytes.get(f..t)) {
            text.extend_from_slice(piece);
        }
    }
    text
}

/// A run's profile ranked for the shadow candidates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileReport {
    /// Every retirement the profile counted.
    pub retired: u64,
    /// The retirements that count in no class, because nothing holds
    /// their word or it does not decode.
    pub unresolved: u64,
    /// Each class with its retirements and its words in the text, most
    /// retired first.
    pub insns: Vec<(InsnClass, u64, u64)>,
    /// Each pair class with its retirements, most retired first.
    pub pairs: Vec<(PairClass, u64)>,
    /// What the shadow's quickened forms and fused pairs cover.
    pub shipped: Option<Coverage>,
    /// The retirements of each shipped form, as [`shipped_forms`] counts
    /// them. Empty when the report has no shadow.
    pub shipped_forms: Vec<(String, u64)>,
}

impl ProfileReport {
    /// Ranks `profile` by the words `mem` holds at its addresses, and
    /// counts the words of `text`.
    pub fn new<M: WordSource + ?Sized>(
        profile: &OperandProfile,
        mem: &M,
        text: &[u8],
        shipped: Option<Coverage>,
    ) -> Self {
        let (dynamic, pair_counts) = profile.classes(|pc| mem.word(pc));
        let retired = profile.retired();
        let unresolved = retired.saturating_sub(dynamic.values().sum());
        let mut rows: BTreeMap<InsnClass, (u64, u64)> = BTreeMap::new();
        for (class, n) in dynamic {
            rows.entry(class).or_default().0 += n;
        }
        for (class, n) in static_counts(text) {
            rows.entry(class).or_default().1 += n;
        }
        Self {
            retired,
            unresolved,
            insns: rank_insns(rows),
            pairs: rank(pair_counts),
            shipped,
            shipped_forms: Vec::new(),
        }
    }

    /// Adds `other`'s rows to this report's, row by row on each label.
    ///
    /// The shipped coverage sums over the reports that have one. It is
    /// `None` only when neither report has one.
    pub fn merge(&mut self, other: ProfileReport) {
        self.retired += other.retired;
        self.unresolved += other.unresolved;
        let mut rows: BTreeMap<InsnClass, (u64, u64)> = BTreeMap::new();
        for (class, d, s) in self.insns.drain(..).chain(other.insns) {
            let row = rows.entry(class).or_default();
            row.0 += d;
            row.1 += s;
        }
        self.insns = rank_insns(rows);
        self.pairs = rank(sum_rows(self.pairs.drain(..).chain(other.pairs)));
        self.shipped = match (self.shipped, other.shipped) {
            (Some(a), Some(b)) => Some(Coverage {
                retired: a.retired + b.retired,
                covered: a.covered + b.covered,
                dispatches_removed: a.dispatches_removed + b.dispatches_removed,
            }),
            (a, b) => a.or(b),
        };
        let forms = self.shipped_forms.drain(..).chain(other.shipped_forms);
        self.shipped_forms = rank(sum_rows(forms));
    }

    /// The report as JSON, with these keys in this order:
    ///
    /// - `retired` and `unresolved`
    /// - `insns` rows of `class`, `dynamic` and `static`
    /// - `pairs` rows of `pair` and `dynamic`
    /// - `shipped` coverage or `null`
    /// - `shipped_forms` rows of `form` and `dynamic`
    ///
    /// The label keys each row, so a caller can sum the reports of several
    /// runs row by row.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "{{\n  \"retired\": {},\n  \"unresolved\": {},\n  \"insns\": [",
            self.retired, self.unresolved
        );
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
        out.push_str(",\n  \"shipped_forms\": [");
        for (i, (form, dynamic)) in self.shipped_forms.iter().enumerate() {
            let sep = if i == 0 { "" } else { "," };
            let _ = write!(
                out,
                "{sep}\n    {{\"form\": \"{}\", \"dynamic\": {dynamic}}}",
                escape(form)
            );
        }
        out.push_str("\n  ]\n}\n");
        out
    }
}

/// The class rows, most retired first, then most words in the text, then
/// by class.
fn rank_insns(rows: BTreeMap<InsnClass, (u64, u64)>) -> Vec<(InsnClass, u64, u64)> {
    let mut insns: Vec<_> = rows.into_iter().map(|(c, (d, s))| (c, d, s)).collect();
    insns.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    insns
}

/// The rows, most retired first, then by key.
fn rank<K: Ord>(rows: BTreeMap<K, u64>) -> Vec<(K, u64)> {
    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows
}

/// The counts of `rows` summed on each key.
fn sum_rows<K: Ord>(rows: impl Iterator<Item = (K, u64)>) -> BTreeMap<K, u64> {
    let mut sums = BTreeMap::new();
    for (key, n) in rows {
        *sums.entry(key).or_default() += n;
    }
    sums
}

/// `s` with its JSON string metacharacters escaped.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
