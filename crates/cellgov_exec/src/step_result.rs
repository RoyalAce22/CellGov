//! Return shape of `ExecutionUnit::run_until_yield`.
//!
//! Effects flow through the `&mut Vec<Effect>` parameter; a
//! [`crate::YieldReason::Fault`] step has its effects discarded by the
//! commit pipeline.

use crate::yield_reason::YieldReason;
use cellgov_effects::FaultKind;
use cellgov_time::InstructionCost;

/// Per-step local diagnostics surfaced by an execution unit.
///
/// Every field is optional; consumers must tolerate `None` and render
/// `<unknown>` rather than fabricating a value.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalDiagnostics {
    /// Program counter at yield.
    pub pc: Option<u64>,
    /// Caller return address at yield. Populated on
    /// `YieldReason::Syscall` so HLE / LV2 dispatch can attribute the
    /// call site without a downcast through the registry.
    pub lr: Option<u64>,
    /// LEV field of the `sc` instruction (PPC64 Book III 2.3.1).
    /// LEV=0 is a kernel syscall, LEV=1 a hypercall (CBE Handbook
    /// 11.1), LEV>1 reserved. Populated on `YieldReason::Syscall`;
    /// the runtime classifier rejects LEV>=1 before LV2 dispatch.
    pub syscall_lev: Option<u8>,
    /// Effective address of the faulting access.
    pub faulting_ea: Option<u64>,
    /// Register snapshot at fault time. PPU and SPU units populate it
    /// on every fault; synthetic units may leave it `None`.
    pub fault_regs: Option<FaultRegisterDump>,
}

/// Arch-neutral register snapshot at fault time.
///
/// The field set matches `PpuStateHash`'s FNV-1a fingerprint
/// (GPR + LR + CTR + XER + CR) so CLI fault formatting and divergence
/// traces hash the same state. SPU dumps populate `xer = 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultRegisterDump {
    /// General-purpose registers r0..r31.
    pub gprs: [u64; 32],
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// PowerPC XER (SO/OV/CA carry across instructions). Zero on SPU.
    pub xer: u64,
    /// Condition register (8 4-bit fields).
    pub cr: u32,
}

impl FaultRegisterDump {
    /// All-zero register dump for tests.
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
    /// Diagnostics with every field unset.
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

    /// Diagnostics carrying only the program counter.
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

    /// Diagnostics carrying program counter and faulting effective address.
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

    /// Diagnostics carrying program counter and link register.
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

    /// Diagnostics carrying program counter, link register, and syscall LEV field.
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

    /// Diagnostics carrying program counter, faulting EA, and register dump.
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

/// Value returned by a single `run_until_yield` call.
///
/// # Invariants
///
/// - `fault.is_some()` iff `yield_reason == YieldReason::Fault`.
/// - `syscall_args.is_some()` iff `yield_reason == YieldReason::Syscall`.
///
/// Checked by [`ExecutionStepResult::is_well_formed`]; the runtime
/// routes fault attribution and HLE / LV2 dispatch on these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStepResult {
    /// Why the unit yielded control.
    pub yield_reason: YieldReason,
    /// Work retired this step; bridges to [`cellgov_time::GuestTicks`]
    /// via `From<InstructionCost>` in the commit pipeline, which
    /// advances guest time by `consumed_cost`.
    ///
    /// `InstructionCost::ZERO` is legal only on `YieldReason::Fault`.
    /// A non-fault zero-cost yield in a loop deadlocks the schedule.
    pub consumed_cost: InstructionCost,
    /// Per-step local diagnostics from the execution unit.
    pub local_diagnostics: LocalDiagnostics,
    /// Fault payload, present iff `yield_reason == YieldReason::Fault`.
    pub fault: Option<FaultKind>,
    /// Raw syscall arguments. Index 0 is the syscall number from the
    /// arch's syscall-number register (GPR 11 on PPC64 LV2; channel
    /// value on SPU); indices 1..=8 are argument registers in
    /// arch-defined order (GPR 3..=10 on PPC64 LV2). Responses go
    /// through the runtime's syscall-response table, not by mutating
    /// this array. Mapping is the emitting `ExecutionUnit`'s
    /// responsibility.
    pub syscall_args: Option<[u64; 9]>,
}

impl ExecutionStepResult {
    /// Whether both struct-level invariants hold.
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
