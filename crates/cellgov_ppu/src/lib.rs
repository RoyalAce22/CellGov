//! PPU execution unit for CellGov.
//!
//! Implements the `ExecutionUnit` trait for a Power Processing Unit.
//! The fetch-decode-execute loop lives here; instruction semantics
//! live in `exec.rs`; decoding lives in `decode.rs`.
//!
//! The PPU reads from committed shared memory (via `ExecutionContext`)
//! and communicates with the runtime through `Effect` packets.
//! Syscall dispatch translates LV2 syscall numbers into Effects
//! (managed SPU thread group lifecycle, TTY write, process exit).

pub mod decode;
pub mod exec;
mod fp;
pub mod instruction;
pub mod loader;
pub mod nid_db;
pub mod prx;
pub mod state;
pub mod syscall;

use crate::exec::{PpuFault, PpuStepOutcome};
use cellgov_effects::{FaultKind, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{Budget, GuestTicks};

/// Fault code constants for PPU faults encoded into `FaultKind::Guest`.
const FAULT_PC_OUT_OF_RANGE: u32 = 0x0102_0000;
const FAULT_DECODE_ERROR: u32 = 0x0105_0000;
const FAULT_INVALID_ADDRESS: u32 = 0x0106_0000;
const FAULT_UNSUPPORTED_SYSCALL: u32 = 0x0107_0000;

/// PPU execution unit snapshot for replay.
#[derive(Debug, Clone)]
pub struct PpuSnapshot {
    /// General-purpose registers.
    pub gpr: [u64; 32],
    /// Program counter.
    pub pc: u64,
    /// Condition register.
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
}

/// A Power Processing Unit execution unit.
///
/// Owns its architectural state (registers, PC, CR, LR, CTR, XER) and
/// implements the `ExecutionUnit` trait. The PPU fetches instructions
/// from committed guest memory, executes them, and emits Effects for
/// stores and syscalls.
pub struct PpuExecutionUnit {
    id: UnitId,
    state: state::PpuState,
    status: UnitStatus,
}

impl PpuExecutionUnit {
    /// Create a new PPU execution unit with zeroed state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::PpuState::new(),
            status: UnitStatus::Runnable,
        }
    }

    /// Mutable access to the PPU's architectural state.
    pub fn state_mut(&mut self) -> &mut state::PpuState {
        &mut self.state
    }

    /// Read access to the PPU's architectural state.
    pub fn state(&self) -> &state::PpuState {
        &self.state
    }
}

impl ExecutionUnit for PpuExecutionUnit {
    type Snapshot = PpuSnapshot;

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
    ) -> ExecutionStepResult {
        let mut remaining = budget.raw();
        let mut effects = Vec::new();

        if let Some(code) = ctx.syscall_return() {
            self.state.gpr[3] = code;
            self.state.pc += 4;
        }
        for &(reg, val) in ctx.register_writes() {
            if (reg as usize) < 32 {
                self.state.gpr[reg as usize] = val;
            }
        }

        let mem = ctx.memory().as_bytes();

        // Helper: build a memory-fault step result. Used by Load,
        // LoadVec, and FpLoad to avoid duplicating the fault
        // construction.
        macro_rules! mem_fault {
            ($step_pc:expr, $ea:expr, $budget:expr, $remaining:expr, $effects:expr) => {
                ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_budget: Budget::new($budget.raw() - $remaining),
                    emitted_effects: $effects,
                    local_diagnostics: LocalDiagnostics::with_pc_ea($step_pc, $ea),
                    fault: Some(FaultKind::Guest(FAULT_INVALID_ADDRESS)),
                    syscall_args: None,
                }
            };
        }

        loop {
            // Capture PC before fetch/decode/execute for diagnostics.
            let step_pc = self.state.pc;
            // Fetch: read 4 bytes from committed guest memory at PC.
            let pc = step_pc as usize;
            if pc + 4 > mem.len() {
                self.status = UnitStatus::Faulted;
                return ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_budget: Budget::new(budget.raw() - remaining),
                    emitted_effects: effects,
                    local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                    fault: Some(FaultKind::Guest(FAULT_PC_OUT_OF_RANGE)),
                    syscall_args: None,
                };
            }
            let raw = u32::from_be_bytes([mem[pc], mem[pc + 1], mem[pc + 2], mem[pc + 3]]);

            // Decode
            let insn = match decode::decode(raw) {
                Ok(i) => i,
                Err(_) => {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        emitted_effects: effects,
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_DECODE_ERROR)),
                        syscall_args: None,
                    };
                }
            };

            // Execute
            match exec::execute(&insn, &mut self.state, self.id) {
                PpuStepOutcome::Continue => {
                    self.state.pc += 4;
                }
                PpuStepOutcome::Branch => {
                    // PC already set by the branch instruction.
                }
                PpuStepOutcome::Load { ea, size, rt } => {
                    let addr = ea as usize;
                    if addr + size as usize > mem.len() {
                        self.status = UnitStatus::Faulted;
                        return mem_fault!(step_pc, ea, budget, remaining, effects);
                    }
                    let val = match size {
                        1 => mem[addr] as u64,
                        2 => u16::from_be_bytes([mem[addr], mem[addr + 1]]) as u64,
                        4 => u32::from_be_bytes([
                            mem[addr],
                            mem[addr + 1],
                            mem[addr + 2],
                            mem[addr + 3],
                        ]) as u64,
                        8 => u64::from_be_bytes([
                            mem[addr],
                            mem[addr + 1],
                            mem[addr + 2],
                            mem[addr + 3],
                            mem[addr + 4],
                            mem[addr + 5],
                            mem[addr + 6],
                            mem[addr + 7],
                        ]),
                        _ => 0,
                    };
                    self.state.gpr[rt as usize] = val;
                    self.state.pc += 4;
                }
                PpuStepOutcome::Store { ea, size, value } => {
                    let bytes = match size {
                        1 => vec![value as u8],
                        2 => (value as u16).to_be_bytes().to_vec(),
                        4 => (value as u32).to_be_bytes().to_vec(),
                        8 => value.to_be_bytes().to_vec(),
                        _ => vec![],
                    };
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), size as u64) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: WritePayload::new(bytes),
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        });
                    }
                    self.state.pc += 4;
                }
                PpuStepOutcome::LoadVec { ea, vt } => {
                    let addr = ea as usize;
                    if addr + 16 > mem.len() {
                        self.status = UnitStatus::Faulted;
                        return mem_fault!(step_pc, ea, budget, remaining, effects);
                    }
                    let mut bytes = [0u8; 16];
                    bytes.copy_from_slice(&mem[addr..addr + 16]);
                    self.state.vr[vt as usize] = u128::from_be_bytes(bytes);
                    self.state.pc += 4;
                }
                PpuStepOutcome::FpLoad { ea, size, frt } => {
                    let addr = ea as usize;
                    if addr + size as usize > mem.len() {
                        self.status = UnitStatus::Faulted;
                        return mem_fault!(step_pc, ea, budget, remaining, effects);
                    }
                    let val = match size {
                        4 => {
                            let bits = u32::from_be_bytes([
                                mem[addr],
                                mem[addr + 1],
                                mem[addr + 2],
                                mem[addr + 3],
                            ]);
                            // lfs: convert single to double
                            (f32::from_bits(bits) as f64).to_bits()
                        }
                        8 => u64::from_be_bytes([
                            mem[addr],
                            mem[addr + 1],
                            mem[addr + 2],
                            mem[addr + 3],
                            mem[addr + 4],
                            mem[addr + 5],
                            mem[addr + 6],
                            mem[addr + 7],
                        ]),
                        _ => 0,
                    };
                    self.state.fpr[frt as usize] = val;
                    self.state.pc += 4;
                }
                PpuStepOutcome::FpStore { ea, size, value } => {
                    let bytes = match size {
                        4 => {
                            // stfs: convert double to single
                            let f = f64::from_bits(value) as f32;
                            f.to_be_bytes().to_vec()
                        }
                        8 => value.to_be_bytes().to_vec(),
                        _ => vec![],
                    };
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), size as u64) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: WritePayload::new(bytes),
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        });
                    }
                    self.state.pc += 4;
                }
                PpuStepOutcome::StoreVec { ea, value } => {
                    let bytes = value.to_be_bytes().to_vec();
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), 16) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: WritePayload::new(bytes),
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        });
                    }
                    self.state.pc += 4;
                }
                PpuStepOutcome::Syscall => {
                    let s = &self.state;
                    let args = [
                        s.gpr[11], s.gpr[3], s.gpr[4], s.gpr[5], s.gpr[6], s.gpr[7], s.gpr[8],
                        s.gpr[9], s.gpr[10],
                    ];
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Syscall,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        emitted_effects: effects,
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: Some(args),
                    };
                }
                PpuStepOutcome::Yield {
                    effects: step_effects,
                    reason,
                } => {
                    effects.extend(step_effects);
                    if reason == YieldReason::Finished {
                        self.status = UnitStatus::Finished;
                    } else {
                        self.state.pc += 4;
                    }
                    return ExecutionStepResult {
                        yield_reason: reason,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        emitted_effects: effects,
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                PpuStepOutcome::Fault(f) => {
                    self.status = UnitStatus::Faulted;
                    let code = match f {
                        PpuFault::PcOutOfRange(a) => FAULT_PC_OUT_OF_RANGE | a as u32,
                        PpuFault::InvalidAddress(a) => FAULT_INVALID_ADDRESS | a as u32,
                        PpuFault::UnsupportedSyscall(n) => FAULT_UNSUPPORTED_SYSCALL | n as u32,
                    };
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        emitted_effects: effects,
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
                    emitted_effects: effects,
                    local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                    fault: None,
                    syscall_args: None,
                };
            }
        }
    }

    fn snapshot(&self) -> PpuSnapshot {
        PpuSnapshot {
            gpr: self.state.gpr,
            pc: self.state.pc,
            cr: self.state.cr,
            lr: self.state.lr,
            ctr: self.state.ctr,
        }
    }
}

#[cfg(test)]
mod tests;
