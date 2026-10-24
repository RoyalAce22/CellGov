//! The human and JSON renderings of a comparison result.

use super::types::{
    ByteDivergence, EventCompare, ObservationCompareResult, RegionCompareSummary,
    RegionPairOutcome, StateHashCompare, StepCompare,
};

/// Render the compare result to the stdout format the
/// `diff observations` CLI emits.
///
/// Fields are walked in fixed order (outcome -> regions -> events ->
/// state hashes -> steps); every divergent section emits its own
/// DIVERGE line. The MATCH summary line at the end appears only when
/// `has_divergence()` is false and the result holds no scheme mismatch.
/// The caller is responsible for the stderr WARN / NOTE lines around
/// vacuous comparisons and cross-runner notes; see
/// [`ObservationCompareResult::is_vacuous`] and
/// [`ObservationCompareResult::cross_runner_step_note`].
pub fn format_observation_compare_human(result: &ObservationCompareResult) -> String {
    let mut out = String::new();
    push_outcome(&mut out, result);
    push_regions(&mut out, &result.region_compare);
    push_events(&mut out, &result.event_compare);
    push_state_hashes(&mut out, result);
    push_scheme(&mut out, result);
    push_steps(&mut out, result);
    push_match_line(&mut out, result);
    out
}

fn push_outcome(out: &mut String, result: &ObservationCompareResult) {
    if !result.outcome_match {
        out.push_str(&format!(
            "DIVERGE outcome: {}={:?} vs {}={:?}\n",
            result.a_runner, result.a_outcome, result.b_runner, result.b_outcome,
        ));
    }
}

fn push_regions(out: &mut String, regions: &RegionCompareSummary) {
    if regions.is_count_mismatch() {
        out.push_str(&format!(
            "DIVERGE region count: {} vs {}\n",
            regions.a_count, regions.b_count
        ));
    }
    for pair in &regions.pairs {
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
                    push_byte_divergence(out, name, *addr, div);
                }
            }
        }
    }
}

/// One DIVERGE line for a byte run in region `name` at guest `addr`.
fn push_byte_divergence(out: &mut String, name: &str, addr: u64, div: &ByteDivergence) {
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
        return;
    }
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

fn push_events(out: &mut String, events: &EventCompare) {
    match events {
        EventCompare::Equal { .. } => {}
        EventCompare::LengthMismatch { a, b } => {
            out.push_str(&format!("DIVERGE event count: {a} vs {b}\n"));
        }
        EventCompare::FirstIndexDiffers { index, a, b } => {
            out.push_str(&format!("DIVERGE event at index {index}: {a:?} vs {b:?}\n"));
        }
    }
}

fn push_state_hashes(out: &mut String, result: &ObservationCompareResult) {
    if let StateHashCompare::SameRunnerMismatch { a, b } = &result.state_hash_compare {
        out.push_str(&format!(
            "DIVERGE state hashes within runner '{}': {a:?} vs {b:?}\n",
            result.a_runner,
        ));
    }
}

fn push_scheme(out: &mut String, result: &ObservationCompareResult) {
    if let Some((a, b)) = result.scheme_mismatch() {
        out.push_str(&format!(
            "SCHEME_MISMATCH state hashes: scheme 0x{a:016x} vs 0x{b:016x}; the hashes were not compared\n",
        ));
    }
}

fn push_steps(out: &mut String, result: &ObservationCompareResult) {
    if let StepCompare::SameRunnerMismatch { a, b } = result.step_compare {
        out.push_str(&format!(
            "DIVERGE step count: {a} vs {b} within runner '{}' (byte-equal state reached via different work -- a determinism failure)\n",
            result.a_runner,
        ));
    }
}

/// The MATCH summary, pushed only when no section diverged and the
/// schemes agree.
fn push_match_line(out: &mut String, result: &ObservationCompareResult) {
    if result.has_divergence() || result.scheme_mismatch().is_some() {
        return;
    }
    let (sa, sb) = steps_pair(&result.step_compare);
    let event_count = match &result.event_compare {
        EventCompare::Equal { count } => *count,
        EventCompare::LengthMismatch { .. } | EventCompare::FirstIndexDiffers { .. } => {
            unreachable!("has_divergence() filters out event divergence")
        }
    };
    out.push_str(&format!(
        "MATCH outcome={:?}, {} regions ({} bytes) identical, {} events, {}, steps {:?} vs {:?}\n",
        result.a_outcome,
        result.region_compare.matched_regions(),
        result.region_compare.matched_bytes(),
        event_count,
        hash_label(&result.state_hash_compare),
        sa,
        sb,
    ));
}

/// The MATCH line's words for the state-hash comparison.
fn hash_label(hashes: &StateHashCompare) -> &'static str {
    match hashes {
        StateHashCompare::Equal => "state hashes equal",
        StateHashCompare::NoHashInfo => "no state hashes",
        StateHashCompare::OneMissing { .. } => "state hashes one-sided",
        StateHashCompare::CrossRunnerNote { .. } => "state hashes differ (cross-runner)",
        // The MATCH guard lets neither variant reach this match.
        StateHashCompare::SameRunnerMismatch { .. } => "state hashes differ",
        StateHashCompare::SchemeMismatch { .. } => "state hashes of two schemes",
    }
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
