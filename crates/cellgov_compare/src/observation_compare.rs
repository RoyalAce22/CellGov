//! Observation-vs-observation comparator. Walks outcome, memory
//! regions (with per-region byte-divergence coalescing into
//! [`ByteDivergence`] runs), events, state hashes, and steps. A
//! region pair's identity / length mismatch short-circuits the
//! pair but not subsequent pairs.

use serde::{Deserialize, Serialize};

use crate::observation::{
    NamedMemoryRegion, Observation, ObservedEvent, ObservedHashes, ObservedOutcome,
};

/// Aggregate verdict from comparing two [`Observation`]s.
///
/// One field per compared dimension; `a_runner` / `b_runner` carry
/// the runner names from each observation's metadata so renderers can
/// label divergence lines without re-threading the source observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationCompareResult {
    /// True iff the two observations reported the same [`ObservedOutcome`].
    pub outcome_match: bool,
    /// Outcome reported by observation `a` (preserved verbatim for
    /// renderer use even when `outcome_match` is true).
    pub a_outcome: ObservedOutcome,
    /// Outcome reported by observation `b`.
    pub b_outcome: ObservedOutcome,
    /// Per-region comparison summary; see [`RegionCompareSummary`].
    pub region_compare: RegionCompareSummary,
    /// Event-sequence comparison verdict.
    pub event_compare: EventCompare,
    /// State-hash comparison verdict (same-runner only counts as
    /// divergence; see [`StateHashCompare`]).
    pub state_hash_compare: StateHashCompare,
    /// Step-count comparison verdict (same-runner only counts as
    /// divergence; see [`StepCompare`]).
    pub step_compare: StepCompare,
    /// Runner name from `a.metadata.runner` (e.g., `"cellgov"`,
    /// `"rpcs3"`).
    pub a_runner: String,
    /// Runner name from `b.metadata.runner`.
    pub b_runner: String,
}

/// Aggregate of per-region pair outcomes plus the raw region counts.
///
/// `a_count` / `b_count` are the lengths of each side's
/// `memory_regions` vector. When the counts disagree, `pairs` is
/// empty -- no per-pair walk happens because there is no
/// well-defined zipping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionCompareSummary {
    /// Number of regions in observation `a`.
    pub a_count: usize,
    /// Number of regions in observation `b`.
    pub b_count: usize,
    /// Per-pair outcomes in observation order. Empty when region
    /// counts disagree.
    pub pairs: Vec<RegionPairOutcome>,
}

/// Outcome for one zipped pair of [`NamedMemoryRegion`]s.
///
/// Variants are checked in order: identity, then length, then byte
/// content. The first mismatch terminates the pair (a length
/// mismatch suppresses the byte walk), but subsequent region pairs
/// are still walked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegionPairOutcome {
    /// Regions share name, address, length, and byte content.
    Match {
        /// Region name shared by both sides.
        name: String,
        /// Guest base address shared by both sides.
        addr: u64,
        /// Region length in bytes.
        length: u64,
    },
    /// Region pair disagrees on name or guest base address.
    ///
    /// Distinguishing name vs address is left to the renderer; both
    /// pairs are recorded so callers can report the actual mismatch.
    IdentityMismatch {
        /// Region name from observation `a`.
        a_name: String,
        /// Guest base address from observation `a`.
        a_addr: u64,
        /// Region name from observation `b`.
        b_name: String,
        /// Guest base address from observation `b`.
        b_addr: u64,
    },
    /// Region pair shares identity but disagrees on byte length.
    /// Suppresses the byte-level walk for this pair.
    LengthMismatch {
        /// Region name (matches on both sides).
        name: String,
        /// Byte length of observation `a`'s data buffer.
        a_length: u64,
        /// Byte length of observation `b`'s data buffer.
        b_length: u64,
    },
    /// Region pair shares identity and length but has at least one
    /// differing byte.
    ByteDivergence {
        /// Region name (matches on both sides).
        name: String,
        /// Guest base address (matches on both sides).
        addr: u64,
        /// Region length in bytes (matches on both sides).
        length: u64,
        /// Coalesced runs of differing bytes within the region, in
        /// ascending offset order. A single differing byte produces
        /// one entry with `length == 1`; a contiguous run produces
        /// one entry with `length == N`.
        bytes: Vec<ByteDivergence>,
    },
}

/// One coalesced run of differing bytes within a region.
///
/// Only the first byte pair (`a_byte`, `b_byte`) is recorded; the
/// full run requires holding the source observations alongside this
/// result. Consumers that need every diverging byte must reopen the
/// source observations and re-slice by `(name, offset, length)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteDivergence {
    /// Byte offset within the region where this run starts.
    pub offset: u64,
    /// Always >= 1; the producer asserts this and classifier
    /// consumers may `debug_assert!` it too.
    pub length: u64,
    /// Byte from observation `a` at `offset` (only the first pair in
    /// the run is recorded; consumers needing more must reopen the
    /// source observation).
    pub a_byte: u8,
    /// Byte from observation `b` at `offset`.
    pub b_byte: u8,
}

/// Step-count comparison verdict.
///
/// Step counts are reported only by runners that expose an internal
/// step counter (CellGov). Same-runner mismatches indicate
/// non-determinism; cross-runner mismatches are notes because the
/// two runners can legitimately reach the same observable state via
/// different amounts of internal work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepCompare {
    /// Neither observation carries a step count.
    NoStepInfo,
    /// Both observations report the same step count.
    Equal {
        /// Step count shared by both sides.
        steps: usize,
    },
    /// Differing step counts within the same runner: a determinism
    /// failure.
    SameRunnerMismatch {
        /// Step count from observation `a`.
        a: usize,
        /// Step count from observation `b`.
        b: usize,
    },
    /// Differing step counts across runners: informational only.
    CrossRunnerNote {
        /// Step count from observation `a`.
        a: usize,
        /// Step count from observation `b`.
        b: usize,
    },
    /// One observation reports a step count, the other does not.
    /// Producer guarantees exactly one of `a` / `b` is `Some`.
    OneMissing {
        /// Step count from observation `a`, or `None` if absent.
        a: Option<usize>,
        /// Step count from observation `b`, or `None` if absent.
        b: Option<usize>,
    },
}

/// Event-sequence comparison verdict.
///
/// Equality is strict by index: the producer is responsible for
/// emitting events in the normalized order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventCompare {
    /// Both sequences have identical events at every index.
    Equal {
        /// Number of events in the (matching) sequences.
        count: usize,
    },
    /// Sequences differ in length; per-index walk is suppressed.
    LengthMismatch {
        /// Number of events in observation `a`.
        a: usize,
        /// Number of events in observation `b`.
        b: usize,
    },
    /// First index where the two sequences disagree. `index < min(a_len, b_len)`.
    FirstIndexDiffers {
        /// Zero-based index of the first differing event.
        index: usize,
        /// Event from observation `a` at `index`.
        a: ObservedEvent,
        /// Event from observation `b` at `index`.
        b: ObservedEvent,
    },
}

/// CellGov state-hash comparison verdict.
///
/// The RPCS3 adapter sets `state_hashes` to `None` (see
/// [`ObservedHashes`] doc), so cross-runner pairs land in
/// `OneMissing` or `NoHashInfo` and are never a divergence. A
/// same-runner pair carrying differing hashes is a determinism
/// failure analogous to [`StepCompare::SameRunnerMismatch`].
///
/// If a second runner ever starts emitting state hashes, the
/// `CrossRunnerNote` arm has to grow a real-vs-noise decision:
/// today it silently buries any cross-runner state-hash
/// divergence, which is safe only while RPCS3 is the only
/// non-CellGov producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateHashCompare {
    /// Neither observation carries state hashes.
    NoHashInfo,
    /// Both observations carry matching state hashes.
    Equal,
    /// One side carries state hashes and the other does not.
    OneMissing {
        /// True iff observation `a` carried state hashes.
        a_present: bool,
        /// True iff observation `b` carried state hashes.
        b_present: bool,
    },
    /// Same-runner pair with differing hashes; a determinism failure.
    SameRunnerMismatch {
        /// Hashes from observation `a`.
        a: ObservedHashes,
        /// Hashes from observation `b`.
        b: ObservedHashes,
    },
    /// Cross-runner pair with differing hashes; informational only
    /// (state-hash shape is CellGov-defined).
    CrossRunnerNote {
        /// Hashes from observation `a`.
        a: ObservedHashes,
        /// Hashes from observation `b`.
        b: ObservedHashes,
    },
}

impl RegionCompareSummary {
    /// True iff the two observations reported different numbers of
    /// regions. When true, `pairs` is empty.
    pub fn is_count_mismatch(&self) -> bool {
        self.a_count != self.b_count
    }

    /// True iff at least one zipped pair is anything other than
    /// [`RegionPairOutcome::Match`].
    pub fn has_pair_divergence(&self) -> bool {
        self.pairs
            .iter()
            .any(|p| !matches!(p, RegionPairOutcome::Match { .. }))
    }

    /// Total bytes across all [`RegionPairOutcome::Match`] entries
    /// (sum of their `length` fields). Used by the MATCH summary line.
    pub fn matched_bytes(&self) -> u64 {
        self.pairs
            .iter()
            .filter_map(|p| match p {
                RegionPairOutcome::Match { length, .. } => Some(*length),
                _ => None,
            })
            .sum()
    }

    /// Number of [`RegionPairOutcome::Match`] entries in `pairs`.
    pub fn matched_regions(&self) -> u64 {
        self.pairs
            .iter()
            .filter(|p| matches!(p, RegionPairOutcome::Match { .. }))
            .count() as u64
    }
}

impl EventCompare {
    /// True iff the event sequences are non-equal.
    pub fn is_divergent(&self) -> bool {
        !matches!(self, Self::Equal { .. })
    }
}

impl StateHashCompare {
    /// True iff this verdict drives a non-zero exit code (only
    /// same-runner hash mismatches qualify; cross-runner mismatches
    /// are informational).
    pub fn is_divergent(&self) -> bool {
        matches!(self, Self::SameRunnerMismatch { .. })
    }
}

impl ObservationCompareResult {
    /// True iff anything in this result drives a non-zero exit code:
    /// outcome mismatch, any region-side mismatch, an event-sequence
    /// mismatch, a same-runner step mismatch, or a same-runner
    /// state-hash mismatch. Cross-runner step / state-hash mismatches
    /// are notes, not divergences.
    pub fn has_divergence(&self) -> bool {
        !self.outcome_match
            || self.region_compare.is_count_mismatch()
            || self.region_compare.has_pair_divergence()
            || self.event_compare.is_divergent()
            || self.state_hash_compare.is_divergent()
            || matches!(self.step_compare, StepCompare::SameRunnerMismatch { .. })
    }

    /// True iff outcomes and regions match and both observations
    /// reported zero regions. Drives the CLI's WARN line on stderr
    /// for the "nothing was compared" case.
    pub fn is_vacuous(&self) -> bool {
        self.outcome_match
            && !self.region_compare.is_count_mismatch()
            && !self.region_compare.has_pair_divergence()
            && self.region_compare.a_count == 0
            && self.region_compare.b_count == 0
    }

    /// Returns `Some((a_steps, b_steps))` exactly when the CLI
    /// prints its `NOTE: step counts differ ...` stderr line; cross-runner
    /// step divergence is informational, not a divergence.
    pub fn cross_runner_step_note(&self) -> Option<(usize, usize)> {
        if let StepCompare::CrossRunnerNote { a, b } = self.step_compare {
            Some((a, b))
        } else {
            None
        }
    }
}

/// Compare two observations.
///
/// Compared fields: `outcome`, `memory_regions` (region identity,
/// length, and byte content), `events` (strict equality by index),
/// `state_hashes` (same-runner only; see [`StateHashCompare`]), and
/// `metadata.steps`.
///
/// `tty_log` is informational and is NOT compared; cross-runner TTY
/// streams can legitimately differ in trailing newlines / framing.
///
/// Per region, all differing bytes are recorded as coalesced runs;
/// region pairs are walked in observation order even when an earlier
/// region had byte divergences. Region identity / length mismatches
/// terminate that pair (no byte-level walk on a malformed pair) but
/// do not halt the next pair.
pub fn compare_observations(a: &Observation, b: &Observation) -> ObservationCompareResult {
    let region_compare = compare_regions(&a.memory_regions, &b.memory_regions);
    let event_compare = compare_events(&a.events, &b.events);
    let state_hash_compare = compare_state_hashes(
        a.state_hashes.as_ref(),
        b.state_hashes.as_ref(),
        &a.metadata.runner,
        &b.metadata.runner,
    );
    let step_compare = compare_steps(
        a.metadata.steps,
        b.metadata.steps,
        &a.metadata.runner,
        &b.metadata.runner,
    );
    ObservationCompareResult {
        outcome_match: a.outcome == b.outcome,
        a_outcome: a.outcome,
        b_outcome: b.outcome,
        region_compare,
        event_compare,
        state_hash_compare,
        step_compare,
        a_runner: a.metadata.runner.clone(),
        b_runner: b.metadata.runner.clone(),
    }
}

fn compare_regions(a: &[NamedMemoryRegion], b: &[NamedMemoryRegion]) -> RegionCompareSummary {
    let mut pairs = Vec::new();
    if a.len() == b.len() {
        for (ra, rb) in a.iter().zip(b.iter()) {
            if ra.name != rb.name || ra.addr != rb.addr {
                pairs.push(RegionPairOutcome::IdentityMismatch {
                    a_name: ra.name.clone(),
                    a_addr: ra.addr,
                    b_name: rb.name.clone(),
                    b_addr: rb.addr,
                });
                continue;
            }
            if ra.data.len() != rb.data.len() {
                pairs.push(RegionPairOutcome::LengthMismatch {
                    name: ra.name.clone(),
                    a_length: ra.data.len() as u64,
                    b_length: rb.data.len() as u64,
                });
                continue;
            }
            let runs = collect_byte_divergences(&ra.data, &rb.data);
            if runs.is_empty() {
                pairs.push(RegionPairOutcome::Match {
                    name: ra.name.clone(),
                    addr: ra.addr,
                    length: ra.data.len() as u64,
                });
            } else {
                pairs.push(RegionPairOutcome::ByteDivergence {
                    name: ra.name.clone(),
                    addr: ra.addr,
                    length: ra.data.len() as u64,
                    bytes: runs,
                });
            }
        }
    }
    RegionCompareSummary {
        a_count: a.len(),
        b_count: b.len(),
        pairs,
    }
}

/// Emit one [`ByteDivergence`] per contiguous run of differing
/// bytes.
fn collect_byte_divergences(a: &[u8], b: &[u8]) -> Vec<ByteDivergence> {
    debug_assert_eq!(
        a.len(),
        b.len(),
        "collect_byte_divergences requires equal-length slices"
    );
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            let start = i;
            let a_first = a[i];
            let b_first = b[i];
            while i < a.len() && a[i] != b[i] {
                i += 1;
            }
            let div = ByteDivergence {
                offset: start as u64,
                length: (i - start) as u64,
                a_byte: a_first,
                b_byte: b_first,
            };
            debug_assert!(
                div.length >= 1,
                "ByteDivergence::length must be >= 1; producer bug"
            );
            out.push(div);
        } else {
            i += 1;
        }
    }
    out
}

fn compare_events(a: &[ObservedEvent], b: &[ObservedEvent]) -> EventCompare {
    if a.len() != b.len() {
        return EventCompare::LengthMismatch {
            a: a.len(),
            b: b.len(),
        };
    }
    for (index, (ea, eb)) in a.iter().zip(b.iter()).enumerate() {
        if ea != eb {
            return EventCompare::FirstIndexDiffers {
                index,
                a: *ea,
                b: *eb,
            };
        }
    }
    EventCompare::Equal { count: a.len() }
}

fn compare_state_hashes(
    a: Option<&ObservedHashes>,
    b: Option<&ObservedHashes>,
    a_runner: &str,
    b_runner: &str,
) -> StateHashCompare {
    match (a, b) {
        (None, None) => StateHashCompare::NoHashInfo,
        (Some(ha), Some(hb)) if ha == hb => StateHashCompare::Equal,
        (Some(ha), Some(hb)) if a_runner == b_runner => {
            StateHashCompare::SameRunnerMismatch { a: *ha, b: *hb }
        }
        (Some(ha), Some(hb)) => StateHashCompare::CrossRunnerNote { a: *ha, b: *hb },
        (a, b) => {
            debug_assert!(
                matches!((a.is_some(), b.is_some()), (true, false) | (false, true)),
                "OneMissing requires exactly one Some, got (a_present={}, b_present={})",
                a.is_some(),
                b.is_some()
            );
            StateHashCompare::OneMissing {
                a_present: a.is_some(),
                b_present: b.is_some(),
            }
        }
    }
}

fn compare_steps(
    a_steps: Option<usize>,
    b_steps: Option<usize>,
    a_runner: &str,
    b_runner: &str,
) -> StepCompare {
    match (a_steps, b_steps) {
        (None, None) => StepCompare::NoStepInfo,
        (Some(sa), Some(sb)) if sa == sb => StepCompare::Equal { steps: sa },
        (Some(sa), Some(sb)) if a_runner == b_runner => {
            StepCompare::SameRunnerMismatch { a: sa, b: sb }
        }
        (Some(sa), Some(sb)) => StepCompare::CrossRunnerNote { a: sa, b: sb },
        (a, b) => {
            debug_assert!(
                matches!((a, b), (Some(_), None) | (None, Some(_))),
                "OneMissing requires exactly one Some, got ({a:?}, {b:?})"
            );
            StepCompare::OneMissing { a, b }
        }
    }
}

/// Render the compare result to the stdout format the
/// `compare-observations` CLI emits.
///
/// Fields are walked in fixed order (outcome -> regions -> events ->
/// state hashes -> steps); every divergent section emits its own
/// DIVERGE line. The MATCH summary line at the end appears only when
/// `has_divergence()` is false. The caller is responsible for the
/// stderr WARN / NOTE lines around vacuous comparisons and
/// cross-runner notes; see [`ObservationCompareResult::is_vacuous`]
/// and [`ObservationCompareResult::cross_runner_step_note`].
pub fn format_observation_compare_human(result: &ObservationCompareResult) -> String {
    let mut out = String::new();
    if !result.outcome_match {
        out.push_str(&format!(
            "DIVERGE outcome: {}={:?} vs {}={:?}\n",
            result.a_runner, result.a_outcome, result.b_runner, result.b_outcome,
        ));
    }
    if result.region_compare.is_count_mismatch() {
        out.push_str(&format!(
            "DIVERGE region count: {} vs {}\n",
            result.region_compare.a_count, result.region_compare.b_count
        ));
    }
    for pair in &result.region_compare.pairs {
        match pair {
            RegionPairOutcome::Match { .. } => continue,
            RegionPairOutcome::IdentityMismatch {
                a_name,
                a_addr,
                b_name,
                b_addr,
            } => {
                out.push_str(&format!(
                    "DIVERGE region identity: {}@0x{:x} vs {}@0x{:x}\n",
                    a_name, a_addr, b_name, b_addr
                ));
            }
            RegionPairOutcome::LengthMismatch {
                name,
                a_length,
                b_length,
            } => {
                out.push_str(&format!(
                    "DIVERGE region {}: length {} vs {} bytes\n",
                    name, a_length, b_length
                ));
            }
            RegionPairOutcome::ByteDivergence {
                name, addr, bytes, ..
            } => {
                for div in bytes {
                    debug_assert!(
                        addr.checked_add(div.offset)
                            .and_then(|s| s.checked_add(div.length))
                            .is_some(),
                        "guest address arithmetic overflowed u64"
                    );
                    if div.length == 1 {
                        out.push_str(&format!(
                            "DIVERGE region {}: byte at offset 0x{:x} (guest 0x{:x}) -- {:02x} vs {:02x}\n",
                            name,
                            div.offset,
                            addr + div.offset,
                            div.a_byte,
                            div.b_byte,
                        ));
                    } else {
                        out.push_str(&format!(
                            "DIVERGE region {}: run of {} bytes at offset 0x{:x}..0x{:x} (guest 0x{:x}..0x{:x}) -- first pair {:02x} vs {:02x}\n",
                            name,
                            div.length,
                            div.offset,
                            div.offset + div.length,
                            addr + div.offset,
                            addr + div.offset + div.length,
                            div.a_byte,
                            div.b_byte,
                        ));
                    }
                }
            }
        }
    }
    match &result.event_compare {
        EventCompare::Equal { .. } => {}
        EventCompare::LengthMismatch { a, b } => {
            out.push_str(&format!("DIVERGE event count: {a} vs {b}\n"));
        }
        EventCompare::FirstIndexDiffers { index, a, b } => {
            out.push_str(&format!("DIVERGE event at index {index}: {a:?} vs {b:?}\n"));
        }
    }
    if let StateHashCompare::SameRunnerMismatch { a, b } = &result.state_hash_compare {
        out.push_str(&format!(
            "DIVERGE state hashes within runner '{}': {a:?} vs {b:?}\n",
            result.a_runner,
        ));
    }
    if let StepCompare::SameRunnerMismatch { a, b } = result.step_compare {
        out.push_str(&format!(
            "DIVERGE step count: {a} vs {b} within runner '{}' (byte-equal state reached via different work -- a determinism failure)\n",
            result.a_runner,
        ));
    }
    if !result.has_divergence() {
        let (sa, sb) = steps_pair(&result.step_compare);
        let event_count = match &result.event_compare {
            EventCompare::Equal { count } => *count,
            EventCompare::LengthMismatch { .. } | EventCompare::FirstIndexDiffers { .. } => {
                unreachable!("has_divergence() filters out event divergence")
            }
        };
        let hash_label = match &result.state_hash_compare {
            StateHashCompare::Equal => "state hashes equal",
            StateHashCompare::NoHashInfo => "no state hashes",
            StateHashCompare::OneMissing { .. } => "state hashes one-sided",
            StateHashCompare::CrossRunnerNote { .. } => "state hashes differ (cross-runner)",
            StateHashCompare::SameRunnerMismatch { .. } => {
                unreachable!("has_divergence() filters out same-runner hash mismatches")
            }
        };
        out.push_str(&format!(
            "MATCH outcome={:?}, {} regions ({} bytes) identical, {} events, {}, steps {:?} vs {:?}\n",
            result.a_outcome,
            result.region_compare.matched_regions(),
            result.region_compare.matched_bytes(),
            event_count,
            hash_label,
            sa,
            sb,
        ));
    }
    out
}

/// Serialize the compare result as pretty JSON.
pub fn format_observation_compare_json(
    result: &ObservationCompareResult,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(result)
}

fn steps_pair(sc: &StepCompare) -> (Option<usize>, Option<usize>) {
    match sc {
        StepCompare::NoStepInfo => (None, None),
        StepCompare::Equal { steps } => (Some(*steps), Some(*steps)),
        StepCompare::SameRunnerMismatch { a, b } => (Some(*a), Some(*b)),
        StepCompare::CrossRunnerNote { a, b } => (Some(*a), Some(*b)),
        StepCompare::OneMissing { a, b } => (*a, *b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{
        ObservationMetadata, ObservedEvent, ObservedEventKind, ObservedOutcome,
    };
    use cellgov_trace::StateHash;

    fn obs(
        outcome: ObservedOutcome,
        regions: Vec<NamedMemoryRegion>,
        runner: &str,
        steps: Option<usize>,
    ) -> Observation {
        Observation {
            outcome,
            memory_regions: regions,
            events: Vec::new(),
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: runner.to_string(),
                steps,
            },
            tty_log: Vec::new(),
        }
    }

    fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
        NamedMemoryRegion {
            name: name.to_string(),
            addr,
            data,
        }
    }

    fn evt(kind: ObservedEventKind, unit: u64, sequence: u32) -> ObservedEvent {
        ObservedEvent {
            kind,
            unit,
            sequence,
        }
    }

    fn hashes(memory: u64, unit_status: u64, sync: u64) -> ObservedHashes {
        ObservedHashes {
            memory: StateHash::new(memory),
            unit_status: StateHash::new(unit_status),
            sync: StateHash::new(sync),
        }
    }

    #[test]
    fn identical_observations_match() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![1, 2, 3, 4])],
            "cellgov",
            Some(100),
        );
        let b = a.clone();
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert!(out.contains("MATCH outcome=Completed"));
        assert!(out.contains("1 regions (4 bytes) identical"));
        assert!(out.contains("steps Some(100) vs Some(100)"));
    }

    #[test]
    fn outcome_mismatch_is_divergence() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = obs(ObservedOutcome::Fault, vec![], "rpcs3", Some(1));
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert!(out.starts_with("DIVERGE outcome: cellgov=Completed vs rpcs3=Fault\n"));
    }

    #[test]
    fn outcome_mismatch_with_region_divergence_renders_both_lines() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0u8; 4];
        b_data[2] = 0xAA;
        let b = obs(
            ObservedOutcome::Fault,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert!(
            out.contains("DIVERGE outcome:"),
            "outcome line should render alongside region line: {out}"
        );
        assert!(
            out.contains("DIVERGE region code: byte at offset 0x2"),
            "region byte line should render after outcome line: {out}"
        );
        assert!(!out.contains("MATCH"));
    }

    #[test]
    fn region_count_mismatch_is_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("a", 0, vec![0])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("a", 0, vec![0]), region("b", 1, vec![0])],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert_eq!(out, "DIVERGE region count: 1 vs 2\n");
    }

    #[test]
    fn identity_mismatch_diverges_with_prior_format() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x20000, vec![0])],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        let out = format_observation_compare_human(&r);
        assert_eq!(
            out,
            "DIVERGE region identity: code@0x10000 vs data@0x20000\n"
        );
    }

    #[test]
    fn length_mismatch_diverges_with_prior_format() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0; 4])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0; 8])],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        let out = format_observation_compare_human(&r);
        assert_eq!(out, "DIVERGE region code: length 4 vs 8 bytes\n");
    }

    #[test]
    fn single_byte_divergence_renders_byte_format() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0x00; 0x40])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0x00u8; 0x40];
        b_data[0x17] = 0x01;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert_eq!(
            out,
            "DIVERGE region code: byte at offset 0x17 (guest 0x10017) -- 00 vs 01\n"
        );
    }

    #[test]
    fn divergence_at_offset_zero_renders() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0x00; 4])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0x00u8; 4];
        b_data[0] = 0xFF;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert_eq!(
            out,
            "DIVERGE region code: byte at offset 0x0 (guest 0x10000) -- 00 vs ff\n"
        );
    }

    #[test]
    fn divergence_at_last_byte_renders() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0x00; 4])],
            "cellgov",
            Some(1),
        );
        let mut b_data = vec![0x00u8; 4];
        b_data[3] = 0x55;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(1),
        );
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert_eq!(
            out,
            "DIVERGE region code: byte at offset 0x3 (guest 0x10003) -- 00 vs 55\n"
        );
    }

    #[test]
    fn empty_region_pair_matches() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("empty", 0x10000, vec![])],
            "cellgov",
            Some(1),
        );
        let b = a.clone();
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert_eq!(r.region_compare.matched_regions(), 1);
        assert_eq!(r.region_compare.matched_bytes(), 0);
    }

    #[test]
    fn contiguous_run_coalesces_into_one_byte_divergence() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0x00; 8])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0x01; 8])],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        let pair = &r.region_compare.pairs[0];
        match pair {
            RegionPairOutcome::ByteDivergence { bytes, .. } => {
                assert_eq!(bytes.len(), 1, "contiguous run coalesces to one entry");
                assert_eq!(bytes[0].offset, 0);
                assert_eq!(bytes[0].length, 8);
                assert_eq!(bytes[0].a_byte, 0x00);
                assert_eq!(bytes[0].b_byte, 0x01);
            }
            other => panic!("expected ByteDivergence, got {other:?}"),
        }
    }

    #[test]
    fn run_renders_with_length_and_first_pair() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0xAA; 4])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("data", 0x80000, vec![0xBB; 4])],
            "rpcs3",
            Some(1),
        );
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert_eq!(
            out,
            "DIVERGE region data: run of 4 bytes at offset 0x0..0x4 (guest 0x80000..0x80004) -- first pair aa vs bb\n"
        );
    }

    #[test]
    fn non_contiguous_divergences_become_separate_runs() {
        let mut a_data = vec![0u8; 16];
        let mut b_data = vec![0u8; 16];
        b_data[1] = 0x10;
        b_data[2] = 0x10;
        b_data[5] = 0x20;
        b_data[10] = 0x30;
        b_data[11] = 0x30;
        b_data[12] = 0x30;
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x100, a_data.clone())],
            "cellgov",
            Some(1),
        );
        a_data[0] = 0;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x100, b_data)],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        let pair = &r.region_compare.pairs[0];
        match pair {
            RegionPairOutcome::ByteDivergence { bytes, .. } => {
                assert_eq!(bytes.len(), 3, "three separate runs");
                assert_eq!(bytes[0].offset, 1);
                assert_eq!(bytes[0].length, 2);
                assert_eq!(bytes[1].offset, 5);
                assert_eq!(bytes[1].length, 1);
                assert_eq!(bytes[2].offset, 10);
                assert_eq!(bytes[2].length, 3);
            }
            other => panic!("expected ByteDivergence, got {other:?}"),
        }
    }

    #[test]
    fn three_runs_in_one_region_render_three_diverge_lines_in_offset_order() {
        let mut b_data = vec![0u8; 16];
        b_data[1] = 0x10;
        b_data[5] = 0x20;
        b_data[10] = 0x30;
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x100, vec![0u8; 16])],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x100, b_data)],
            "rpcs3",
            Some(1),
        );
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        let off1 = out.find("offset 0x1 ").expect("first run line missing");
        let off5 = out.find("offset 0x5 ").expect("second run line missing");
        let off10 = out.find("offset 0xa ").expect("third run line missing");
        assert!(off1 < off5 && off5 < off10, "ascending offset order: {out}");
    }

    #[test]
    fn multi_region_divergences_are_all_walked() {
        let mut a_code = vec![0u8; 8];
        let mut b_code = vec![0u8; 8];
        b_code[3] = 0x55;
        let mut a_data = vec![0u8; 4];
        let mut b_data = vec![0u8; 4];
        b_data[0] = 0xCC;
        b_data[1] = 0xDD;
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, a_code.clone()),
                region("data", 0x80000, a_data.clone()),
            ],
            "cellgov",
            Some(1),
        );
        a_code[0] = 0;
        a_data[0] = 0;
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("code", 0x10000, b_code),
                region("data", 0x80000, b_data),
            ],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        assert_eq!(r.region_compare.pairs.len(), 2);
        assert!(matches!(
            r.region_compare.pairs[0],
            RegionPairOutcome::ByteDivergence { .. }
        ));
        assert!(matches!(
            r.region_compare.pairs[1],
            RegionPairOutcome::ByteDivergence { .. }
        ));
        let out = format_observation_compare_human(&r);
        assert!(
            out.contains("DIVERGE region code: byte at offset 0x3"),
            "got: {out}"
        );
        assert!(
            out.contains("DIVERGE region data: run of 2 bytes"),
            "got: {out}"
        );
        assert!(!out.contains("MATCH"));
    }

    #[test]
    fn length_mismatch_in_one_region_does_not_block_subsequent_regions() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("first", 0x10000, vec![0u8; 4]),
                region("second", 0x20000, vec![0xAA; 4]),
            ],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("first", 0x10000, vec![0u8; 8]),
                region("second", 0x20000, vec![0xBB; 4]),
            ],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert_eq!(r.region_compare.pairs.len(), 2);
        assert!(matches!(
            r.region_compare.pairs[0],
            RegionPairOutcome::LengthMismatch { .. }
        ));
        assert!(matches!(
            r.region_compare.pairs[1],
            RegionPairOutcome::ByteDivergence { .. }
        ));
    }

    #[test]
    fn matching_regions_before_diverging_region_are_recorded() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![
                region("first", 0x10000, vec![0xAA; 4]),
                region("second", 0x20000, vec![0xBB; 4]),
            ],
            "cellgov",
            Some(1),
        );
        let b = obs(
            ObservedOutcome::Completed,
            vec![
                region("first", 0x10000, vec![0xAA; 4]),
                region("second", 0x20000, vec![0xCC; 4]),
            ],
            "rpcs3",
            Some(1),
        );
        let r = compare_observations(&a, &b);
        assert_eq!(r.region_compare.pairs.len(), 2);
        assert!(matches!(
            r.region_compare.pairs[0],
            RegionPairOutcome::Match { .. }
        ));
        assert!(matches!(
            r.region_compare.pairs[1],
            RegionPairOutcome::ByteDivergence { .. }
        ));
    }

    #[test]
    fn same_runner_step_mismatch_renders_diverge_without_match_summary() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
        let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        let out = format_observation_compare_human(&r);
        assert!(out.contains("DIVERGE step count: 100 vs 200 within runner 'cellgov'"));
        assert!(!out.contains("MATCH outcome="));
    }

    #[test]
    fn cross_runner_step_mismatch_is_note_not_divergence() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(200));
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert_eq!(r.cross_runner_step_note(), Some((100, 200)));
        let out = format_observation_compare_human(&r);
        assert!(out.contains("MATCH outcome=Completed"));
        assert!(!out.contains("DIVERGE"));
    }

    #[test]
    fn zero_regions_both_sides_is_vacuous() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert!(r.is_vacuous());
    }

    #[test]
    fn nonempty_regions_are_not_vacuous() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0, vec![0])],
            "cellgov",
            Some(1),
        );
        let b = a.clone();
        let r = compare_observations(&a, &b);
        assert!(!r.is_vacuous());
    }

    #[test]
    fn step_compare_no_step_info() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", None);
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", None);
        let r = compare_observations(&a, &b);
        assert_eq!(r.step_compare, StepCompare::NoStepInfo);
        assert!(!r.has_divergence());
    }

    #[test]
    fn step_compare_b_missing() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(50));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", None);
        let r = compare_observations(&a, &b);
        assert_eq!(
            r.step_compare,
            StepCompare::OneMissing {
                a: Some(50),
                b: None
            }
        );
        assert!(!r.has_divergence());
    }

    #[test]
    fn step_compare_a_missing() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", None);
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(50));
        let r = compare_observations(&a, &b);
        assert_eq!(
            r.step_compare,
            StepCompare::OneMissing {
                a: None,
                b: Some(50)
            }
        );
        assert!(!r.has_divergence());
    }

    #[test]
    fn step_compare_equal_steps() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(42));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(42));
        let r = compare_observations(&a, &b);
        assert_eq!(r.step_compare, StepCompare::Equal { steps: 42 });
    }

    #[test]
    fn events_equal_when_sequences_match() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.events = vec![
            evt(ObservedEventKind::MailboxSend, 1, 0),
            evt(ObservedEventKind::DmaComplete, 2, 1),
        ];
        b.events = a.events.clone();
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert_eq!(r.event_compare, EventCompare::Equal { count: 2 });
    }

    #[test]
    fn events_length_differs_is_divergence() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.events = vec![evt(ObservedEventKind::MailboxSend, 1, 0)];
        b.events = vec![
            evt(ObservedEventKind::MailboxSend, 1, 0),
            evt(ObservedEventKind::UnitWake, 1, 1),
        ];
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        assert_eq!(r.event_compare, EventCompare::LengthMismatch { a: 1, b: 2 });
        let out = format_observation_compare_human(&r);
        assert!(out.contains("DIVERGE event count: 1 vs 2"), "got: {out}");
    }

    #[test]
    fn events_differ_at_index_is_divergence() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.events = vec![
            evt(ObservedEventKind::MailboxSend, 1, 0),
            evt(ObservedEventKind::MailboxReceive, 2, 1),
            evt(ObservedEventKind::DmaComplete, 3, 2),
        ];
        b.events = vec![
            evt(ObservedEventKind::MailboxSend, 1, 0),
            evt(ObservedEventKind::MailboxReceive, 2, 1),
            evt(ObservedEventKind::UnitBlock, 3, 2),
        ];
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        assert!(matches!(
            r.event_compare,
            EventCompare::FirstIndexDiffers { index: 2, .. }
        ));
        let out = format_observation_compare_human(&r);
        assert!(out.contains("DIVERGE event at index 2"), "got: {out}");
    }

    #[test]
    fn state_hash_equal_when_present_and_matching() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        b.state_hashes = a.state_hashes;
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert_eq!(r.state_hash_compare, StateHashCompare::Equal);
    }

    #[test]
    fn state_hash_same_runner_mismatch_is_divergence() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        b.state_hashes = Some(hashes(1, 2, 4));
        let r = compare_observations(&a, &b);
        assert!(r.has_divergence());
        assert!(matches!(
            r.state_hash_compare,
            StateHashCompare::SameRunnerMismatch { .. }
        ));
        let out = format_observation_compare_human(&r);
        assert!(
            out.contains("DIVERGE state hashes within runner 'cellgov'"),
            "got: {out}"
        );
    }

    #[test]
    fn state_hash_cross_runner_mismatch_is_note() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        b.state_hashes = Some(hashes(9, 9, 9));
        let r = compare_observations(&a, &b);
        // Different runners: not a divergence (state-hash shape is
        // CellGov-defined; RPCS3 would normally set None).
        assert!(!r.has_divergence());
        assert!(matches!(
            r.state_hash_compare,
            StateHashCompare::CrossRunnerNote { .. }
        ));
    }

    #[test]
    fn state_hash_one_missing_is_not_divergence() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
        assert!(matches!(
            r.state_hash_compare,
            StateHashCompare::OneMissing {
                a_present: true,
                b_present: false,
            },
        ));
    }

    #[test]
    fn tty_log_differences_do_not_diverge() {
        // tty_log is informational; differences must NOT flip
        // has_divergence().
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.tty_log = b"hello\n".to_vec();
        b.tty_log = b"hello\r\n".to_vec();
        let r = compare_observations(&a, &b);
        assert!(!r.has_divergence());
    }

    #[test]
    fn json_round_trip_preserves_structure() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(100),
        );
        let mut b_data = vec![0u8; 4];
        b_data[1] = 0xFF;
        b_data[2] = 0xFE;
        let b = obs(
            ObservedOutcome::Completed,
            vec![region("code", 0x10000, b_data)],
            "rpcs3",
            Some(200),
        );
        let r = compare_observations(&a, &b);
        let json = format_observation_compare_json(&r).unwrap();
        let parsed: ObservationCompareResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
        // Sanity: structured fields show up in the JSON.
        assert!(json.contains("\"a_runner\": \"cellgov\""));
        assert!(json.contains("\"b_runner\": \"rpcs3\""));
        assert!(json.contains("\"length\": 2"));
        assert!(json.contains("\"a_byte\": 0"));
        assert!(json.contains("\"b_byte\": 255"));
        assert!(json.contains("\"event_compare\""));
        assert!(json.contains("\"state_hash_compare\""));
    }

    #[test]
    fn json_is_pretty_printed_for_human_inspection() {
        let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = a.clone();
        let json = format_observation_compare_json(&compare_observations(&a, &b)).unwrap();
        assert!(
            json.contains('\n'),
            "pretty-printed JSON must include newlines"
        );
        assert!(
            json.contains("  \""),
            "pretty-printed JSON must indent fields"
        );
    }

    #[test]
    fn match_line_byte_format_is_pinned() {
        let a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x10000, vec![0u8; 8])],
            "cellgov",
            Some(7),
        );
        let b = a.clone();
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert_eq!(
            out,
            "MATCH outcome=Completed, 1 regions (8 bytes) identical, 0 events, no state hashes, steps Some(7) vs Some(7)\n"
        );
    }

    #[test]
    fn match_line_carries_event_count_when_events_are_present() {
        let mut a = obs(
            ObservedOutcome::Completed,
            vec![region("r", 0x10000, vec![0u8; 4])],
            "cellgov",
            Some(7),
        );
        a.events = vec![
            evt(ObservedEventKind::MailboxSend, 1, 0),
            evt(ObservedEventKind::DmaComplete, 2, 1),
            evt(ObservedEventKind::UnitWake, 1, 2),
        ];
        let b = a.clone();
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert!(out.contains("3 events"), "got: {out}");
    }

    #[test]
    fn match_line_labels_state_hashes_equal_when_both_present_and_matching() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        b.state_hashes = a.state_hashes;
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert!(out.contains("state hashes equal"), "got: {out}");
    }

    #[test]
    fn match_line_labels_state_hashes_one_sided_when_only_one_runner_has_them() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert!(out.contains("state hashes one-sided"), "got: {out}");
    }

    #[test]
    fn match_line_labels_state_hashes_cross_runner_when_both_present_and_differ() {
        let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
        let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
        a.state_hashes = Some(hashes(1, 2, 3));
        b.state_hashes = Some(hashes(9, 9, 9));
        let out = format_observation_compare_human(&compare_observations(&a, &b));
        assert!(
            out.contains("state hashes differ (cross-runner)"),
            "got: {out}"
        );
    }
}
