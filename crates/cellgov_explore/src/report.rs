//! ASCII and JSON formatters for [`ExplorationResult`].

use crate::classify::{ExplorationResult, OutcomeClass};

/// Format an exploration result as a human-readable ASCII report.
pub fn format_human(result: &ExplorationResult) -> String {
    let mut out = String::new();
    out.push_str(&format!("outcome: {}\n", outcome_label(result.outcome)));
    out.push_str(&format!("baseline_hash: 0x{:016x}\n", result.baseline_hash));
    out.push_str(&format!(
        "branching_points: {}\n",
        result.total_branching_points
    ));
    out.push_str(&format!("schedules_explored: {}\n", result.schedules.len()));
    out.push_str(&format!("schedules_pruned: {}\n", result.schedules_pruned));
    out.push_str(&format!("bounds_hit: {}\n", result.bounds_hit));

    if !result.schedules.is_empty() {
        out.push_str("schedules:\n");
        for (i, s) in result.schedules.iter().enumerate() {
            let diverged = if s.memory_hash != result.baseline_hash {
                " DIVERGED"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {}: step={} alt_unit={} hash=0x{:016x}{}\n",
                i,
                s.branch_step,
                s.alternate_choice.raw(),
                s.memory_hash,
                diverged,
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
                "diverged": s.memory_hash != result.baseline_hash,
            })
        })
        .collect();

    let json = serde_json::json!({
        "outcome": outcome_label(result.outcome),
        "baseline_hash": format!("0x{:016x}", result.baseline_hash),
        "branching_points": result.total_branching_points,
        "schedules_explored": result.schedules.len(),
        "schedules_pruned": result.schedules_pruned,
        "bounds_hit": result.bounds_hit,
        "schedules": schedules,
    });
    serde_json::to_string_pretty(&json).expect("JSON serialization cannot fail")
}

fn outcome_label(o: OutcomeClass) -> &'static str {
    <&'static str>::from(&o)
}

#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod tests;
