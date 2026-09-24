//! The human and JSON renderings of a comparison result.

use super::types::{
    EventCompare, ObservationCompareResult, RegionPairOutcome, StateHashCompare, StepCompare,
};

/// Render the compare result to the stdout format the
/// `diff observations` CLI emits.
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
