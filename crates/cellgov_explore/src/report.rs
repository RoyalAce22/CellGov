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
    match o {
        OutcomeClass::ScheduleStable => "schedule-stable",
        OutcomeClass::ScheduleSensitive => "schedule-sensitive",
        OutcomeClass::Inconclusive => "inconclusive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{ExplorationResult, OutcomeClass, ScheduleRecord};
    use cellgov_event::UnitId;

    fn sample_result() -> ExplorationResult {
        ExplorationResult {
            baseline_hash: 0xDEADBEEF,
            schedules: vec![ScheduleRecord {
                branch_step: 0,
                alternate_choice: UnitId::new(1),
                memory_hash: 0xCAFEBABE,
            }],
            outcome: OutcomeClass::ScheduleSensitive,
            total_branching_points: 2,
            bounds_hit: false,
            schedules_pruned: 1,
        }
    }

    #[test]
    fn human_report_contains_key_fields() {
        let text = format_human(&sample_result());
        assert!(text.contains("schedule-sensitive"));
        assert!(text.contains("0x00000000deadbeef"));
        assert!(text.contains("branching_points: 2"));
        assert!(text.contains("schedules_explored: 1"));
        assert!(text.contains("schedules_pruned: 1"));
        assert!(text.contains("DIVERGED"));
    }

    #[test]
    fn json_report_parses_correctly() {
        let json_str = format_json(&sample_result());
        let v: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");
        assert_eq!(v["outcome"], "schedule-sensitive");
        assert_eq!(v["branching_points"], 2);
        assert_eq!(v["schedules_explored"], 1);
        assert_eq!(v["schedules_pruned"], 1);
        assert_eq!(v["schedules"][0]["diverged"], true);
    }

    #[test]
    fn stable_result_no_diverged_tag() {
        let r = ExplorationResult {
            baseline_hash: 0xBEEF,
            schedules: vec![],
            outcome: OutcomeClass::ScheduleStable,
            total_branching_points: 1,
            bounds_hit: false,
            schedules_pruned: 1,
        };
        let text = format_human(&r);
        assert!(text.contains("schedule-stable"));
        assert!(!text.contains("DIVERGED"));
        assert!(!text.contains("schedules:\n"));
    }
}
