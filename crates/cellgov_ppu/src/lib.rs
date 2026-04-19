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
pub mod store_buffer;
pub mod syscall;

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::store_buffer::StoreBuffer;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FaultRegisterDump, LocalDiagnostics,
    UnitStatus, YieldReason,
};
use cellgov_time::Budget;

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
    /// Diagnostic counters for the instruction-fetch path. Incremented
    /// once per retired instruction: `shadow_hits` when the predecoded
    /// shadow returned Some at the fetch PC, `shadow_misses` when the
    /// fallback decode-on-fetch path was taken (out-of-shadow address,
    /// stale slot, or first-time fill). A steadily rising miss rate at
    /// the same PCs across runs is a silent perf cliff -- code has
    /// moved outside the shadowed base-0 region.
    shadow_hits: u64,
    shadow_misses: u64,
    /// Intra-block store-forwarding buffer. Pending stores
    /// accumulate here during the inner loop and are flushed to
    /// effects at every yield/fault/budget-exhaustion point.
    store_buf: StoreBuffer,
    /// When Some(1), force Budget=1 on the next run_until_yield
    /// call regardless of the caller's budget. Set after a mid-block
    /// fault so the re-execution path commits instructions one at a
    /// time up to the faulting instruction.
    budget_override: Option<u64>,
    /// When true, each retired instruction is re-decoded from
    /// committed memory (bypassing the shadow) and counted in
    /// `profile_insns` and `profile_pairs`. Off by default.
    profile_mode: bool,
    /// Individual instruction variant frequency (raw decoded stream).
    profile_insns: std::collections::BTreeMap<&'static str, u64>,
    /// Adjacent instruction-pair frequency (raw decoded stream,
    /// pc and pc+4 both decoded from committed memory).
    profile_pairs: std::collections::BTreeMap<(&'static str, &'static str), u64>,
    /// Previous instruction's variant name for pair counting.
    profile_prev: Option<&'static str>,
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
            shadow_hits: 0,
            shadow_misses: 0,
            store_buf: StoreBuffer::new(),
            budget_override: None,
            profile_mode: false,
            profile_insns: std::collections::BTreeMap::new(),
            profile_pairs: std::collections::BTreeMap::new(),
            profile_prev: None,
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

    /// Turn instruction profiling on or off.
    pub fn set_profile_mode(&mut self, on: bool) {
        self.profile_mode = on;
    }

    /// Accumulated instruction frequency data (raw decoded stream).
    pub fn profile_insns(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.profile_insns
    }

    /// Accumulated adjacent-pair frequency data (raw decoded stream).
    pub fn profile_pairs(&self) -> &std::collections::BTreeMap<(&'static str, &'static str), u64> {
        &self.profile_pairs
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

    /// Return `(shadow_hits, shadow_misses)` counters. Hits +
    /// misses equals the number of instructions this unit has
    /// fetched; a high miss ratio indicates code executing outside
    /// the predecoded shadow region (e.g. in a PRX body above
    /// 0x10000000 or in a stub trampoline past the shadow end)
    /// and falling back to decode-on-fetch. Correctness is
    /// preserved on misses; only the fast path is lost.
    pub fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let max_budget = match self.budget_override.take() {
            Some(b) => b,
            None => budget.raw(),
        };
        let mut remaining = max_budget;
        effects.clear();
        self.store_buf.clear();

        let snapshot = if max_budget > 1 {
            Some(self.state.clone())
        } else {
            None
        };

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
                self.shadow_hits += 1;
                cached
            } else {
                self.shadow_misses += 1;
                let pc = step_pc as usize;
                if pc + 4 > mem.len() {
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: self.fault_diag(step_pc),
                        fault: Some(FaultKind::Guest(FAULT_PC_OUT_OF_RANGE)),
                        syscall_args: None,
                    };
                }
                let raw = u32::from_be_bytes([mem[pc], mem[pc + 1], mem[pc + 2], mem[pc + 3]]);
                match decode::decode(raw) {
                    Ok(i) => {
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
                            local_diagnostics: self.fault_diag(step_pc),
                            fault: Some(FaultKind::Guest(FAULT_DECODE_ERROR)),
                            syscall_args: None,
                        };
                    }
                }
            };

            // Consumed slots are placeholders left by superinstruction
            // pairing. The preceding superinstruction already did the
            // work; just advance PC and burn one budget tick.
            if matches!(insn, instruction::PpuInstruction::Consumed) {
                self.state.pc += 4;
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    self.store_buf.flush(effects, self.id);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::BudgetExhausted,
                        consumed_budget: budget,
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                continue;
            }

            // Execute
            match exec::execute(
                &insn,
                &mut self.state,
                self.id,
                region_views,
                effects,
                &mut self.store_buf,
            ) {
                ExecuteVerdict::Continue => {
                    self.state.pc += 4;
                }
                ExecuteVerdict::Branch => {
                    // PC already set by the branch instruction.
                }
                ExecuteVerdict::Syscall => {
                    self.store_buf.flush(effects, self.id);
                    let s = &self.state;
                    let args = [
                        s.gpr[11], s.gpr[3], s.gpr[4], s.gpr[5], s.gpr[6], s.gpr[7], s.gpr[8],
                        s.gpr[9], s.gpr[10],
                    ];
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Syscall,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: Some(args),
                    };
                }
                ExecuteVerdict::Fault(f) => {
                    // Capture fault diagnostic at the failing instruction
                    // BEFORE any state rollback so registers reflect the
                    // actual fault context (not the batch-start snapshot).
                    let diag = self.fault_diag(step_pc);
                    // Roll back to snapshot when faulting mid-batch: the
                    // fault rule discards every effect emitted in this
                    // batch, so state must roll back to keep register
                    // and memory views consistent.
                    if remaining < max_budget {
                        if let Some(snap) = snapshot.as_ref() {
                            self.state = snap.clone();
                            self.store_buf.clear();
                        }
                    } else {
                        self.store_buf.flush(effects, self.id);
                    }
                    effects.clear();
                    self.status = UnitStatus::Faulted;
                    let code = match f {
                        PpuFault::PcOutOfRange(a) => FAULT_PC_OUT_OF_RANGE | a as u32,
                        PpuFault::InvalidAddress(a) => FAULT_INVALID_ADDRESS | a as u32,
                        PpuFault::UnsupportedSyscall(n) => FAULT_UNSUPPORTED_SYSCALL | n as u32,
                    };
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(0),
                        local_diagnostics: diag,
                        fault: Some(FaultKind::Guest(code)),
                        syscall_args: None,
                    };
                }
                ExecuteVerdict::MemFault(ea) => {
                    // Same rollback policy as Fault above: capture diag
                    // first, then roll back state if mid-batch, propagate
                    // the fault directly.
                    let diag = self.fault_diag_ea(step_pc, ea);
                    if remaining < max_budget {
                        if let Some(snap) = snapshot.as_ref() {
                            self.state = snap.clone();
                            self.store_buf.clear();
                        }
                    } else {
                        self.store_buf.flush(effects, self.id);
                    }
                    effects.clear();
                    self.status = UnitStatus::Faulted;
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Fault,
                        consumed_budget: Budget::new(0),
                        local_diagnostics: diag,
                        fault: Some(FaultKind::Guest(FAULT_INVALID_ADDRESS)),
                        syscall_args: None,
                    };
                }
                ExecuteVerdict::BufferFull => {
                    // Store buffer capacity exceeded. Flush buffered
                    // stores and yield without advancing PC so the
                    // overflowing instruction retries on the next step.
                    self.store_buf.flush(effects, self.id);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::BudgetExhausted,
                        consumed_budget: Budget::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
            }

            if self.profile_mode {
                let pc_usize = step_pc as usize;
                if pc_usize + 4 <= mem.len() {
                    let raw = u32::from_be_bytes([
                        mem[pc_usize],
                        mem[pc_usize + 1],
                        mem[pc_usize + 2],
                        mem[pc_usize + 3],
                    ]);
                    if let Ok(raw_insn) = decode::decode(raw) {
                        let name = raw_insn.variant_name();
                        *self.profile_insns.entry(name).or_insert(0) += 1;
                        if let Some(prev) = self.profile_prev {
                            *self.profile_pairs.entry((prev, name)).or_insert(0) += 1;
                        }
                        self.profile_prev = Some(name);
                    }
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
                self.store_buf.flush(effects, self.id);
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

    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        let mut v: Vec<_> = self.profile_insns.iter().map(|(&k, &v)| (k, v)).collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        let mut v: Vec<_> = self.profile_pairs.iter().map(|(&k, &v)| (k, v)).collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        if let Some(s) = self.instruction_shadow.as_mut() {
            s.invalidate_range(addr, len);
        }
    }

    fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
    }
}

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;
