//! Step drivers for `run-game` (diagnostic) and `bench-boot` (throughput).
//!
//! Both loops share [`classify_step_outcome`] as the single source of
//! verdict precedence.

use std::time::Instant;

use cellgov_core::{CommitError, CommitOutcome, Runtime, StepError};
use cellgov_lv2::GuestBlockReason;

use super::diag::{
    append_orphan_exit_info, fetch_raw_at, format_commit_fault, format_deadlock, format_fault,
    format_max_steps, format_process_exit, print_trace_line, ProcessExitInfo, TtyCapture,
};
use super::manifest;

pub(super) const PC_RING_SIZE: usize = 64;
pub(super) const SYSCALL_RING_SIZE: usize = 32;

/// Bounded ring-buffer write cursor.
///
/// Invariant: `pos` is always in `[0, capacity)`; `full` flips on first wrap.
#[derive(Debug, Clone, Copy)]
pub(super) struct RingCursor {
    pos: usize,
    full: bool,
    capacity: usize,
}

impl RingCursor {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            pos: 0,
            full: false,
            capacity,
        }
    }

    pub(super) fn record(&mut self) -> usize {
        let idx = self.pos;
        self.pos += 1;
        if self.pos >= self.capacity {
            self.pos = 0;
            self.full = true;
        }
        idx
    }

    pub(super) fn filled(&self) -> usize {
        if self.full {
            self.capacity
        } else {
            self.pos
        }
    }

    #[allow(dead_code)]
    pub(super) fn is_full(&self) -> bool {
        self.full
    }

    /// Populated indices oldest-to-newest.
    pub(super) fn iter_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let (a_start, a_end, b_start, b_end) = if self.full {
            (self.pos, self.capacity, 0, self.pos)
        } else {
            (0, self.pos, 0, 0)
        };
        (a_start..a_end).chain(b_start..b_end)
    }
}

#[derive(Default)]
pub(super) struct StepTiming {
    pub(super) step_time: std::time::Duration,
    pub(super) commit_time: std::time::Duration,
    pub(super) coverage_time: std::time::Duration,
}

pub(super) struct StepLoopCtx<'a> {
    pub(super) steps: &'a mut usize,
    pub(super) distinct_pcs: &'a mut std::collections::BTreeSet<u64>,
    pub(super) hle_calls: &'a mut std::collections::BTreeMap<u32, usize>,
    pub(super) insn_coverage: &'a mut std::collections::BTreeMap<&'static str, usize>,
    pub(super) hle_bindings: &'a [cellgov_ppu::prx::HleBinding],
    pub(super) trace: bool,
    pub(super) timing: &'a mut Option<StepTiming>,
    pub(super) loop_start: Instant,
    pub(super) pc_ring: [u64; PC_RING_SIZE],
    pub(super) pc_ring_cursor: RingCursor,
    pub(super) last_tty: Option<TtyCapture>,
    pub(super) last_exit: Option<ProcessExitInfo>,
    pub(super) syscall_ring: [(u64, u64); SYSCALL_RING_SIZE],
    pub(super) syscall_ring_cursor: RingCursor,
    /// Top entries identify busy-loop bodies on max-steps.
    pub(super) pc_hits: &'a mut std::collections::HashMap<u64, u64>,
    pub(super) checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls dropped because `buf + len` exceeded mapped memory.
    pub(super) tty_oob_count: usize,
    /// `sys_tty_write` calls whose fd exceeded `u32::MAX` (narrowed to sentinel).
    pub(super) bogus_fd_count: usize,
}

/// Returns the RSX-region write address when a `ReservedWrite("rsx")`
/// commit error matches a [`manifest::CheckpointTrigger::FirstRsxWrite`]
/// trigger; `None` otherwise.
pub(super) fn rsx_write_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    commit_result: &Result<CommitOutcome, CommitError>,
) -> Option<u64> {
    if trigger != manifest::CheckpointTrigger::FirstRsxWrite {
        return None;
    }
    if let Err(cellgov_core::CommitError::Memory(cellgov_mem::MemError::ReservedWrite {
        addr,
        region: "rsx",
    })) = commit_result
    {
        Some(*addr)
    } else {
        None
    }
}

/// Per-step verdict shared by both loops.
///
/// Precedence (high to low):
/// 1. `CommitFault` -- non-checkpoint commit error.
/// 2. `StepFault` -- `YieldReason::Fault` raised by the unit, batch discarded.
/// 3. `RsxCheckpoint(addr)` -- `ReservedWrite("rsx")` under a `FirstRsxWrite` trigger.
/// 4. `PcReached(addr)` -- step retired the caller-supplied PC.
/// 5. `Continue`.
///
/// A callback-worker fault absorbed by commit (`callback_worker_fault_absorbed`)
/// suppresses `StepFault` even when `step.fault.is_some()`, so the run can
/// resume after the absorbing module recovered.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum StepVerdict {
    Continue,
    CommitFault,
    StepFault,
    RsxCheckpoint(u64),
    PcReached(u64),
}

/// Apply [`StepVerdict`] precedence to a step + commit pair.
///
/// The relative ordering of `CommitFault` vs `StepFault` is forward-compatible
/// guard rail: under current `commit_step` semantics a `YieldReason::Fault`
/// returns `Ok(fault_discarded)`, so they cannot co-occur today.
pub(super) fn classify_step_outcome(
    step: &cellgov_exec::ExecutionStepResult,
    commit_result: &Result<CommitOutcome, CommitError>,
    checkpoint: manifest::CheckpointTrigger,
    target_pc: Option<u64>,
) -> StepVerdict {
    let checkpoint_addr = rsx_write_checkpoint_addr(checkpoint, commit_result);
    if commit_result.is_err() && checkpoint_addr.is_none() {
        return StepVerdict::CommitFault;
    }
    let callback_fault_absorbed = matches!(
        commit_result,
        Ok(o) if o.callback_worker_fault_absorbed
    );
    if step.fault.is_some() && !callback_fault_absorbed {
        return StepVerdict::StepFault;
    }
    if let Some(addr) = checkpoint_addr {
        return StepVerdict::RsxCheckpoint(addr);
    }
    if let Some(target) = target_pc {
        if step.local_diagnostics.pc == Some(target) {
            return StepVerdict::PcReached(target);
        }
    }
    StepVerdict::Continue
}

/// Pure decision over `(args, mem_bytes)` -- no I/O, no counter mutation.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TtyCaptureDecision {
    /// Buffer fits in mapped memory, or `len == 0` (buf not dereferenced).
    InBounds {
        fd: u32,
        fd_was_bogus: bool,
        bytes: Vec<u8>,
    },
    Oob {
        buf: usize,
        len: usize,
        mem_len: usize,
    },
}

/// Classify a `sys_tty_write` call without touching runtime state.
///
/// Bytes are captured at full fidelity; display layers bound output width.
pub(super) fn classify_tty_capture(args: &[u64; 9], mem_bytes: &[u8]) -> TtyCaptureDecision {
    let buf = args[2] as usize;
    let len = args[3] as usize;
    // Narrow oversized fd to a sentinel rather than aliasing to a low fd.
    let (fd, fd_was_bogus) = match u32::try_from(args[1]) {
        Ok(fd) => (fd, false),
        Err(_) => (u32::MAX, true),
    };
    if len == 0 {
        return TtyCaptureDecision::InBounds {
            fd,
            fd_was_bogus,
            bytes: Vec::new(),
        };
    }
    // checked_add guards a guest `buf` near `usize::MAX`.
    let end = buf.checked_add(len);
    if end.is_none_or(|e| e > mem_bytes.len()) {
        return TtyCaptureDecision::Oob {
            buf,
            len,
            mem_len: mem_bytes.len(),
        };
    }
    let bytes = mem_bytes[buf..buf + len].to_vec();
    TtyCaptureDecision::InBounds {
        fd,
        fd_was_bogus,
        bytes,
    }
}

/// Untracked time = `t_loop - (step + commit + coverage)`.
///
/// # Errors
///
/// Returns `Err(excess)` when tracked buckets exceed `t_loop` -- bucket
/// overlap, double-counting, or non-monotonic clock.
pub(super) fn compute_untracked(
    t_loop: std::time::Duration,
    step: std::time::Duration,
    commit: std::time::Duration,
    coverage: std::time::Duration,
) -> Result<std::time::Duration, std::time::Duration> {
    let tracked = step
        .checked_add(commit)
        .and_then(|s| s.checked_add(coverage))
        .unwrap_or(std::time::Duration::MAX);
    if tracked <= t_loop {
        Ok(t_loop - tracked)
    } else {
        Err(tracked - t_loop)
    }
}

pub(super) fn pct(part: std::time::Duration, total: std::time::Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        100.0 * part.as_secs_f64() / total.as_secs_f64()
    }
}

pub(super) fn step_loop(
    rt: &mut Runtime,
    ctx: &mut StepLoopCtx<'_>,
) -> (String, cellgov_compare::BootOutcome) {
    use cellgov_compare::BootOutcome;
    loop {
        let t0 = Instant::now();
        let step_result = rt.step();
        let t1 = Instant::now();

        match step_result {
            Ok(step) => {
                *ctx.steps += 1;

                // PC ring and distinct-PC set track attempted execution,
                // so they advance before commit (kept on a discarded batch).
                if let Some(pc) = step.result.local_diagnostics.pc {
                    ctx.distinct_pcs.insert(pc);
                    let idx = ctx.pc_ring_cursor.record();
                    ctx.pc_ring[idx] = pc;
                }

                if (*ctx.steps).is_multiple_of(10_000) {
                    let elapsed = ctx.loop_start.elapsed();
                    println!(
                        "  [{:>6}] {:.1?} elapsed, {} distinct PCs, {} HLE calls",
                        ctx.steps,
                        elapsed,
                        ctx.distinct_pcs.len(),
                        ctx.hle_calls.values().sum::<usize>(),
                    );
                }

                if ctx.trace {
                    print_trace_line(rt, step.unit, &step.result, *ctx.steps, ctx.hle_bindings);
                }

                let t2 = Instant::now();
                let commit_result = rt.commit_step(&step.result, &step.effects);
                let t3 = Instant::now();

                match classify_step_outcome(&step.result, &commit_result, ctx.checkpoint, None) {
                    StepVerdict::RsxCheckpoint(addr) => {
                        break (
                            format!(
                                "RSX_WRITE_CHECKPOINT at 0x{addr:x} after {} steps",
                                ctx.steps
                            ),
                            BootOutcome::RsxWriteCheckpoint,
                        );
                    }
                    StepVerdict::CommitFault => {
                        let err = commit_result
                            .as_ref()
                            .expect_err("classified as CommitFault implies Err");
                        let mut diag = format_commit_fault(
                            rt,
                            err,
                            *ctx.steps,
                            &ctx.pc_ring,
                            &ctx.pc_ring_cursor,
                        );
                        append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                        break (diag, BootOutcome::Fault);
                    }
                    StepVerdict::StepFault => {
                        let fault = step
                            .result
                            .fault
                            .as_ref()
                            .expect("classified as StepFault implies Some");
                        let mut diag = format_fault(
                            rt,
                            &step.result,
                            fault,
                            *ctx.steps,
                            &ctx.pc_ring,
                            &ctx.pc_ring_cursor,
                        );
                        append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                        break (diag, BootOutcome::Fault);
                    }
                    StepVerdict::PcReached(_) => {
                        unreachable!("step_loop never sets target_pc")
                    }
                    StepVerdict::Continue => {}
                }

                // Post-commit: pc_hits, coverage, syscall ring, last_tty,
                // last_exit only advance when the batch was applied (a
                // discarded batch leaves guest-visible state untouched).
                if let Some(pc) = step.result.local_diagnostics.pc {
                    *ctx.pc_hits.entry(pc).or_insert(0) += 1;
                }

                let t_cov_start = Instant::now();
                if let Some(pc) = step.result.local_diagnostics.pc {
                    if let Some(raw) = fetch_raw_at(rt, pc) {
                        let name = match cellgov_ppu::decode::decode(raw) {
                            Ok(insn) => insn.variant_name(),
                            Err(_) => "DECODE_ERROR",
                        };
                        *ctx.insn_coverage.entry(name).or_insert(0) += 1;
                    }
                }
                let t_cov_end = Instant::now();

                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
                        if let Some(binding) = ctx.hle_bindings.get(idx as usize) {
                            if binding.nid == cellgov_ps3_abi::nid::sys_prx_for_user::PROCESS_EXIT
                                || binding.nid
                                    == cellgov_ps3_abi::nid::sys_prx_for_user::PPU_THREAD_EXIT
                                || binding.nid
                                    == cellgov_ps3_abi::nid::sys_prx_for_user::PRX_EXITSPAWN_WITH_LEVEL
                            {
                                ctx.last_exit = Some(ProcessExitInfo {
                                    code: args[1] as u32,
                                    call_pc: pc,
                                });
                            }
                        }
                    } else if args[0] == cellgov_ps3_abi::syscall::TTY_WRITE {
                        match classify_tty_capture(args, rt.memory().as_bytes()) {
                            TtyCaptureDecision::InBounds {
                                fd,
                                fd_was_bogus,
                                bytes,
                            } => {
                                if fd_was_bogus {
                                    ctx.bogus_fd_count += 1;
                                }
                                let bogus_marker = if fd_was_bogus {
                                    " (bogus, narrowed)"
                                } else {
                                    ""
                                };
                                // ASCII-safe to survive cp1252/cp437 consoles.
                                let preview = super::diag::ascii_safe_preview(&bytes);
                                print!("  tty[fd={fd}{bogus_marker}]: {preview}");
                                if !preview.ends_with('\n') {
                                    println!();
                                }
                                // Flush stdout so a stderr fault stack does
                                // not interleave ahead of the tty line.
                                use std::io::Write;
                                let _ = std::io::stdout().flush();
                                ctx.last_tty = Some(TtyCapture {
                                    fd,
                                    raw_bytes: bytes,
                                    call_pc: pc,
                                });
                            }
                            TtyCaptureDecision::Oob { buf, len, mem_len } => {
                                ctx.tty_oob_count += 1;
                                eprintln!(
                                    "  tty_oob: sys_tty_write buf=0x{buf:x}+0x{len:x} exceeds guest memory (0x{mem_len:x}); capture dropped at step {}",
                                    *ctx.steps
                                );
                            }
                        }
                    } else if args[0] == cellgov_ps3_abi::syscall::PROCESS_EXIT
                        || args[0] == cellgov_ps3_abi::syscall::PPU_THREAD_EXIT
                    {
                        ctx.last_exit = Some(ProcessExitInfo {
                            code: args[1] as u32,
                            call_pc: pc,
                        });
                    }
                    let sc_idx = ctx.syscall_ring_cursor.record();
                    ctx.syscall_ring[sc_idx] = (args[0], pc);
                }

                if let Some(t) = ctx.timing.as_mut() {
                    t.step_time += t1 - t0;
                    t.commit_time += t3 - t2;
                    t.coverage_time += t_cov_end - t_cov_start;
                    // Why this can't fire: monotonic clock + disjoint regions
                    // imply tracked <= loop_start.elapsed(). Triggers only on
                    // bucket overlap or a non-monotonic clock.
                    debug_assert!(
                        compute_untracked(
                            ctx.loop_start.elapsed(),
                            t.step_time,
                            t.commit_time,
                            t.coverage_time,
                        )
                        .is_ok(),
                        "tracked timing buckets exceed loop total -- bucket overlap or non-monotonic clock"
                    );
                }
            }
            Err(StepError::NoRunnableUnit) => {
                if let Some(ref exit) = ctx.last_exit {
                    break (
                        format_process_exit(
                            exit,
                            ctx.last_tty.as_ref(),
                            *ctx.steps,
                            &ctx.pc_ring,
                            &ctx.pc_ring_cursor,
                            &ctx.syscall_ring,
                            &ctx.syscall_ring_cursor,
                            ctx.hle_bindings,
                        ),
                        BootOutcome::ProcessExit,
                    );
                }
                // Every unit drained without a sys_process_exit /
                // sys_ppu_thread_exit dispatch -- guest-undefined state.
                break (
                    format!(
                        "ALL_UNITS_FINISHED after {} steps without sys_process_exit",
                        ctx.steps
                    ),
                    BootOutcome::Fault,
                );
            }
            Err(StepError::AllBlocked) => {
                // Deadlock: at least one Blocked unit, none Runnable.
                let mut diag = format_deadlock(rt, *ctx.steps, &ctx.pc_ring, &ctx.pc_ring_cursor);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                break (diag, BootOutcome::Fault);
            }
            Err(StepError::MaxStepsExceeded) => {
                let mut diag = format_max_steps(
                    *ctx.steps,
                    &ctx.pc_ring,
                    &ctx.pc_ring_cursor,
                    &ctx.syscall_ring,
                    &ctx.syscall_ring_cursor,
                    ctx.hle_bindings,
                );
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                break (diag, BootOutcome::MaxSteps);
            }
            Err(StepError::TimeOverflow) => {
                let mut diag = format!("TIME_OVERFLOW after {} steps", ctx.steps);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                break (diag, BootOutcome::TimeOverflow);
            }
        }
    }
}

/// Throughput driver: tracks termination only, no diagnostic state.
///
/// `CommitFault` and `StepFault` both surface as `BootOutcome::Fault`;
/// `NoRunnableUnit` and `AllBlocked` both surface as `ProcessExit`.
pub(super) fn bench_step_loop(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
) -> cellgov_compare::BootOutcome {
    use cellgov_compare::BootOutcome;
    use manifest::CheckpointTrigger;
    let target_pc = match checkpoint {
        CheckpointTrigger::Pc(addr) => Some(addr),
        _ => None,
    };
    loop {
        match rt.step() {
            Ok(step) => {
                *steps += 1;
                let commit_result = rt.commit_step(&step.result, &step.effects);
                match classify_step_outcome(&step.result, &commit_result, checkpoint, target_pc) {
                    StepVerdict::Continue => {}
                    StepVerdict::RsxCheckpoint(_) => return BootOutcome::RsxWriteCheckpoint,
                    StepVerdict::CommitFault | StepVerdict::StepFault => return BootOutcome::Fault,
                    StepVerdict::PcReached(addr) => return BootOutcome::PcReached(addr),
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                return BootOutcome::ProcessExit;
            }
            Err(StepError::MaxStepsExceeded) => return BootOutcome::MaxSteps,
            Err(StepError::TimeOverflow) => return BootOutcome::TimeOverflow,
        }
    }
}

/// One-line label per [`GuestBlockReason`] for deadlock dumps.
///
/// Match is exhaustive so a new variant fails compilation rather than
/// silently rendering via `Debug`.
pub(super) fn block_reason_label(reason: &GuestBlockReason) -> String {
    match reason {
        GuestBlockReason::WaitingOnJoin { target } => {
            format!("WaitingOnJoin(target={})", target.raw())
        }
        GuestBlockReason::WaitingOnLwMutex { id } => format!("WaitingOnLwMutex(id={id})"),
        GuestBlockReason::WaitingOnMutex { id } => format!("WaitingOnMutex(id={id})"),
        GuestBlockReason::WaitingOnSemaphore { id } => format!("WaitingOnSemaphore(id={id})"),
        GuestBlockReason::WaitingOnEventQueue { id } => format!("WaitingOnEventQueue(id={id})"),
        GuestBlockReason::WaitingOnEventFlag { id, mask, mode } => {
            format!("WaitingOnEventFlag(id={id}, mask=0x{mask:x}, mode={mode:?})")
        }
        GuestBlockReason::WaitingOnCond { cond_id, mutex_id } => {
            format!("WaitingOnCond(cond={cond_id}, mutex={mutex_id})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_core::{CommitError, CommitOutcome};
    use cellgov_exec::ExecutionStepResult;
    use cellgov_mem::MemError;
    use manifest::CheckpointTrigger;
    use std::time::Duration;

    fn ok_commit() -> Result<CommitOutcome, CommitError> {
        Ok(CommitOutcome::default())
    }

    fn rsx_checkpoint_err() -> Result<CommitOutcome, CommitError> {
        Err(CommitError::Memory(MemError::ReservedWrite {
            addr: 0xC000_0040,
            region: "rsx",
        }))
    }

    fn other_commit_err() -> Result<CommitOutcome, CommitError> {
        Err(CommitError::PayloadLengthMismatch { effect_index: 0 })
    }

    fn ok_step() -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::BudgetExhausted,
            consumed_cost: cellgov_time::InstructionCost::ZERO,
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn faulted_step() -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: cellgov_exec::YieldReason::Fault,
            consumed_cost: cellgov_time::InstructionCost::ZERO,
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: Some(cellgov_effects::FaultKind::Guest(
                cellgov_ppu::FAULT_DECODE_ERROR,
            )),
            syscall_args: None,
        }
    }

    #[test]
    fn untracked_is_loop_minus_tracked_sum_in_happy_path() {
        let t_loop = Duration::from_millis(100);
        let step = Duration::from_millis(40);
        let commit = Duration::from_millis(20);
        let coverage = Duration::from_millis(10);
        assert_eq!(
            compute_untracked(t_loop, step, commit, coverage),
            Ok(Duration::from_millis(30))
        );
    }

    #[test]
    fn untracked_zero_when_buckets_fill_the_loop() {
        let t_loop = Duration::from_millis(100);
        let step = Duration::from_millis(60);
        let commit = Duration::from_millis(30);
        let coverage = Duration::from_millis(10);
        assert_eq!(
            compute_untracked(t_loop, step, commit, coverage),
            Ok(Duration::ZERO)
        );
    }

    #[test]
    fn untracked_errors_when_tracked_exceeds_loop() {
        let t_loop = Duration::from_millis(100);
        let step = Duration::from_millis(60);
        let commit = Duration::from_millis(30);
        let coverage = Duration::from_millis(25);
        assert_eq!(
            compute_untracked(t_loop, step, commit, coverage),
            Err(Duration::from_millis(15))
        );
    }

    #[test]
    fn untracked_handles_zero_loop_cleanly() {
        assert_eq!(
            compute_untracked(
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO
            ),
            Ok(Duration::ZERO)
        );
    }

    #[test]
    fn untracked_saturates_on_arithmetic_overflow() {
        let result = compute_untracked(
            Duration::from_millis(100),
            Duration::MAX,
            Duration::from_millis(1),
            Duration::from_millis(1),
        );
        assert!(result.is_err());
    }

    #[test]
    fn rsx_checkpoint_fires_on_reserved_write_to_rsx() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &rsx_checkpoint_err()),
            Some(0xC000_0040)
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_other_reserved_regions() {
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xE000_0000,
                region: "spu_reserved",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_inert_for_process_exit_trigger() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::ProcessExit, &rsx_checkpoint_err()),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_successful_commit() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &ok_commit()),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_non_memory_commit_errors() {
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &other_commit_err()),
            None
        );
    }

    // classify_step_outcome priority tree

    #[test]
    fn classify_continue_on_clean_step_and_commit() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                None
            ),
            StepVerdict::Continue
        );
    }

    #[test]
    fn classify_commit_fault_when_non_checkpoint_err() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &other_commit_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_commit_fault_wins_over_step_fault() {
        assert_eq!(
            classify_step_outcome(
                &faulted_step(),
                &other_commit_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_step_fault_wins_over_pc_reached() {
        let mut s = faulted_step();
        s.local_diagnostics.pc = Some(0x1000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x1000),
            ),
            StepVerdict::StepFault
        );
    }

    #[test]
    fn classify_rsx_checkpoint_fires_under_first_rsx_write() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &rsx_checkpoint_err(),
                CheckpointTrigger::FirstRsxWrite,
                None,
            ),
            StepVerdict::RsxCheckpoint(0xC000_0040)
        );
    }

    #[test]
    fn classify_rsx_checkpoint_demotes_to_commit_fault_when_trigger_mismatched() {
        assert_eq!(
            classify_step_outcome(
                &ok_step(),
                &rsx_checkpoint_err(),
                CheckpointTrigger::ProcessExit,
                None,
            ),
            StepVerdict::CommitFault
        );
    }

    #[test]
    fn classify_pc_reached_only_when_no_fault_and_no_checkpoint() {
        let mut s = ok_step();
        s.local_diagnostics.pc = Some(0x4000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x4000),
            ),
            StepVerdict::PcReached(0x4000)
        );
    }

    #[test]
    fn classify_pc_target_mismatch_continues() {
        let mut s = ok_step();
        s.local_diagnostics.pc = Some(0x4000);
        assert_eq!(
            classify_step_outcome(
                &s,
                &ok_commit(),
                CheckpointTrigger::ProcessExit,
                Some(0x5000),
            ),
            StepVerdict::Continue
        );
    }

    // RingCursor

    #[test]
    fn ring_cursor_records_in_order_until_full() {
        let mut c = RingCursor::new(4);
        assert_eq!(c.record(), 0);
        assert_eq!(c.record(), 1);
        assert_eq!(c.filled(), 2);
        assert!(!c.is_full());
    }

    #[test]
    fn ring_cursor_wraps_and_marks_full() {
        let mut c = RingCursor::new(3);
        c.record();
        c.record();
        c.record();
        assert!(c.is_full());
        assert_eq!(c.filled(), 3);
        assert_eq!(c.record(), 0);
        assert_eq!(c.filled(), 3);
        assert!(c.is_full());
    }

    #[test]
    fn ring_cursor_iter_indices_partial_yields_in_order() {
        let mut c = RingCursor::new(4);
        c.record();
        c.record();
        let v: Vec<_> = c.iter_indices().collect();
        assert_eq!(v, vec![0, 1]);
    }

    #[test]
    fn ring_cursor_iter_indices_full_yields_oldest_first() {
        let mut c = RingCursor::new(3);
        for _ in 0..5 {
            c.record();
        }
        // pos = 5 % 3 = 2, full; oldest sits at the next write slot.
        let v: Vec<_> = c.iter_indices().collect();
        assert_eq!(v, vec![2, 0, 1]);
    }

    #[test]
    fn ring_cursor_iter_indices_empty_yields_nothing() {
        let c = RingCursor::new(4);
        assert_eq!(c.iter_indices().count(), 0);
    }

    fn tty_args(fd: u64, buf: u64, len: u64) -> [u64; 9] {
        [403, fd, buf, len, 0, 0, 0, 0, 0]
    }

    #[test]
    fn classify_tty_capture_happy_path_returns_bytes_and_small_fd() {
        let mem = b"hello\0padding".to_vec();
        let args = tty_args(1, 0, 5);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: b"hello".to_vec(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_narrows_wide_fd_and_flags_bogus() {
        let mem = b"ok".to_vec();
        let args = tty_args(u64::from(u32::MAX) + 1, 0, 2);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: u32::MAX,
                fd_was_bogus: true,
                bytes: b"ok".to_vec(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_flags_oob_when_end_exceeds_mem() {
        let mem = b"tiny!".to_vec();
        let args = tty_args(1, 0, 10);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::Oob {
                buf: 0,
                len: 10,
                mem_len: 5,
            }
        );
    }

    #[test]
    fn classify_tty_capture_flags_oob_on_checked_add_overflow() {
        let mem = vec![0u8; 16];
        let buf = usize::MAX as u64;
        let args = tty_args(1, buf, 8);
        let decision = classify_tty_capture(&args, &mem);
        assert!(
            matches!(decision, TtyCaptureDecision::Oob { .. }),
            "usize::MAX + 8 must classify as Oob, got {decision:?}"
        );
    }

    #[test]
    fn classify_tty_capture_keeps_full_buffer_above_4kib() {
        let mem = vec![b'x'; 8192];
        let args = tty_args(1, 0, 8000);
        let decision = classify_tty_capture(&args, &mem);
        match decision {
            TtyCaptureDecision::InBounds {
                fd,
                fd_was_bogus,
                bytes,
            } => {
                assert_eq!(fd, 1);
                assert!(!fd_was_bogus);
                assert_eq!(bytes.len(), 8000);
            }
            other => panic!("expected InBounds, got {other:?}"),
        }
    }

    #[test]
    fn classify_tty_capture_zero_len_with_garbage_buf_is_inbounds() {
        let mem = b"only-16-bytes!!!".to_vec();
        let args = tty_args(1, 0xDEAD_BEEF, 0);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: Vec::new(),
            }
        );
    }

    #[test]
    fn classify_tty_capture_zero_len_at_mem_end_is_inbounds() {
        // buf == mem.len(), len == 0: empty slice is valid.
        let mem = vec![0u8; 16];
        let args = tty_args(1, 16, 0);
        let decision = classify_tty_capture(&args, &mem);
        assert_eq!(
            decision,
            TtyCaptureDecision::InBounds {
                fd: 1,
                fd_was_bogus: false,
                bytes: Vec::new(),
            }
        );
    }

    #[test]
    fn block_reason_label_distinguishes_each_variant() {
        use cellgov_lv2::{EventFlagWaitMode, PpuThreadId};
        let labels = [
            block_reason_label(&GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            }),
            block_reason_label(&GuestBlockReason::WaitingOnLwMutex { id: 7 }),
            block_reason_label(&GuestBlockReason::WaitingOnMutex { id: 7 }),
            block_reason_label(&GuestBlockReason::WaitingOnSemaphore { id: 7 }),
            block_reason_label(&GuestBlockReason::WaitingOnEventQueue { id: 7 }),
            block_reason_label(&GuestBlockReason::WaitingOnEventFlag {
                id: 7,
                mask: 0xF0,
                mode: EventFlagWaitMode::AndClear,
            }),
            block_reason_label(&GuestBlockReason::WaitingOnCond {
                cond_id: 11,
                mutex_id: 13,
            }),
        ];
        let unique: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "label collision: {labels:?}",);
        // Resource id must reach the dump so the investigator can name
        // the hanging sync object.
        assert!(labels[1].contains("id=7"), "got {}", labels[1]);
        assert!(labels[5].contains("mask=0xf0"), "got {}", labels[5]);
        assert!(labels[6].contains("cond=11"), "got {}", labels[6]);
        assert!(labels[6].contains("mutex=13"), "got {}", labels[6]);
    }
}
