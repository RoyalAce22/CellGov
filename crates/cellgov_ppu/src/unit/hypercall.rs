//! The hypercall yield of a thread with supervisor registers.
//!
//! On such a thread, `sc` with LEV = 1 is a hypercall in either
//! processor state. The thread yields it to the runtime, which answers
//! as the hypervisor. The answer arrives as the runtime's committed inputs of
//! the next batch: r3, the outputs from r4, and the step past `sc`. The
//! runtime names the request's PPE hardware thread from the thread's
//! CTRL.

use cellgov_effects::Effect;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_ps3_abi::hw::ppu_spr::CTRL_CT_THREAD1;

use super::batch::BatchEntry;
use super::ppu_unit::PpuExecutionUnit;
use crate::state;

impl PpuExecutionUnit {
    /// Ends the batch at the `sc` at `pc` on a thread with supervisor registers.
    ///
    /// The `sc` retired with LEV `lev`:
    /// - LEV = 1 yields a hypercall;
    /// - any other LEV takes the system call interrupt at the kernel's
    ///   vector.
    ///
    /// The hypervisor reads the processor state of a LEV = 1 call from
    /// `ExecutionUnit::in_problem_state`.
    pub(super) fn supervisor_sc_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        lev: u8,
    ) -> ExecutionStepResult {
        if lev == 1 {
            return self.hypercall_yield(entry, effects, pc);
        }
        self.system_call_yield(entry, effects, pc, lev)
    }

    /// Yields the hypercall of the `sc` at `pc`: its number from r11 and
    /// its inputs from r3 to r10.
    ///
    /// The `sc` stays the thread's pc until the runtime's answer moves it
    /// past. The issue model charges nothing past the `sc`, because no
    /// hypercall number has a measured cost.
    fn hypercall_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
    ) -> ExecutionStepResult {
        self.timing.drop_pending_syscall();
        self.close_block(effects);
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: self.cost(entry),
            local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(pc, self.state.lr(), 1),
            fault: None,
            syscall_args: Some(state::ppu_syscall_args(&self.state)),
        }
    }

    /// The PPE hardware thread the thread's CTRL names: 1 when CT names
    /// thread 1, else 0.
    ///
    /// [CBE-Handbook p:307 s:10.4.4] CTRL bits 0:1 (CT) name the thread that reads CTRL.
    pub(super) fn ctrl_thread(&self) -> u8 {
        u8::from(self.state.ctrl & CTRL_CT_THREAD1 != 0)
    }
}
