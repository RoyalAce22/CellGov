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
    /// LEV field of the `sc` instruction.
    // [PPC-Book1 p:26 s:2.4.2 System Call Instruction] LEV=0 user-mode kernel syscall path.
    // [PPC-Book3 p:12 s:2.3.1] LEV=1 invokes the hypervisor; LEV>1 reserved.
    /// Populated on `YieldReason::Syscall`; the runtime classifier
    /// rejects LEV >= 1 before LV2 dispatch.
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
#[path = "tests/step_result_tests.rs"]
mod tests;
