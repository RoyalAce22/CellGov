//! Return shape of `ExecutionUnit::run_until_yield`.
//!
//! Effects are collected separately via the `&mut Vec<Effect>`
//! parameter and are not carried on this struct. A step yielding
//! [`crate::YieldReason::Fault`] has all of its effects discarded by
//! the commit pipeline; this type only carries the data.

use crate::yield_reason::YieldReason;
use cellgov_effects::FaultKind;
use cellgov_time::Budget;

/// Per-step local diagnostics surfaced by an execution unit.
///
/// All fields are optional: synthetic and test-only step results may
/// omit them, and arch units populate what they can. `fault_regs` is
/// populated only on fault steps by units that support it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalDiagnostics {
    /// PC at the start of the step. `None` only from synthetic/test
    /// step results.
    pub pc: Option<u64>,
    /// Effective address that caused a memory fault, if applicable.
    pub faulting_ea: Option<u64>,
    /// Register snapshot captured at fault time.
    pub fault_regs: Option<FaultRegisterDump>,
}

/// Arch-neutral register snapshot captured at fault time for the CLI
/// to format without knowing PPU vs SPU specifics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultRegisterDump {
    /// GPR[0..31].
    pub gprs: [u64; 32],
    /// Link register (LR on PPC64).
    pub lr: u64,
    /// Count register (CTR on PPC64).
    pub ctr: u64,
    /// Condition register (CR on PPC64, packed 32-bit).
    pub cr: u32,
}

impl LocalDiagnostics {
    /// Empty diagnostics; equivalent to [`LocalDiagnostics::default`].
    #[inline]
    pub const fn empty() -> Self {
        Self {
            pc: None,
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics with only `pc` set.
    #[inline]
    pub const fn with_pc(pc: u64) -> Self {
        Self {
            pc: Some(pc),
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics with `pc` and `faulting_ea` set.
    #[inline]
    pub const fn with_pc_ea(pc: u64, ea: u64) -> Self {
        Self {
            pc: Some(pc),
            faulting_ea: Some(ea),
            fault_regs: None,
        }
    }
}

/// The value returned by a single `run_until_yield` call.
///
/// Fields are `pub` because the set is fixed and any future addition
/// is a contract change, not a drive-by edit.
///
/// **Invariant on `fault`:** `fault` is `Some` if and only if
/// `yield_reason == YieldReason::Fault`. The runtime relies on this
/// to route fault attribution. Violations are programming errors and
/// are checked by [`ExecutionStepResult::is_well_formed`]; the check
/// is exposed rather than enforced at construction so callers can
/// run it explicitly without paying for redundant checks at every
/// emission site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStepResult {
    /// Why the unit yielded.
    pub yield_reason: YieldReason,
    /// Budget the unit actually used; may be less than granted when
    /// the unit yielded early.
    pub consumed_budget: Budget,
    /// Per-step diagnostics for trace and assertion consumers.
    pub local_diagnostics: LocalDiagnostics,
    /// Fault data, present iff `yield_reason == YieldReason::Fault`.
    pub fault: Option<FaultKind>,
    /// Raw syscall arguments, present iff
    /// `yield_reason == YieldReason::Syscall`. Index 0 is the
    /// syscall number (from the arch's syscall-number register, e.g.
    /// GPR 11 on PPC64); indices 1..=8 are the argument registers
    /// (e.g. GPR 3..=10).
    pub syscall_args: Option<[u64; 9]>,
}

impl ExecutionStepResult {
    /// Whether the `fault`/`yield_reason` invariant holds. Callers
    /// should run this on every step result before processing it; a
    /// `false` return indicates a unit bug.
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

    #[test]
    fn local_diagnostics_default_and_empty_are_equal() {
        assert_eq!(LocalDiagnostics::default(), LocalDiagnostics::empty());
    }

    #[test]
    fn empty_step_constructs() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_budget: Budget::new(0),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(r.consumed_budget, Budget::new(0));
        assert!(r.fault.is_none());
        assert!(r.is_well_formed());
    }

    #[test]
    fn fault_step_carries_fault_kind() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_budget: Budget::new(7),
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
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        let c = r.clone();
        assert_eq!(r, c);
    }
}
