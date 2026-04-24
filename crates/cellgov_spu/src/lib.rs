//! Synergistic Processing Unit execution unit.
//!
//! Owns the fetch-decode-execute loop; instruction semantics live in
//! [`exec`], decoding in [`decode`]. Guest-visible writes flow through
//! `Effect` packets; reads into the 256 KB local store are serviced
//! from the frozen committed snapshot exposed by
//! [`cellgov_exec::ExecutionContext::memory`].

pub mod channels;
pub mod decode;
pub mod exec;
pub mod instruction;
pub mod loader;
pub mod state;

use crate::exec::{SpuFault, SpuStepOutcome};
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_time::Budget;

/// Fault code constants encoded into `FaultKind::Guest`.
const FAULT_LS_OUT_OF_RANGE: u32 = 0x0002_0000;
const FAULT_UNSUPPORTED_CHANNEL: u32 = 0x0003_0000;
const FAULT_UNSUPPORTED_MFC_CMD: u32 = 0x0004_0000;
const FAULT_DECODE_ERROR: u32 = 0x0005_0000;

/// SPU execution unit snapshot for replay.
#[derive(Debug, Clone)]
pub struct SpuSnapshot {
    /// Register file.
    pub regs: [[u8; 16]; 128],
    /// Program counter.
    pub pc: u32,
    /// Local store contents.
    pub ls: Vec<u8>,
    /// Canonical line address of the atomic reservation; `None` when
    /// no reservation is held.
    pub reservation_line: Option<u64>,
}

/// A Synergistic Processing Unit execution unit.
pub struct SpuExecutionUnit {
    id: UnitId,
    state: state::SpuState,
    status: UnitStatus,
}

impl SpuExecutionUnit {
    /// Create a new SPU execution unit with zeroed state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::SpuState::new(),
            status: UnitStatus::Runnable,
        }
    }

    /// Mutable access to the SPU's architectural state.
    pub fn state_mut(&mut self) -> &mut state::SpuState {
        &mut self.state
    }

    /// Read access to the SPU's architectural state.
    pub fn state(&self) -> &state::SpuState {
        &self.state
    }
}

impl ExecutionUnit for SpuExecutionUnit {
    type Snapshot = SpuSnapshot;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        self.status
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        // Mailbox yield leaves PC at the rdch so retry is possible when
        // the mailbox was empty; on message delivery we advance past it.
        if let Some(&msg) = ctx.received_messages().first() {
            let rt = self.state.channels.pending_mbox_rt.take().unwrap_or(2);
            self.state.set_reg_word_splat(rt, msg);
            self.state.pc += 4;
        }

        if let Some((ea, lsa, size)) = self.state.channels.pending_get.take() {
            let src_start = ea as usize;
            let src_end = src_start + size as usize;
            let mem = ctx.memory().as_bytes();
            if src_end <= mem.len() {
                let dst_start = lsa as usize;
                let dst_end = dst_start + size as usize;
                if dst_end <= self.state.ls.len() {
                    self.state.ls[dst_start..dst_end].copy_from_slice(&mem[src_start..src_end]);
                }
            }
        }

        // Mirror cross-unit invalidation of the atomic reservation
        // from the committed table. The context view is frozen for
        // this step, so one check at entry suffices.
        if self.state.reservation.is_some() && !ctx.reservation_held(self.id) {
            self.state.reservation = None;
        }

        let mut remaining = budget.raw();
        effects.clear();

        loop {
            let step_pc = self.state.pc as u64;
            let raw = match self.state.fetch() {
                Some(w) => w,
                None => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_LS_OUT_OF_RANGE | self.state.pc)),
                        syscall_args: None,
                    };
                }
            };

            let insn = match decode::decode(raw) {
                Ok(i) => i,
                Err(_) => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_DECODE_ERROR)),
                        syscall_args: None,
                    };
                }
            };

            match exec::execute(&insn, &mut self.state, self.id) {
                SpuStepOutcome::Continue => {
                    self.state.pc += 4;
                }
                SpuStepOutcome::Branch => {}
                SpuStepOutcome::Yield {
                    effects: step_effects,
                    reason,
                } => {
                    effects.extend(step_effects);
                    if reason == YieldReason::Finished {
                        self.status = UnitStatus::Finished;
                    } else if reason == YieldReason::MailboxAccess {
                        // PC stays on the rdch; advance occurs at the
                        // top of run_until_yield on message delivery.
                    } else {
                        self.state.pc += 4;
                    }
                    return ExecutionStepResult {
                        yield_reason: reason,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                SpuStepOutcome::MemoryRead {
                    ea,
                    lsa,
                    size,
                    acquire_line,
                } => {
                    let src_start = ea as usize;
                    let src_end = src_start + size as usize;
                    let mem = ctx.memory().as_bytes();
                    if src_end <= mem.len() {
                        let dst_start = lsa as usize;
                        let dst_end = dst_start + size as usize;
                        if dst_end <= self.state.ls.len() {
                            self.state.ls[dst_start..dst_end]
                                .copy_from_slice(&mem[src_start..src_end]);
                        }
                    }
                    // MFC_GETLLAR additionally installs the unit's
                    // entry in the reservation table.
                    if let Some(line_addr) = acquire_line {
                        effects.push(Effect::ReservationAcquire {
                            line_addr,
                            source: self.id,
                        });
                    }
                    self.state.pc += 4;
                }
                SpuStepOutcome::Fault(f) => {
                    self.status = UnitStatus::Faulted;
                    let code = match f {
                        SpuFault::LsOutOfRange(a) => FAULT_LS_OUT_OF_RANGE | a,
                        SpuFault::UnsupportedChannel { channel, .. } => {
                            FAULT_UNSUPPORTED_CHANNEL | channel as u32
                        }
                        SpuFault::UnsupportedMfcCommand(c) => FAULT_UNSUPPORTED_MFC_CMD | c,
                    };
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(code)),
                        syscall_args: None,
                    };
                }
            }

            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                return ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_budget: budget,
                    local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                    fault: None,
                    syscall_args: None,
                };
            }
        }
    }

    fn snapshot(&self) -> SpuSnapshot {
        SpuSnapshot {
            regs: self.state.regs,
            pc: self.state.pc,
            ls: self.state.ls.clone(),
            reservation_line: self.state.reservation.map(|l| l.addr()),
        }
    }
}

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
