//! ASCII and JSON formatters for [`ExplorationResult`].

use crate::classify::{ExplorationResult, OutcomeClass, ScheduleRecord};

/// Format an exploration result as a human-readable ASCII report.
pub fn format_human(result: &ExplorationResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("outcome: {}\n", outcome_label(result.outcome)));
    out.push_str(&format!("baseline_hash: 0x{:016x}\n", result.baseline_hash));
    out.push_str(&format!("baseline_steps: {}\n", result.baseline_steps));
    out.push_str(&format!(
        "baseline_stop: {} ({})\n",
        result.baseline_stop,
        result.baseline_stop.class().label(),
    ));
    out.push_str(&format!(
        "branching_points: {}\n",
        result.total_branching_points
    ));
    out.push_str(&format!("schedules_explored: {}\n", result.schedules.len()));
    out.push_str(&format!(
        "classes_explored: {}\n",
        match (result.classes_explored, result.reversals_dropped) {
            (Some(n), 0) => n.to_string(),
            // A count beside a drop breaks the rule the field states,
            // and the JSON below prints the drop whatever the count
            // says, so this names the pair rather than hiding one half.
            (Some(n), dropped) => format!("{n} (contradicted by {dropped} reversal(s) dropped)"),
            (None, 0) => "not covered".to_string(),
            (None, dropped) => format!("not covered ({dropped} reversal(s) dropped)"),
        }
    ));
    out.push_str(&format!("schedules_pruned: {}\n", result.schedules_pruned));
    out.push_str(&format!(
        "schedules_truncated: {}\n",
        result.schedules_truncated
    ));
    out.push_str(&format!(
        "schedules_refused: {}\n",
        result.schedules_refused
    ));
    out.push_str(&format!("bounds_hit: {}\n", result.bounds_hit));

    if !result.schedules.is_empty() {
        out.push_str("schedules:\n");
        for (i, s) in result.schedules.iter().enumerate() {
            // A prefix hash is never labelled DIVERGED: it differs from
            // a finished baseline whether or not the workload is
            // schedule-sensitive.
            let tag = match truncated_by(s) {
                Some(by) => format!(" TRUNCATED({by})"),
                None if s.memory_hash != result.baseline_hash => " DIVERGED".to_string(),
                None => String::new(),
            };
            // Every finished replay reports a stall, so a row omits it.
            // The class label goes with the reason because the
            // runtime's own step cap prints as a step refusal.
            let stop = if s.stop.is_truncated() {
                format!(" stop={} ({})", s.stop, s.stop.class().label())
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  {}: step={} alt_unit={} hash=0x{:016x}{}{}\n",
                i,
                s.branch_step,
                s.alternate_choice.raw(),
                s.memory_hash,
                tag,
                stop,
            ));
        }
    }
    out
}

/// Format an exploration result as a JSON string.
pub fn format_json(result: &ExplorationResult) -> String {
    let schedules: Vec<serde_json::Value> = result
        .schedules
        .iter()
        .map(|s| {
            serde_json::json!({
                "branch_step": s.branch_step,
                "alternate_choice": s.alternate_choice.raw(),
                "memory_hash": format!("0x{:016x}", s.memory_hash),
                "diverged": !s.truncated && s.memory_hash != result.baseline_hash,
                "truncated": s.truncated,
                "truncated_by": truncated_by(s),
                "stop": s.stop.to_string(),
                "stop_class": s.stop.class().label(),
            })
        })
        .collect();

    let json = serde_json::json!({
        "outcome": outcome_label(result.outcome),
        "baseline_hash": format!("0x{:016x}", result.baseline_hash),
        "baseline_steps": result.baseline_steps,
        "baseline_stop": result.baseline_stop.to_string(),
        "baseline_stop_class": result.baseline_stop.class().label(),
        "branching_points": result.total_branching_points,
        "schedules_explored": result.schedules.len(),
        "classes_explored": result.classes_explored,
        "reversals_dropped": result.reversals_dropped,
        "schedules_pruned": result.schedules_pruned,
        "schedules_truncated": result.schedules_truncated,
        "schedules_refused": result.schedules_refused,
        "bounds_hit": result.bounds_hit,
        "schedules": schedules,
    });
    serde_json::to_string_pretty(&json).expect("JSON serialization cannot fail")
}

fn outcome_label(o: OutcomeClass) -> &'static str {
    <&'static str>::from(&o)
}

/// What withdrew a record's hash -- its own replay, or the baseline the
/// exploration measured it against -- and `None` when nothing did.
fn truncated_by(record: &ScheduleRecord) -> Option<&'static str> {
    match (record.truncated, record.stop.is_truncated()) {
        (true, true) => Some("replay"),
        (true, false) => Some("baseline"),
        (false, _) => None,
    }
}

#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod tests;
