//! ExecutionStepResult well-formedness rules and LocalDiagnostics constructors.

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

impl FaultRegisterDump {
    /// All-zero register dump for tests.
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
