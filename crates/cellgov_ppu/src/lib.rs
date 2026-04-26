//! PPU `ExecutionUnit`: fetch-decode-execute loop, emitting `Effect`s
//! for all guest-visible writes. Instruction semantics live in
//! `exec`; decoding in `decode`.

pub mod decode;
pub mod exec;
mod fp;
pub mod instruction;
pub mod loader;
pub mod nid_db;
pub mod prx;
pub mod shadow;
pub mod sprx;
pub mod state;
pub mod store_buffer;

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
/// Syscall number has no handler.
pub const FAULT_UNSUPPORTED_SYSCALL: u32 = 0x0107_0000;
/// Debug breakpoint fired at a user-requested PC.
pub const FAULT_DEBUG_BREAK: u32 = 0x0108_0000;
/// Decoded instruction (typically a VMX sub-opcode) had no exec arm.
pub const FAULT_UNIMPLEMENTED_INSN: u32 = 0x0109_0000;

/// Whether `insn` is a 2-instruction super-pair fusion. The shadow
/// build emits a `Consumed` placeholder at the +4 slot for these
/// variants; quickenings (1-instruction rewrites like `Mr`, `Slwi`)
/// don't.
fn is_super_pair(insn: &instruction::PpuInstruction) -> bool {
    matches!(
        insn,
        instruction::PpuInstruction::LwzCmpwi { .. }
            | instruction::PpuInstruction::LiStw { .. }
            | instruction::PpuInstruction::MflrStw { .. }
            | instruction::PpuInstruction::LwzMtlr { .. }
            | instruction::PpuInstruction::MflrStd { .. }
            | instruction::PpuInstruction::LdMtlr { .. }
            | instruction::PpuInstruction::StdStd { .. }
            | instruction::PpuInstruction::CmpwiBc { .. }
            | instruction::PpuInstruction::CmpwBc { .. }
    )
}

/// PPU architectural state snapshot for replay.
#[derive(Debug, Clone)]
pub struct PpuSnapshot {
    /// General-purpose registers.
    pub gpr: [u64; 32],
    /// Floating-point registers (raw f64 bit patterns, matching `PpuState`).
    pub fpr: [u64; 32],
    /// Vector registers, big-endian (byte 0 in MSB).
    pub vr: [u128; 32],
    /// Program counter.
    pub pc: u64,
    /// Condition register.
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    /// Time base counter.
    pub tb: u64,
    /// Canonical reservation-line address, or `None` when no reservation is held.
    pub reservation_line: Option<u64>,
}

/// PPU `ExecutionUnit`: owns architectural state, fetches and executes
/// instructions, emits `Effect`s for stores and syscalls.
pub struct PpuExecutionUnit {
    id: UnitId,
    state: state::PpuState,
    status: UnitStatus,
    /// Fires a synthetic `FAULT_DEBUG_BREAK` after `break_skip` prior hits.
    break_pc: Option<u64>,
    break_skip: u32,
    per_step_trace: bool,
    /// Drained by the runtime via `drain_retired_state_hashes`.
    per_step_hashes: Vec<(u64, u64)>,
    /// Inclusive `[lo, hi]` retirement-index window for full-state capture.
    full_state_window: Option<(u64, u64)>,
    /// Increments on successful retirement only; matches the per-step-hash gate.
    retirement_counter: u64,
    /// Drained by the runtime via `drain_retired_state_full`.
    per_step_full_states: Vec<(u64, [u64; 32], u64, u64, u64, u32)>,
    /// Predecoded shadow of the main text region; stale slots re-decode on miss.
    instruction_shadow: Option<shadow::PredecodedShadow>,
    /// Sustained miss growth at stable PCs signals code outside the shadowed region.
    shadow_hits: u64,
    shadow_misses: u64,
    /// Flushed to `effects` at every yield/fault/budget-exhaustion point.
    store_buf: StoreBuffer,
    /// Forces `Budget=1` on the next `run_until_yield` for post-fault single-step replay.
    budget_override: Option<u64>,
    profile_mode: bool,
    profile_insns: std::collections::BTreeMap<&'static str, u64>,
    profile_pairs: std::collections::BTreeMap<(&'static str, &'static str), u64>,
    profile_prev: Option<&'static str>,
}

impl PpuExecutionUnit {
    /// Create a unit with zeroed architectural state.
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

    /// Enable per-step `(pc, state_hash)` tracing.
    pub fn set_per_step_trace(&mut self, on: bool) {
        self.per_step_trace = on;
    }

    /// Returns whether per-step state-hash tracing is enabled.
    pub fn per_step_trace(&self) -> bool {
        self.per_step_trace
    }

    /// Configure the inclusive retirement-index window for full-register snapshots.
    pub fn set_full_state_window(&mut self, window: Option<(u64, u64)>) {
        self.full_state_window = window;
    }

    /// Returns the current full-state capture window, if any.
    pub fn full_state_window(&self) -> Option<(u64, u64)> {
        self.full_state_window
    }

    /// Set a counted debug breakpoint: skip `skip` hits at `pc`, then fault.
    pub fn set_break_pc(&mut self, pc: u64, skip: u32) {
        self.break_pc = Some(pc);
        self.break_skip = skip;
    }

    /// Enable instruction-frequency profiling.
    pub fn set_profile_mode(&mut self, on: bool) {
        self.profile_mode = on;
    }

    /// Accumulated instruction-variant frequencies.
    pub fn profile_insns(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.profile_insns
    }

    /// Accumulated adjacent-pair frequencies.
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

    /// Attach a predecoded shadow built from the same memory the PPU will fetch.
    ///
    /// The shadow must be built after all boot-time code writes (ELF/PRX load,
    /// HLE stub planting) and before the step loop begins.
    pub fn set_instruction_shadow(&mut self, shadow: shadow::PredecodedShadow) {
        self.instruction_shadow = Some(shadow);
    }

    /// Returns `(hits, misses)` counters; high miss ratios signal code
    /// executing outside the shadowed region (correctness preserved, fast path lost).
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

        // Mirror cross-unit reservation clears from the committed table.
        // The ctx view is frozen for this step, so one read suffices.
        if self.state.reservation.is_some() && !ctx.reservation_held(self.id) {
            self.state.reservation = None;
        }

        // Resync the TB register to the global tick counter's
        // current-step value. `max` preserves strict monotonicity
        // across steps in the case where a prior step retired more
        // mftb reads than the inter-step tick-derived delta.
        let tb_from_tick = cellgov_time::ticks_to_tb(ctx.current_tick().raw());
        if tb_from_tick > self.state.tb {
            self.state.tb = tb_from_tick;
        }

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
        // Stack-allocated region table avoids per-call heap alloc on the
        // Budget=1 hot path (one call per retired instruction). Code
        // always lives in the base-0 region (`mem`); this table serves
        // loads/stores against auxiliary regions (stack, rsx, ...).
        const MAX_REGIONS: usize = 8;
        let mut region_views_storage: [(u64, &[u8]); MAX_REGIONS] =
            [(0, &[] as &[u8]); MAX_REGIONS];
        let mut n_regions = 0usize;
        for r in ctx.memory().regions() {
            assert!(
                n_regions < MAX_REGIONS,
                "region_views table too small; bump MAX_REGIONS"
            );
            region_views_storage[n_regions] = (r.base(), r.bytes());
            n_regions += 1;
        }
        let region_views = &region_views_storage[..n_regions];

        loop {
            let step_pc = self.state.pc;

            // Shadow is O(1); fallback decodes from raw memory.
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

            // Placeholder left by superinstruction pairing: advance PC, burn a tick.
            // The slot represents the second architectural instruction of a fused
            // super-pair, so it retires for trace and counter purposes too -- otherwise
            // retirement_counter and consumed_budget drift apart on every super-pair.
            if matches!(insn, instruction::PpuInstruction::Consumed) {
                self.state.pc += 4;
                if self.per_step_trace {
                    self.per_step_hashes
                        .push((step_pc, self.state.state_hash()));
                }
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
                continue;
            }

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
                    // Super-pair variants must be followed by a
                    // `Consumed` placeholder; otherwise the second
                    // half of the original pair would re-execute.
                    // Contract is locked from the outside by the
                    // `fetch_loop_advances_past_consumed_after_super_pair`
                    // and `fetch_loop_super_pair_taken_branch_uses_target_not_consumed_skip`
                    // tests in tests/ppu_tests.rs.
                    debug_assert!(
                        !is_super_pair(&insn)
                            || self
                                .instruction_shadow
                                .as_ref()
                                .and_then(|s| s.get(self.state.pc))
                                .is_none_or(|next| matches!(
                                    next,
                                    instruction::PpuInstruction::Consumed
                                )),
                        "super-pair {} at 0x{step_pc:x} not followed by Consumed at 0x{:x}",
                        insn.variant_name(),
                        self.state.pc,
                    );
                }
                ExecuteVerdict::Branch => {}
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
                    // Capture diag before rollback so registers reflect the fault site.
                    // Address-bearing variants route the address through diag.faulting_ea
                    // so the 16-bit low slot of the fault code stays clean of upper bits.
                    let diag = match f {
                        PpuFault::InvalidAddress(a) => self.fault_diag_ea(step_pc, a),
                        PpuFault::PcOutOfRange(a) => self.fault_diag_ea(step_pc, a),
                        _ => self.fault_diag(step_pc),
                    };
                    // Fault-discards-all: mid-batch rollback keeps state consistent
                    // with the dropped effect batch.
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
                    // Syscall numbers (<= ~1024) and VMX sub-opcodes (<= 11 bits) fit
                    // in the low 16 bits; mask defensively so a stray high bit cannot
                    // corrupt the upper-16 fault category prefix.
                    let code = match f {
                        PpuFault::PcOutOfRange(_) => FAULT_PC_OUT_OF_RANGE,
                        PpuFault::InvalidAddress(_) => FAULT_INVALID_ADDRESS,
                        PpuFault::UnsupportedSyscall(n) => {
                            FAULT_UNSUPPORTED_SYSCALL | (n as u32 & 0xFFFF)
                        }
                        PpuFault::UnimplementedInstruction(xo) => {
                            FAULT_UNIMPLEMENTED_INSN | (xo as u32 & 0xFFFF)
                        }
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
                    // Same rollback policy as `Fault` above.
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
                    // Yield without advancing PC so the store retries next step.
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
                // Profile the variant the unit actually dispatched, including
                // quickenings (Mr, Slwi, ...) and super-pairs (LwzCmpwi, ...);
                // re-decoding the raw word would attribute fused/quickened work
                // back to its pre-rewrite encoding.
                let name = insn.variant_name();
                *self.profile_insns.entry(name).or_insert(0) += 1;
                if let Some(prev) = self.profile_prev {
                    *self.profile_pairs.entry((prev, name)).or_insert(0) += 1;
                }
                self.profile_prev = Some(name);
            }

            if self.per_step_trace {
                self.per_step_hashes
                    .push((step_pc, self.state.state_hash()));
            }

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
            fpr: self.state.fpr,
            vr: self.state.vr,
            pc: self.state.pc,
            cr: self.state.cr,
            lr: self.state.lr,
            ctr: self.state.ctr,
            xer: self.state.xer,
            tb: self.state.tb,
            reservation_line: self.state.reservation.map(|l| l.addr()),
        }
    }

    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        std::mem::take(&mut self.per_step_full_states)
    }

    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        let map = std::mem::take(&mut self.profile_insns);
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        let map = std::mem::take(&mut self.profile_pairs);
        let mut v: Vec<_> = map.into_iter().collect();
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
