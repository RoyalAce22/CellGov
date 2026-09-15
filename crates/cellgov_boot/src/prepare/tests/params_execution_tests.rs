//! Which execution-group field each process parameter comes from, and
//! which value the budget refusals and warnings name.

use std::cell::RefCell;

use cellgov_core::{default_budget_for_mode, RuntimeMode};
use cellgov_time::Budget;

use super::{resolve_boot_params, BootParams, ParamsError};
use crate::prepare::ExecutionOptions;
use crate::{BootError, BootSink};

/// Keeps the warning lines so a test can name which one fired.
#[derive(Default)]
struct WarnLog(RefCell<Vec<String>>);

impl BootSink for WarnLog {
    fn note(&self, _line: &str) {}
    fn warn(&self, line: &str) {
        self.0.borrow_mut().push(line.to_string());
    }
    fn guest_text(&self, _text: &str) {}
}

fn execution(max_steps: usize, budget: Option<u64>) -> ExecutionOptions<'static> {
    ExecutionOptions {
        runtime_max_steps: max_steps,
        budget_override: budget.map(Budget::new),
        strict_reserved: false,
        capture_state_trace: false,
        guest_args: &[],
        patch_bytes: &[],
    }
}

/// An empty image carries no `sys_proc_param` and no TLS segment, so
/// everything the result holds comes from the execution group.
fn resolve(opts: ExecutionOptions<'_>) -> (Result<BootParams, BootError>, Vec<String>) {
    let log = WarnLog::default();
    let result = resolve_boot_params(&opts, &log, &[]);
    let warnings = log.0.borrow().clone();
    (result, warnings)
}

#[test]
fn a_budget_override_replaces_the_mode_default() {
    let (result, warnings) = resolve(execution(40, Some(4)));
    let params = result.expect("40 instructions is ten whole budgets");
    assert_eq!(params.step_budget, Budget::new(4));
    assert_eq!(params.adjusted_max_steps, 10);
    assert_eq!(params.mode, RuntimeMode::FaultDriven);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn an_absent_override_takes_the_mode_default() {
    let default = default_budget_for_mode(RuntimeMode::FaultDriven);
    let (result, warnings) = resolve(execution(default.raw() as usize, None));
    let params = result.expect("one whole budget");
    assert_eq!(params.step_budget, default);
    assert_eq!(params.adjusted_max_steps, 1);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_zero_budget_is_raised_to_one_and_witnessed() {
    let (result, warnings) = resolve(execution(8, Some(0)));
    let params = result.expect("a raised budget of 1 divides every cap");
    assert_eq!(params.step_budget, Budget::new(1));
    assert_eq!(params.adjusted_max_steps, 8);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("budget 0 retires no work")),
        "{warnings:?}"
    );
}

fn refusal(result: Result<BootParams, BootError>) -> ParamsError {
    match result {
        Err(BootError::Params(e)) => e,
        Err(other) => panic!("wrong refusal: {other}"),
        Ok(_) => panic!("a cap below one budget must be refused"),
    }
}

#[test]
fn a_step_cap_below_one_budget_is_refused_naming_both() {
    let (result, _) = resolve(execution(15, Some(16)));
    assert_eq!(
        refusal(result),
        ParamsError::MaxStepsBelowBudget {
            max_steps: 15,
            budget: 16
        }
    );
}

#[test]
fn a_refusal_after_a_raised_budget_names_the_raised_value() {
    // The cap is measured against the budget the run will actually
    // use, so a raise from 0 has to reach the refusal too.
    let (result, _) = resolve(execution(0, Some(0)));
    assert_eq!(
        refusal(result),
        ParamsError::MaxStepsBelowBudget {
            max_steps: 0,
            budget: 1
        }
    );
}

#[test]
fn a_cap_that_is_not_a_multiple_of_the_budget_reports_the_reachable_one() {
    let (result, warnings) = resolve(execution(100, Some(16)));
    let params = result.expect("100 instructions covers six whole budgets");
    assert_eq!(params.adjusted_max_steps, 6);
    let warning = warnings
        .iter()
        .find(|w| w.contains("is not a multiple of budget"))
        .unwrap_or_else(|| panic!("no round-down warning in {warnings:?}"));
    assert!(warning.contains("max_steps=100"), "{warning}");
    assert!(warning.contains("budget=16"), "{warning}");
    assert!(warning.contains("96 retired instructions"), "{warning}");
}

#[test]
fn capture_state_trace_selects_the_determinism_check_mode() {
    let mut opts = execution(0, None);
    opts.capture_state_trace = true;
    let default = default_budget_for_mode(RuntimeMode::DeterminismCheck);
    opts.runtime_max_steps = default.raw() as usize;
    let (result, _) = resolve(opts);
    let params = result.expect("one whole budget");
    assert_eq!(params.mode, RuntimeMode::DeterminismCheck);
    assert_eq!(params.step_budget, default);
}
