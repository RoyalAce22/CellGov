//! Every microtest boots once per interpreter and commit path, and each
//! path must run the guest as the default path does.
//!
//! - Traced runs compare the per-step state-hash streams. The state
//!   trace puts the runtime in its determinism-check mode, and that
//!   mode never takes the trivial-commit fast path. The suite compares
//!   the fast path on untraced runs, by observation.
//! - Every microtest runs on more than one unit, and the budget sets
//!   where the scheduler switches units. The suite therefore holds a
//!   run at a budget of one instruction per step to the microtest's
//!   documented payload.

use cellgov_compare::diverge::{diverge, DivergeReport};

use super::*;

/// The traced default run that each traced run must match.
const TRACED_DEFAULT: Run<'static> = Run {
    id: "neutral_trace_fused",
    args: &[],
    state_trace: true,
};

/// The traced shadow paths.
const TRACED_SHADOWS: [Run<'static>; 2] = [
    Run {
        id: "neutral_trace_quickened",
        args: &["--ppu-shadow", "quickened"],
        state_trace: true,
    },
    Run {
        id: "neutral_trace_off",
        args: &["--ppu-shadow", "off"],
        state_trace: true,
    },
];

/// The untraced default run that each untraced run must match.
const UNTRACED_DEFAULT: Run<'static> = Run {
    id: "neutral_fused",
    args: &[],
    state_trace: false,
};

/// The untraced shadow and commit paths.
const UNTRACED_PATHS: [Run<'static>; 3] = [
    Run {
        id: "neutral_quickened",
        args: &["--ppu-shadow", "quickened"],
        state_trace: false,
    },
    Run {
        id: "neutral_off",
        args: &["--ppu-shadow", "off"],
        state_trace: false,
    },
    Run {
        id: "neutral_no_trivial_commit",
        args: &["--no-trivial-commit"],
        state_trace: false,
    },
];

/// The default path at a budget of one instruction per step.
const BUDGET_1: Run<'static> = Run {
    id: "neutral_budget1",
    args: &["--budget", "1"],
    state_trace: false,
};

/// Where `other`'s per-step state hashes leave `base`'s, or `None` when
/// every stream matches record for record and holds at least one.
fn trace_divergence(base: &Boot, other: &Boot) -> Option<String> {
    match diverge(&read_trace(base), &read_trace(other)) {
        DivergeReport::Identical { count } if count > 0 => None,
        report => Some(format!("{report:?}")),
    }
}

fn read_trace(boot: &Boot) -> Vec<u8> {
    let path = boot.state_trace.as_ref().expect("a traced run");
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// How `other`'s observation differs from `base`'s.
fn observation_differences(base: &Observation, other: &Observation) -> Vec<String> {
    let mut out = Vec::new();
    if base.outcome != other.outcome {
        out.push(format!(
            "outcome {:?} then {:?}",
            base.outcome, other.outcome
        ));
    }
    if base.metadata.steps != other.metadata.steps {
        out.push(format!(
            "steps {:?} then {:?}",
            base.metadata.steps, other.metadata.steps
        ));
    }
    if base.tty_log != other.tty_log {
        out.push("TTY differs".to_string());
    }
    if base.memory_regions != other.memory_regions {
        out.push("observed memory differs".to_string());
    }
    out
}

/// Why `base`, a case's default run, is no evidence: two runs that
/// retired nothing and printed nothing compare equal on every path.
fn vacuity(case: &Case, base: &Observation) -> Option<String> {
    if base.metadata.steps.is_none_or(|s| s == 0) {
        return Some(format!(
            "the default run retired no steps ({:?}); the comparison would be vacuous",
            base.metadata.steps
        ));
    }
    if base.tty_log.is_empty() && known_defect(case.name).is_none() {
        return Some("the default run produced no TTY; the comparison would be vacuous".into());
    }
    None
}

/// Report every divergence in `failures`.
///
/// # Panics
///
/// Panics if `failures` is not empty, with one divergence per line.
fn assert_no_divergence(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} path divergences:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
}

#[test]
fn every_shadow_and_commit_path_runs_each_microtest_the_same() {
    let cases: Vec<&Case> = booted_cases().collect();
    assert!(!cases.is_empty(), "this build boots no microtest");
    let runs = [
        TRACED_DEFAULT,
        TRACED_SHADOWS[0],
        TRACED_SHADOWS[1],
        UNTRACED_DEFAULT,
        UNTRACED_PATHS[0],
        UNTRACED_PATHS[1],
        UNTRACED_PATHS[2],
    ];
    let mut failures: Vec<String> = Vec::new();
    for (case, boots) in cases.iter().copied().zip(boot_runs(&cases, &runs)) {
        let mut boots = boots.into_iter();
        let traced = boots.next().expect("the traced default run");
        for (run, boot) in TRACED_SHADOWS.iter().zip(boots.by_ref()) {
            if let Some(report) = trace_divergence(&traced, &boot) {
                failures.push(format!("{} {}: {report}", case.name, run.id));
            }
        }
        let untraced = boots.next().expect("the untraced default run");
        if let Some(vacuous) = vacuity(case, &untraced.observation) {
            failures.push(format!("{} {}: {vacuous}", case.name, UNTRACED_DEFAULT.id));
            continue;
        }
        for (run, boot) in UNTRACED_PATHS.iter().zip(boots) {
            for difference in observation_differences(&untraced.observation, &boot.observation) {
                failures.push(format!("{} {}: {difference}", case.name, run.id));
            }
        }
    }
    assert_no_divergence(&failures);
}

/// Microtests whose payload moves at a budget of one instruction per
/// step, each with the defect that moves it. Such a case must still
/// fail there: once it passes, the suite fails until the entry goes.
const BUDGET_1_DEFECTS: &[(&str, &str)] = &[
    ("spu_put_source_timing", PUT_SOURCE_READ_POINT),
    ("spu_put_source_progress", PUT_SOURCE_READ_POINT),
    ("spu_putb_source_progress", PUT_SOURCE_READ_POINT),
    ("spu_put_source_rate", PUT_SOURCE_READ_POINT),
];

/// The open defect behind every [`BUDGET_1_DEFECTS`] entry.
const PUT_SOURCE_READ_POINT: &str = "a long SPU put reads its local-store source at a point \
     the step budget moves, so stores the guest makes over the source while the put runs \
     reach less of the put at a budget of one instruction per step";

#[test]
fn every_microtest_reports_its_documented_payload_at_budget_1() {
    let cases: Vec<&Case> = booted_cases()
        .filter(|case| known_defect(case.name).is_none())
        .collect();
    assert!(!cases.is_empty(), "this build boots no microtest");
    let mut failures: Vec<String> = Vec::new();
    for (case, boots) in cases.iter().copied().zip(boot_runs(&cases, &[BUDGET_1])) {
        let observation = &boots[0].observation;
        let problems = match check_outcome(case, observation) {
            Some(bad) => vec![bad],
            None => check_payload(case, observation),
        };
        let defect = BUDGET_1_DEFECTS
            .iter()
            .find(|(name, _)| *name == case.name)
            .map(|(_, reason)| *reason);
        match defect {
            Some(reason) if problems.is_empty() => failures.push(format!(
                "{}: passes at budget 1, so the known defect ({reason}) no longer \
                 reproduces; drop its BUDGET_1_DEFECTS entry",
                case.name
            )),
            Some(reason) => eprintln!(
                "microtest {} at budget 1: known defect ({reason}): {}",
                case.name,
                problems.join("; ")
            ),
            None => {
                for problem in problems {
                    failures.push(format!("{} {}: {problem}", case.name, BUDGET_1.id));
                }
            }
        }
    }
    assert_no_divergence(&failures);
}
