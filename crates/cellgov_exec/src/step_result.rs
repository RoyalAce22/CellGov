//! `ExecutionStepResult` -- the value an execution unit returns from
//! `run_until_yield`.
//!
//! A step result carries five fields: the yield reason, the budget the
//! unit actually consumed, the list of effects it emitted (in stable
//! emission order), per-step local diagnostics, and optional fault data.
//!
//! `emitted_effects` ordering is part of the determinism contract. The
//! runtime must never reorder effects within a single step: validation,
//! conflict diagnostics, fault attribution, and trace reconstruction all
//! depend on stable intra-step ordering even though commit batches are
//! atomic from the standpoint of guest visibility.
//!
//! The fault rule is enforced at the runtime layer, not
//! here: a step that yields with [`crate::YieldReason::Fault`] has all
//! of its effects discarded. This type just carries the data; the
//! discarding is the commit pipeline's job.

use crate::yield_reason::YieldReason;
use cellgov_effects::{Effect, FaultKind};
use cellgov_time::Budget;

/// Per-step local diagnostics surfaced by an execution unit.
///
/// Currently an empty extension-point struct. `local_diagnostics` is a
/// required field on the step result, but no specific contents are
/// defined yet. Inventing fields ahead of a real consumer would just
/// create churn when the eventual consumers (trace records, scenario
/// assertions, scheduler heuristics) actually need specific data.
/// Adding non-breaking fields later is the same shape as any other Rust
/// struct: append, derive `Default`, done.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalDiagnostics {}

impl LocalDiagnostics {
    /// An empty diagnostics record. Equivalent to
    /// [`LocalDiagnostics::default`]; spelled out as a constructor so
    /// call sites can be explicit about producing one.
    #[inline]
    pub const fn empty() -> Self {
        Self {}
    }
}

/// The result of a single `run_until_yield` call.
///
/// Construction is intentionally explicit (the struct fields are `pub`)
/// because the field set is fixed and any future addition is a
/// deliberate change to the runtime contract, not a drive-by edit.
/// There is no `new(...)` constructor to maintain -- units build the
/// struct directly.
///
/// **Invariant on `fault`:** `fault` is `Some` if and only if
/// `yield_reason == YieldReason::Fault`. The runtime relies on this to
/// route fault attribution; constructing a result with a `Fault` reason
/// and no fault data, or with non-`Fault` reason and `Some` fault data,
/// is a programming error and is checked by [`ExecutionStepResult::is_well_formed`].
/// The check is exposed rather than enforced at construction so tests
/// and the future commit pipeline can run it explicitly without paying
/// for redundant checks at every emission site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStepResult {
    /// Why the unit yielded control back to the runtime.
    pub yield_reason: YieldReason,
    /// How much budget the unit actually used during this step.
    /// May be less than the granted budget if the unit yielded early.
    pub consumed_budget: Budget,
    /// Effects emitted during this step, in the order the unit emitted
    /// them. The runtime must preserve this order end-to-end.
    pub emitted_effects: Vec<Effect>,
    /// Per-step diagnostics for trace and assertion consumers.
    pub local_diagnostics: LocalDiagnostics,
    /// Fault data, present iff `yield_reason == YieldReason::Fault`.
    pub fault: Option<FaultKind>,
    /// Raw syscall arguments, present iff `yield_reason == YieldReason::Syscall`.
    /// Index 0 is the syscall number (from the architecture's syscall-number
    /// register, e.g. GPR 11 on PPC64). Indices 1..=8 are the argument
    /// registers (e.g. GPR 3..=10). The runtime reads these to classify
    /// the request and dispatch through the LV2 host.
    pub syscall_args: Option<[u64; 9]>,
}

impl ExecutionStepResult {
    /// Whether this result satisfies the `fault`/`yield_reason`
    /// invariant. The runtime should call this on every step result
    /// before processing it; failure indicates a unit bug.
    #[inline]
    pub fn is_well_formed(&self) -> bool {
        match (self.yield_reason, &self.fault) {
            (YieldReason::Fault, Some(_)) => true,
            (YieldReason::Fault, None) => false,
            (_, Some(_)) => false,
            (_, None) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_event::UnitId;

    fn marker(id: u32) -> Effect {
        Effect::TraceMarker {
            marker: id,
            source: UnitId::new(0),
        }
    }

    #[test]
    fn local_diagnostics_default_and_empty_are_equal() {
        assert_eq!(LocalDiagnostics::default(), LocalDiagnostics::empty());
    }

    #[test]
    fn empty_step_constructs() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_budget: Budget::new(0),
            emitted_effects: vec![],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(r.consumed_budget, Budget::new(0));
        assert!(r.emitted_effects.is_empty());
        assert!(r.fault.is_none());
        assert!(r.is_well_formed());
    }

    #[test]
    fn step_with_effects_preserves_order() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::MailboxAccess,
            consumed_budget: Budget::new(50),
            emitted_effects: vec![marker(1), marker(2), marker(3)],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        // Direct slice equality preserves both content and order
        // without needing pattern destructuring.
        assert_eq!(
            r.emitted_effects.as_slice(),
            &[marker(1), marker(2), marker(3)]
        );
        assert!(r.is_well_formed());
    }

    #[test]
    fn fault_step_carries_fault_kind() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_budget: Budget::new(7),
            emitted_effects: vec![marker(99)],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: Some(FaultKind::Guest(0xbad)),
            syscall_args: None,
        };
        assert!(r.is_well_formed());
        assert_eq!(r.fault, Some(FaultKind::Guest(0xbad)));
    }

    #[test]
    fn fault_without_data_is_malformed() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_budget: Budget::new(0),
            emitted_effects: vec![],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert!(!r.is_well_formed());
    }

    #[test]
    fn non_fault_with_fault_data_is_malformed() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_budget: Budget::new(0),
            emitted_effects: vec![],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: Some(FaultKind::Validation),
            syscall_args: None,
        };
        assert!(!r.is_well_formed());
    }

    #[test]
    fn finished_step_is_well_formed() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_budget: Budget::new(100),
            emitted_effects: vec![],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert!(r.is_well_formed());
    }

    #[test]
    fn clone_preserves_fields() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::DmaWait,
            consumed_budget: Budget::new(13),
            emitted_effects: vec![marker(42)],
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        let c = r.clone();
        assert_eq!(r, c);
    }
}
