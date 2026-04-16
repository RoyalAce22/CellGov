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
mod exec_vec;
mod fp;
pub mod instruction;
pub mod loader;
pub mod nid_db;
pub mod prx;
pub mod shadow;
pub mod sprx;
pub mod state;
pub mod syscall;

use crate::exec::{PpuFault, PpuStepOutcome};
use cellgov_effects::{FaultKind, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FaultRegisterDump, LocalDiagnostics,
    UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::{Budget, GuestTicks};

/// PPU tried to fetch at an address beyond guest memory.
pub const FAULT_PC_OUT_OF_RANGE: u32 = 0x0102_0000;
/// Instruction word did not match any implemented encoding.
pub const FAULT_DECODE_ERROR: u32 = 0x0105_0000;
/// Load or store targeted an out-of-bounds guest address.
pub const FAULT_INVALID_ADDRESS: u32 = 0x0106_0000;
/// Syscall number or VMX sub-opcode has no handler.
pub const FAULT_UNSUPPORTED_SYSCALL: u32 = 0x0107_0000;
/// Debug breakpoint fired at a user-requested PC.
pub const FAULT_DEBUG_BREAK: u32 = 0x0108_0000;

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
    /// If set, `run_until_yield` entries matching `break_pc` skip the
    /// first `break_skip` hits, then on the next hit emit a synthetic
    /// FAULT_DEBUG_BREAK fault with full register capture.
    break_pc: Option<u64>,
    /// Number of hits to skip before firing the break (0 = first hit).
    break_skip: u32,
    /// When true, `run_until_yield` pushes one `(pc, state_hash)` pair
    /// into `per_step_hashes` after every retired instruction. Off by
    /// default -- the inner loop skips the push entirely when off.
    per_step_trace: bool,
    /// Per-step fingerprints collected during the current
    /// `run_until_yield`. Drained by the runtime via
    /// `ExecutionUnit::drain_retired_state_hashes`.
    per_step_hashes: Vec<(u64, u64)>,
    /// Optional inclusive window of step indices (relative to this
    /// unit's own retirement counter) for the zoom-in trace. When
    /// the unit's retirement counter is in `[lo, hi]`, the unit
    /// pushes a full register snapshot into `per_step_full_states`
    /// after retiring the instruction. None disables the zoom-in
    /// path entirely; the hot loop's check is one branch when the
    /// window is None and an integer-range test when it is Some.
    full_state_window: Option<(u64, u64)>,
    /// Per-unit retirement counter used to test against
    /// `full_state_window`. Increments only on successful retirement,
    /// matching the gate the per-step hash uses.
    retirement_counter: u64,
    /// Full-state snapshots collected during the current
    /// `run_until_yield`. Drained by the runtime via
    /// `ExecutionUnit::drain_retired_state_full`. Each entry is
    /// `(pc, gpr, lr, ctr, xer, cr)` for one retired instruction
    /// inside the configured window.
    per_step_full_states: Vec<(u64, [u64; 32], u64, u64, u64, u32)>,
    /// Predecoded instruction shadow for the main text region.
    /// Built once after boot loading; the hot-path fetch checks
    /// this before falling back to raw memory + decode. Stale
    /// slots (from guest-visible code writes) re-decode on the
    /// next fetch.
    instruction_shadow: Option<shadow::PredecodedShadow>,
}

impl PpuExecutionUnit {
    /// Create a new PPU execution unit with zeroed state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::PpuState::new(),
            status: UnitStatus::Runnable,
            break_pc: None,
            break_skip: 0,
            per_step_trace: false,
            per_step_hashes: Vec::new(),
            full_state_window: None,
            retirement_counter: 0,
            per_step_full_states: Vec::new(),
            instruction_shadow: None,
        }
    }

    /// Turn per-step state-hash tracing on or off.
    ///
    /// When on, `run_until_yield` pushes one `(pc, state_hash)` pair
    /// into an internal buffer after every retired instruction. The
    /// runtime drains the buffer through
    /// `ExecutionUnit::drain_retired_state_hashes` and converts each
    /// entry into a `TraceRecord::PpuStateHash`. Off by default.
    pub fn set_per_step_trace(&mut self, on: bool) {
        self.per_step_trace = on;
    }

    /// Whether per-step state-hash tracing is currently on.
    pub fn per_step_trace(&self) -> bool {
        self.per_step_trace
    }

    /// Configure a zoom-in window: full-register snapshots are
    /// captured for instructions retired with index in `[lo, hi]`
    /// (inclusive). Index counting is per-unit and starts at 0 for
    /// the first retired instruction. Pass `None` to disable.
    ///
    /// The full snapshots are collected separately from per-step
    /// hashes and drained via a separate trait method, so the main
    /// per-step trace stream stays homogeneous.
    pub fn set_full_state_window(&mut self, window: Option<(u64, u64)>) {
        self.full_state_window = window;
    }

    /// Current zoom-in window, if any.
    pub fn full_state_window(&self) -> Option<(u64, u64)> {
        self.full_state_window
    }

    /// Set a breakpoint: fires on the `skip`-th-plus-one hit at `pc`
    /// (skip=0 means first hit). Emits a synthetic FAULT_DEBUG_BREAK.
    pub fn set_break_pc(&mut self, pc: u64, skip: u32) {
        self.break_pc = Some(pc);
        self.break_skip = skip;
    }

    /// Mutable access to the PPU's architectural state.
    pub fn state_mut(&mut self) -> &mut state::PpuState {
        &mut self.state
    }

    /// Read access to the PPU's architectural state.
    pub fn state(&self) -> &state::PpuState {
        &self.state
    }

    /// Attach a predecoded instruction shadow. The shadow must be
    /// built from the same memory the PPU will fetch from; the
    /// caller is responsible for building it after all boot-time
    /// code writes (ELF load, PRX load, HLE stub planting) have
    /// finished and before the step loop begins.
    pub fn set_instruction_shadow(&mut self, shadow: shadow::PredecodedShadow) {
        self.instruction_shadow = Some(shadow);
    }
}

impl PpuExecutionUnit {
    fn capture_regs(&self) -> FaultRegisterDump {
        FaultRegisterDump {
            gprs: self.state.gpr,
            lr: self.state.lr,
            ctr: self.state.ctr,
            cr: self.state.cr,
        }
    }

    fn fault_diag(&self, pc: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            faulting_ea: None,
            fault_regs: Some(self.capture_regs()),
        }
    }

    fn fault_diag_ea(&self, pc: u64, ea: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            faulting_ea: Some(ea),
            fault_regs: Some(self.capture_regs()),
        }
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

        // Counted debug breakpoint. Skips `break_skip` hits, then fires
        // a synthetic FAULT_DEBUG_BREAK with full register capture.
        if self.break_pc == Some(self.state.pc) {
            if self.break_skip > 0 {
                self.break_skip -= 1;
            } else {
                self.break_pc = None;
                return ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_budget: Budget::new(0),
                    emitted_effects: vec![],
                    local_diagnostics: self.fault_diag(self.state.pc),
                    fault: Some(FaultKind::Guest(FAULT_DEBUG_BREAK)),
                    syscall_args: None,
                };
            }
        }

        let mem = ctx.memory().as_bytes();
        // Cache every region as (base, bytes) pairs so load paths can
        // resolve effective addresses against the primary-thread stack
        // at 0xD0000000 and any other auxiliary region without paying
        // a BTreeMap lookup on every access. The fetch path keeps
        // using `mem` (the base-0 region) since code always lives in
        // the main region.
        //
        // Stack-allocated fixed-size table: PS3 runs with <= 4 regions
        // (main, stack, rsx, spu_reserved) and no production codepath
        // adds more. Avoiding the heap allocation matters here because
        // `run_until_yield` is called once per retired instruction in
        // Budget=1 mode (run-game, bench-boot, fault-driven replay),
        // so a per-call `Vec::new` is a per-step allocation on the hot
        // path. `MAX_REGIONS` is wider than the current usage to keep
        // headroom without an assertion-failure risk.
        const MAX_REGIONS: usize = 8;
        let mut region_views_storage: [(u64, &[u8]); MAX_REGIONS] =
            [(0, &[] as &[u8]); MAX_REGIONS];
        let mut n_regions = 0usize;
        for r in ctx.memory().regions() {
            debug_assert!(
                n_regions < MAX_REGIONS,
                "region_views table too small; bump MAX_REGIONS"
            );
            if n_regions < MAX_REGIONS {
                region_views_storage[n_regions] = (r.base(), r.bytes());
                n_regions += 1;
            }
        }
        let region_views = &region_views_storage[..n_regions];
        let load_slice = |ea: u64, len: usize| -> Option<&[u8]> {
            let end = ea.checked_add(len as u64)?;
            for &(base, bytes) in region_views {
                let region_end = base + bytes.len() as u64;
                if ea >= base && end <= region_end {
                    let offset = (ea - base) as usize;
                    return Some(&bytes[offset..offset + len]);
                }
            }
            None
        };

        // Helper: build a memory-fault step result. Used by Load,
        // LoadVec, and FpLoad to avoid duplicating the fault
        // construction.
        macro_rules! mem_fault {
            ($step_pc:expr, $ea:expr, $budget:expr, $remaining:expr, $effects:expr) => {
                ExecutionStepResult {
                    yield_reason: YieldReason::Fault,
                    consumed_budget: Budget::new($budget.raw() - $remaining),
                    emitted_effects: $effects,
                    local_diagnostics: self.fault_diag_ea($step_pc, $ea),
                    fault: Some(FaultKind::Guest(FAULT_INVALID_ADDRESS)),
                    syscall_args: None,
                }
            };
        }

        loop {
            // Capture PC before fetch/decode/execute for diagnostics.
            let step_pc = self.state.pc;

            // Fetch + Decode: try the predecoded shadow first (O(1)
            // index). On miss (stale, out-of-range, decode error)
            // fall back to the raw-memory path.
            let insn = if let Some(cached) = self
                .instruction_shadow
                .as_ref()
                .and_then(|s| s.get(step_pc))
            {
                cached
            } else {
                let pc = step_pc as usize;
                if pc + 4 > mem.len() {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        emitted_effects: effects,
                        local_diagnostics: self.fault_diag(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_PC_OUT_OF_RANGE)),
                        syscall_args: None,
                    };
                }
                let raw = u32::from_be_bytes([mem[pc], mem[pc + 1], mem[pc + 2], mem[pc + 3]]);
                match decode::decode(raw) {
                    Ok(i) => {
                        // Refresh the shadow slot so subsequent
                        // fetches at this PC hit the fast path.
                        if let Some(s) = self.instruction_shadow.as_mut() {
                            let _ = s.refresh(step_pc, raw);
                        }
                        i
                    }
                    Err(_) => {
                        self.status = UnitStatus::Faulted;
                        return ExecutionStepResult {
                            yield_reason: YieldReason::Fault,
                            consumed_budget: Budget::new(budget.raw() - remaining),
                            emitted_effects: effects,
                            local_diagnostics: self.fault_diag(step_pc),
                            fault: Some(FaultKind::Guest(FAULT_DECODE_ERROR)),
                            syscall_args: None,
                        };
                    }
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
                    let slice = match load_slice(ea, size as usize) {
                        Some(s) => s,
                        None => {
                            self.status = UnitStatus::Faulted;
                            return mem_fault!(step_pc, ea, budget, remaining, effects);
                        }
                    };
                    let val = match size {
                        1 => slice[0] as u64,
                        2 => u16::from_be_bytes([slice[0], slice[1]]) as u64,
                        4 => u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as u64,
                        8 => u64::from_be_bytes([
                            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6],
                            slice[7],
                        ]),
                        _ => 0,
                    };
                    self.state.gpr[rt as usize] = val;
                    self.state.pc += 4;
                }
                PpuStepOutcome::LoadSigned { ea, size, rt } => {
                    let slice = match load_slice(ea, size as usize) {
                        Some(s) => s,
                        None => {
                            self.status = UnitStatus::Faulted;
                            return mem_fault!(step_pc, ea, budget, remaining, effects);
                        }
                    };
                    let val: u64 = match size {
                        1 => (slice[0] as i8) as i64 as u64,
                        2 => i16::from_be_bytes([slice[0], slice[1]]) as i64 as u64,
                        4 => i32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64
                            as u64,
                        _ => 0,
                    };
                    self.state.gpr[rt as usize] = val;
                    self.state.pc += 4;
                }
                PpuStepOutcome::Store { ea, size, value } => {
                    let payload = match size {
                        1 => WritePayload::from_slice(&[value as u8]),
                        2 => WritePayload::from_slice(&(value as u16).to_be_bytes()),
                        4 => WritePayload::from_slice(&(value as u32).to_be_bytes()),
                        8 => WritePayload::from_slice(&value.to_be_bytes()),
                        _ => WritePayload::from_slice(&[]),
                    };
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), size as u64) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: payload,
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        });
                    }
                    self.state.pc += 4;
                }
                PpuStepOutcome::LoadVec { ea, vt } => {
                    let slice = match load_slice(ea, 16) {
                        Some(s) => s,
                        None => {
                            self.status = UnitStatus::Faulted;
                            return mem_fault!(step_pc, ea, budget, remaining, effects);
                        }
                    };
                    let mut bytes = [0u8; 16];
                    bytes.copy_from_slice(slice);
                    self.state.vr[vt as usize] = u128::from_be_bytes(bytes);
                    self.state.pc += 4;
                }
                PpuStepOutcome::FpLoad { ea, size, frt } => {
                    let slice = match load_slice(ea, size as usize) {
                        Some(s) => s,
                        None => {
                            self.status = UnitStatus::Faulted;
                            return mem_fault!(step_pc, ea, budget, remaining, effects);
                        }
                    };
                    let val = match size {
                        4 => {
                            let bits = u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]);
                            // lfs: convert single to double
                            (f32::from_bits(bits) as f64).to_bits()
                        }
                        8 => u64::from_be_bytes([
                            slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6],
                            slice[7],
                        ]),
                        _ => 0,
                    };
                    self.state.fpr[frt as usize] = val;
                    self.state.pc += 4;
                }
                PpuStepOutcome::FpStore { ea, size, value } => {
                    let payload = match size {
                        4 => {
                            // stfs: convert double to single
                            let f = f64::from_bits(value) as f32;
                            WritePayload::from_slice(&f.to_be_bytes())
                        }
                        8 => WritePayload::from_slice(&value.to_be_bytes()),
                        _ => WritePayload::from_slice(&[]),
                    };
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), size as u64) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: payload,
                            ordering: PriorityClass::Normal,
                            source: self.id,
                            source_time: GuestTicks::ZERO,
                        });
                    }
                    self.state.pc += 4;
                }
                PpuStepOutcome::StoreVec { ea, value } => {
                    let payload = WritePayload::from_slice(&value.to_be_bytes());
                    if let Some(range) = ByteRange::new(GuestAddr::new(ea), 16) {
                        effects.push(cellgov_effects::Effect::SharedWriteIntent {
                            range,
                            bytes: payload,
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
                        local_diagnostics: self.fault_diag(step_pc),
                        fault: Some(FaultKind::Guest(code)),
                        syscall_args: None,
                    };
                }
            }

            if self.per_step_trace {
                self.per_step_hashes
                    .push((step_pc, self.state.state_hash()));
            }

            // Zoom-in window. The window check is gated on the
            // window being Some so the hot path stays one branch when
            // off.
            if let Some((lo, hi)) = self.full_state_window {
                if self.retirement_counter >= lo && self.retirement_counter <= hi {
                    let s = &self.state;
                    self.per_step_full_states
                        .push((step_pc, s.gpr, s.lr, s.ctr, s.xer, s.cr));
                }
            }
            self.retirement_counter += 1;

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

    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        std::mem::take(&mut self.per_step_full_states)
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        if let Some(s) = self.instruction_shadow.as_mut() {
            s.invalidate_range(addr, len);
        }
    }
}

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;
