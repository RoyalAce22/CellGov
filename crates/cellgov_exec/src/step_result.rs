//! Return shape of `ExecutionUnit::run_until_yield`.
//!
//! Effects are collected separately via the `&mut Vec<Effect>`
//! parameter and are not carried on this struct. A step yielding
//! [`crate::YieldReason::Fault`] has all of its effects discarded by
//! the commit pipeline; this type only carries the data.

use crate::yield_reason::YieldReason;
use cellgov_effects::FaultKind;
use cellgov_time::InstructionCost;

/// Per-step local diagnostics surfaced by an execution unit.
///
/// All fields are optional: synthetic and test-only step results may
/// omit them, and arch units populate what they can. Consumers must
/// tolerate `None` on every field -- a synthetic fault step that
/// omits `fault_regs`, or a syscall step from a fake unit that omits
/// `lr`, is legal and renders as `<unknown>` in CLI diagnostics.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalDiagnostics {
    /// PC at the start of the step. `None` only from synthetic/test
    /// step results.
    pub pc: Option<u64>,
    /// Caller return address at yield time. Populated by arch units
    /// on `YieldReason::Syscall` yields so HLE / LV2 dispatch can
    /// attribute the call to a specific guest call site without a
    /// post-hoc downcast through the registry. `None` on all other
    /// yield reasons and from synthetic / test step results.
    pub lr: Option<u64>,
    /// LEV field of the `sc` instruction (PPC64 Book III §2.3.1).
    /// LEV=0 is the standard kernel-syscall form; LEV=1 is the
    /// hypervisor hcall form (CBE Handbook §11.1); LEV>1 is reserved.
    /// Populated by arch units on `YieldReason::Syscall`. The
    /// runtime classifier uses this to reject hypercalls before
    /// they reach the LV2 dispatch path. `None` on non-syscall
    /// yields and from synthetic / test step results.
    pub syscall_lev: Option<u8>,
    /// Effective address that caused a memory fault, if applicable.
    pub faulting_ea: Option<u64>,
    /// Register snapshot captured at fault time. Optional even on
    /// fault steps: synthetic units that fault before populating a
    /// register file emit `None` here, and CLI fault formatters
    /// fall through to the no-registers path. Real arch units that
    /// support the dump (PPU, SPU) always populate it on fault.
    pub fault_regs: Option<FaultRegisterDump>,
}

/// Arch-neutral register snapshot captured at fault time for the CLI
/// to format without knowing PPU vs SPU specifics.
///
/// Field set matches the registers folded into `PpuStateHash`'s
/// FNV-1a fingerprint (GPR + LR + CTR + XER + CR), so a CLI fault
/// formatter and a divergence trace agree on the same set of state
/// at the same step. SPU dumps populate `xer = 0` -- the SPU has no
/// XER analogue, and an arch-neutral struct that drops XER on PPU
/// dumps would lose state the divergence trace already hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultRegisterDump {
    /// GPR[0..31].
    pub gprs: [u64; 32],
    /// Link register (LR on PPC64).
    pub lr: u64,
    /// Count register (CTR on PPC64).
    pub ctr: u64,
    /// Fixed-Point Exception Register. PowerPC carries XER's
    /// SO/OV/CA bits across instruction boundaries; including it
    /// matches the field set hashed in `PpuStateHash` and lets the
    /// CLI format the same fingerprint a divergence trace would
    /// produce. SPU dumps populate this with zero.
    pub xer: u64,
    /// Condition register (CR on PPC64, packed 32-bit).
    pub cr: u32,
}

impl FaultRegisterDump {
    /// All-zero dump. For test fixtures only -- a real fault step's
    /// dump should be populated by the unit from its committed
    /// register file. `#[cfg(test)]` keeps this out of the public
    /// API so production code cannot accidentally construct a dump
    /// indistinguishable from a unit that forgot to populate one.
    #[cfg(test)]
    pub const fn zeroed() -> Self {
        Self {
            gprs: [0; 32],
            lr: 0,
            ctr: 0,
            xer: 0,
            cr: 0,
        }
    }
}

impl LocalDiagnostics {
    /// Empty diagnostics; equivalent to [`LocalDiagnostics::default`].
    #[inline]
    pub const fn empty() -> Self {
        Self {
            pc: None,
            lr: None,
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics with only `pc` set.
    #[inline]
    pub const fn with_pc(pc: u64) -> Self {
        Self {
            pc: Some(pc),
            lr: None,
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics with `pc` and `faulting_ea` set.
    #[inline]
    pub const fn with_pc_ea(pc: u64, ea: u64) -> Self {
        Self {
            pc: Some(pc),
            lr: None,
            syscall_lev: None,
            faulting_ea: Some(ea),
            fault_regs: None,
        }
    }

    /// Diagnostics with `pc` and `lr` set; for arch units returning a
    /// syscall yield where the calling convention has the return
    /// address staged in LR.
    #[inline]
    pub const fn with_pc_lr(pc: u64, lr: u64) -> Self {
        Self {
            pc: Some(pc),
            lr: Some(lr),
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics for a syscall step: `pc`, `lr`, and the LEV field
    /// of the `sc` instruction. PPU and SPU units populate this on
    /// every syscall yield so the runtime classifier can reject
    /// hypercalls (LEV >= 1) before they reach the LV2 dispatch
    /// path.
    #[inline]
    pub const fn with_pc_lr_syscall_lev(pc: u64, lr: u64, syscall_lev: u8) -> Self {
        Self {
            pc: Some(pc),
            lr: Some(lr),
            syscall_lev: Some(syscall_lev),
            faulting_ea: None,
            fault_regs: None,
        }
    }

    /// Diagnostics for a fault step: `pc`, `faulting_ea`, and the
    /// register dump in one constructor. Centralising the shape
    /// keeps fault sites from accidentally calling `with_pc_ea` and
    /// emitting a fault-shaped diagnostic with no register dump.
    #[inline]
    pub const fn with_fault(pc: u64, ea: u64, regs: FaultRegisterDump) -> Self {
        Self {
            pc: Some(pc),
            lr: None,
            syscall_lev: None,
            faulting_ea: Some(ea),
            fault_regs: Some(regs),
        }
    }
}

/// The value returned by a single `run_until_yield` call.
///
/// Fields are `pub` because the set is fixed and any future addition
/// is a contract change, not a drive-by edit.
///
/// **Invariant on `fault`:** `fault` is `Some` if and only if
/// `yield_reason == YieldReason::Fault`.
///
/// **Invariant on `syscall_args`:** `syscall_args` is `Some` if and
/// only if `yield_reason == YieldReason::Syscall`.
///
/// The runtime relies on both to route fault attribution and HLE /
/// LV2 dispatch. Violations are programming errors and are checked
/// by [`ExecutionStepResult::is_well_formed`]; the check is exposed
/// rather than enforced at construction so callers can run it
/// explicitly without paying for redundant checks at every emission
/// site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStepResult {
    /// Why the unit yielded.
    pub yield_reason: YieldReason,
    /// Work the unit actually retired; may be less than granted when
    /// the unit yielded early. Bridges to [`cellgov_time::GuestTicks`]
    /// via `From<InstructionCost>` at step 8 of the commit pipeline.
    ///
    /// `InstructionCost::ZERO` is legal only on `YieldReason::Fault`
    /// (the unit faulted before retiring its first instruction).
    /// Other yield reasons with zero cost are forward-progress bugs:
    /// the scheduler advances guest time by `consumed_cost` at step
    /// 8, so a non-fault zero-cost yield emitted in a loop deadlocks
    /// the schedule. The runtime can `debug_assert!` this at the
    /// step-loop boundary; this struct does not enforce it because
    /// the legal-cases set is small and emitter-defined.
    pub consumed_cost: InstructionCost,
    /// Per-step diagnostics for trace and assertion consumers.
    pub local_diagnostics: LocalDiagnostics,
    /// Fault data, present iff `yield_reason == YieldReason::Fault`.
    pub fault: Option<FaultKind>,
    /// Raw syscall arguments, present iff
    /// `yield_reason == YieldReason::Syscall`. Index 0 carries the
    /// syscall number from the arch's syscall-number register
    /// (GPR 11 on PPC64 LV2; channel value on SPU), and indices
    /// 1..=8 carry the argument registers in arch-defined order
    /// (GPR 3..=10 on PPC64 LV2; index 1 overlaps the conventional
    /// PPC return register r3 -- the response goes back through the
    /// runtime's syscall-response table, not by mutating this
    /// array). The mapping is the responsibility of the emitting
    /// `ExecutionUnit`; this struct is a transport only and does
    /// not enforce a particular convention.
    pub syscall_args: Option<[u64; 9]>,
}

impl ExecutionStepResult {
    /// Whether both struct-level invariants hold:
    /// - `fault` is `Some` iff `yield_reason == Fault`.
    /// - `syscall_args` is `Some` iff `yield_reason == Syscall`.
    ///
    /// Callers should run this on every step result before processing
    /// it; a `false` return indicates a unit bug.
    #[inline]
    pub fn is_well_formed(&self) -> bool {
        let fault_ok = match (self.yield_reason, &self.fault) {
            (YieldReason::Fault, Some(_)) => true,
            (YieldReason::Fault, None) => false,
            (_, Some(_)) => false,
            (_, None) => true,
        };
        let syscall_ok = match (self.yield_reason, &self.syscall_args) {
            (YieldReason::Syscall, Some(_)) => true,
            (YieldReason::Syscall, None) => false,
            (_, Some(_)) => false,
            (_, None) => true,
        };
        fault_ok && syscall_ok
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
    fn with_pc_lr_populates_both_fields() {
        let d = LocalDiagnostics::with_pc_lr(0x1234, 0xABCD);
        assert_eq!(d.pc, Some(0x1234));
        assert_eq!(d.lr, Some(0xABCD));
        assert!(d.faulting_ea.is_none());
        assert!(d.fault_regs.is_none());
    }

    #[test]
    fn with_pc_does_not_set_lr() {
        // Arch units that cannot supply LR (synthetic / test) leave
        // it None on with_pc; downstream HLE dispatch reports
        // "<unknown>" rather than fabricating an address.
        let d = LocalDiagnostics::with_pc(0x1234);
        assert_eq!(d.pc, Some(0x1234));
        assert!(d.lr.is_none());
    }

    #[test]
    fn with_pc_ea_populates_pc_and_ea_only() {
        let d = LocalDiagnostics::with_pc_ea(0x1000_0000, 0x4000_0008);
        assert_eq!(d.pc, Some(0x1000_0000));
        assert_eq!(d.faulting_ea, Some(0x4000_0008));
        assert!(d.lr.is_none());
        assert!(d.fault_regs.is_none());
    }

    #[test]
    fn with_fault_populates_pc_ea_and_regs() {
        let regs = FaultRegisterDump::zeroed();
        let d = LocalDiagnostics::with_fault(0x1000_0000, 0x4000_0008, regs.clone());
        assert_eq!(d.pc, Some(0x1000_0000));
        assert_eq!(d.faulting_ea, Some(0x4000_0008));
        assert_eq!(d.fault_regs.as_ref(), Some(&regs));
        assert!(d.lr.is_none());
    }

    #[test]
    fn empty_step_constructs() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::ZERO,
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(r.consumed_cost, InstructionCost::ZERO);
        assert!(r.fault.is_none());
        assert!(r.is_well_formed());
    }

    #[test]
    fn fault_step_carries_fault_kind() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_cost: InstructionCost::new(7),
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
            consumed_cost: InstructionCost::ZERO,
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
            consumed_cost: InstructionCost::ZERO,
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
            consumed_cost: InstructionCost::new(100),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        };
        assert!(r.is_well_formed());
    }

    #[test]
    fn syscall_step_with_args_is_well_formed() {
        let mut args = [0u64; 9];
        args[0] = 144; // sys_time_get_timezone
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::with_pc_lr(0x1_0000, 0x1_0004),
            fault: None,
            syscall_args: Some(args),
        };
        assert!(r.is_well_formed());
    }

    #[test]
    fn syscall_without_args_is_malformed() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::ZERO,
            local_diagnostics: LocalDiagnostics::with_pc(0x1_0000),
            fault: None,
            syscall_args: None,
        };
        assert!(!r.is_well_formed());
    }

    #[test]
    fn non_syscall_with_args_is_malformed() {
        let r = ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(256),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: Some([0; 9]),
        };
        assert!(!r.is_well_formed());
    }

    #[test]
    fn clone_preserves_heavy_fields() {
        // Pin clone-soundness on the heavy fields specifically: the
        // 32-element GPR array and the syscall-args array. A future
        // representation change to either would break here rather
        // than in some end-to-end test.
        let mut args = [0u64; 9];
        args[0] = 41; // sys_ppu_thread_exit
        args[1] = 0xDEAD_BEEF;
        let regs = FaultRegisterDump {
            gprs: core::array::from_fn(|i| i as u64),
            lr: 0x1000_0004,
            ctr: 0x2000_0000,
            xer: 0x8000_0001,
            cr: 0xAAAA_5555,
        };
        let r = ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: InstructionCost::new(7),
            local_diagnostics: LocalDiagnostics {
                pc: Some(0x1000_0000),
                lr: Some(0x1000_0004),
                syscall_lev: Some(0),
                faulting_ea: None,
                fault_regs: Some(regs),
            },
            fault: None,
            syscall_args: Some(args),
        };
        let c = r.clone();
        assert_eq!(r, c);
        assert_eq!(
            r.local_diagnostics.fault_regs,
            c.local_diagnostics.fault_regs
        );
        assert_eq!(r.syscall_args, c.syscall_args);
    }
}
