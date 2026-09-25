//! The comparison walk: outcome, regions, events, state hashes and steps.

use crate::observation::{NamedMemoryRegion, Observation, ObservedEvent, ObservedHashes};

use super::types::{
    ByteDivergence, EventCompare, ObservationCompareResult, RegionCompareSummary,
    RegionPairOutcome, StateHashCompare, StepCompare,
};

/// Compare two observations.
///
/// Compared fields: `outcome`, `memory_regions` (region identity,
/// length, and byte content), `events` (strict equality by index),
/// `state_hashes` (same-runner only; see [`StateHashCompare`]), and
/// `metadata.steps`.
///
/// `tty_log` is informational and is not compared; cross-runner TTY
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
        a_identity: a.identity.clone(),
        b_identity: b.identity.clone(),
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
        (Some(ha), Some(hb)) if a_runner == b_runner && ha.scheme != hb.scheme => {
            StateHashCompare::SchemeMismatch {
                a: ha.scheme,
                b: hb.scheme,
            }
        }
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
