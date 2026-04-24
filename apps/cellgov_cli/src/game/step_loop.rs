//! Step-loop machinery for `run-game` and `bench-boot`.
//!
//! `step_loop` is the diagnostic driver (ring buffers, TTY capture,
//! progress checkpoints, timing breakdown); `bench_step_loop` is
//! the minimal throughput driver. Both route through
//! [`rsx_write_checkpoint_addr`] so the two loops cannot diverge
//! on what counts as a checkpoint hit.

use std::time::Instant;

use cellgov_core::{Runtime, StepError};

use super::diag::{
    fetch_raw_at, format_fault, format_max_steps, format_process_exit, print_trace_line,
    ProcessExitInfo, TtyCapture,
};
use super::manifest;

pub(super) const PC_RING_SIZE: usize = 64;
pub(super) const SYSCALL_RING_SIZE: usize = 32;

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
    /// Ring buffer of recent PCs for mini-trace on fault.
    pub(super) pc_ring: [u64; PC_RING_SIZE],
    pub(super) pc_ring_pos: usize,
    /// Last TTY write buffer (raw bytes) for diagnostic artifact.
    pub(super) last_tty: Option<TtyCapture>,
    /// Set when `sys_process_exit` is dispatched.
    pub(super) last_exit: Option<ProcessExitInfo>,
    /// Ring buffer of recent LV2 syscall numbers for exit diagnostic.
    pub(super) syscall_ring: [(u64, u64); SYSCALL_RING_SIZE],
    pub(super) syscall_ring_pos: usize,
    /// Per-PC hit counts. Top entries identify busy-loop bodies
    /// when the run hits max-steps.
    pub(super) pc_hits: &'a mut std::collections::HashMap<u64, u64>,
    /// The boot checkpoint the harness is looking for. Classifies
    /// a reserved-region write as a checkpoint reach vs. a fault;
    /// the commit pipeline discards either way.
    pub(super) checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls skipped because `buf + len` overflowed
    /// guest memory bounds.
    pub(super) tty_oob_count: usize,
    /// `sys_tty_write` calls whose `args[1]` fd did not fit in u32;
    /// narrowed to `u32::MAX` as a visible sentinel.
    pub(super) bogus_fd_count: usize,
}

/// Classify a `commit_step` outcome as an RSX-write checkpoint hit.
///
/// Returns the triggering guest address when the title's trigger is
/// [`manifest::CheckpointTrigger::FirstRsxWrite`] and the commit
/// failed with a `ReservedWrite` to the `"rsx"` region; `None`
/// otherwise. Both step loops route through this so their decisions
/// cannot drift.
pub(super) fn rsx_write_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    commit_result: &Result<cellgov_core::CommitOutcome, cellgov_core::CommitError>,
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

/// Decision for one `sys_tty_write` call. Pure over
/// `(args, mem_bytes)`; no I/O and no counter mutation.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TtyCaptureDecision {
    /// Buffer fits entirely in mapped memory.
    InBounds {
        fd: u32,
        fd_was_bogus: bool,
        bytes: Vec<u8>,
    },
    /// `buf + len` overflows mapped memory; raw values echoed back
    /// for the caller's log.
    Oob {
        buf: usize,
        len: usize,
        mem_len: usize,
    },
}

/// Classify a `sys_tty_write` guest call without touching runtime
/// state. `args` is the raw syscall-args array; `mem_bytes` is the
/// currently-committed guest memory slice. `len` is clamped to 4096
/// to match the PS3 cap.
pub(super) fn classify_tty_capture(args: &[u64; 9], mem_bytes: &[u8]) -> TtyCaptureDecision {
    let buf = args[2] as usize;
    let len = (args[3] as usize).min(4096);
    // checked_add: a guest `buf` near `usize::MAX` would wrap past
    // the `<= mem.len()` check under plain addition.
    let end = buf.checked_add(len);
    if end.is_none_or(|e| e > mem_bytes.len()) {
        return TtyCaptureDecision::Oob {
            buf,
            len,
            mem_len: mem_bytes.len(),
        };
    }
    // fd > u32::MAX surfaces as u32::MAX rather than aliasing to a
    // plausible low fd. `sys_tty_write` uses 0/1/2 in practice.
    let (fd, fd_was_bogus) = match u32::try_from(args[1]) {
        Ok(fd) => (fd, false),
        Err(_) => (u32::MAX, true),
    };
    let bytes = mem_bytes[buf..buf + len].to_vec();
    TtyCaptureDecision::InBounds {
        fd,
        fd_was_bogus,
        bytes,
    }
}

/// Compute untracked time inside the step loop.
///
/// # Errors
///
/// Returns `Err(excess)` when tracked buckets overflow `t_loop` --
/// a timing invariant violation (overlapping regions,
/// double-counting, or `t_loop` sampled after the loop began).
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

                if let Some(pc) = step.result.local_diagnostics.pc {
                    ctx.distinct_pcs.insert(pc);
                    ctx.pc_ring[ctx.pc_ring_pos % PC_RING_SIZE] = pc;
                    ctx.pc_ring_pos += 1;
                    *ctx.pc_hits.entry(pc).or_insert(0) += 1;
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

                if ctx.trace {
                    print_trace_line(rt, &step.result, *ctx.steps, ctx.hle_bindings);
                }
                // Capture HLE/LV2 calls, TTY, and sys_process_exit
                // before commit so the final report has them even
                // when commit then fails.
                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
                        // NID 0xe6f2c1e7 is sys_process_exit.
                        if let Some(binding) = ctx.hle_bindings.get(idx as usize) {
                            if binding.nid == 0xe6f2c1e7 {
                                ctx.last_exit = Some(ProcessExitInfo {
                                    code: args[1] as u32,
                                    call_pc: pc,
                                });
                            }
                        }
                    } else if args[0] == 403 {
                        // sys_tty_write
                        match classify_tty_capture(args, rt.memory().as_bytes()) {
                            TtyCaptureDecision::InBounds {
                                fd,
                                fd_was_bogus,
                                bytes,
                            } => {
                                if fd_was_bogus {
                                    ctx.bogus_fd_count += 1;
                                }
                                // ASCII-safe so binary payloads do
                                // not emit bytes a cp1252/cp437
                                // Windows console mangles.
                                let preview = super::diag::ascii_safe_preview(&bytes);
                                print!("  tty[fd={fd}]: {preview}");
                                if !preview.ends_with('\n') {
                                    println!();
                                }
                                // Flush so a subsequent fault stack
                                // on stderr does not land before the
                                // TTY line.
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
                    } else if args[0] == 22 {
                        // sys_process_exit
                        ctx.last_exit = Some(ProcessExitInfo {
                            code: args[1] as u32,
                            call_pc: pc,
                        });
                    }
                    ctx.syscall_ring[ctx.syscall_ring_pos % SYSCALL_RING_SIZE] = (args[0], pc);
                    ctx.syscall_ring_pos += 1;
                }

                let t2 = Instant::now();
                let commit_result = rt.commit_step(&step.result, &step.effects);
                let t3 = Instant::now();

                if let Some(addr) = rsx_write_checkpoint_addr(ctx.checkpoint, &commit_result) {
                    break (
                        format!(
                            "RSX_WRITE_CHECKPOINT at 0x{addr:x} after {} steps",
                            ctx.steps
                        ),
                        BootOutcome::RsxWriteCheckpoint,
                    );
                }

                if let Some(t) = ctx.timing.as_mut() {
                    t.step_time += t1 - t0;
                    t.commit_time += t3 - t2;
                    t.coverage_time += t_cov_end - t_cov_start;
                }

                if let Some(fault) = &step.result.fault {
                    break (
                        format_fault(
                            rt,
                            &step.result,
                            fault,
                            *ctx.steps,
                            &ctx.pc_ring,
                            ctx.pc_ring_pos,
                        ),
                        BootOutcome::Fault,
                    );
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                if let Some(ref exit) = ctx.last_exit {
                    break (
                        format_process_exit(
                            exit,
                            ctx.last_tty.as_ref(),
                            *ctx.steps,
                            &ctx.pc_ring,
                            ctx.pc_ring_pos,
                            &ctx.syscall_ring,
                            ctx.syscall_ring_pos,
                            ctx.hle_bindings,
                        ),
                        BootOutcome::ProcessExit,
                    );
                }
                break (
                    format!("STALL after {} steps", ctx.steps),
                    BootOutcome::Fault,
                );
            }
            Err(StepError::MaxStepsExceeded) => {
                break (
                    format_max_steps(
                        *ctx.steps,
                        &ctx.pc_ring,
                        ctx.pc_ring_pos,
                        &ctx.syscall_ring,
                        ctx.syscall_ring_pos,
                        ctx.hle_bindings,
                    ),
                    BootOutcome::MaxSteps,
                );
            }
            Err(StepError::TimeOverflow) => {
                break (
                    format!("TIME_OVERFLOW after {} steps", ctx.steps),
                    BootOutcome::TimeOverflow,
                );
            }
        }
    }
}

/// Minimal step loop: only the state needed to detect termination.
///
/// Termination classification:
/// - `ProcessExit` on `NoRunnableUnit`/`AllBlocked` (the
///   `sys_process_exit` path removes the primary unit).
/// - `RsxWriteCheckpoint` when `commit_step` returns `ReservedWrite`
///   into the `"rsx"` region under `FirstRsxWrite`.
/// - `PcReached(addr)` when the step's PC hits `Pc(addr)`.
/// - `Fault` on a step-result fault.
/// - `MaxSteps` on `MaxStepsExceeded`.
/// - `TimeOverflow` on `TimeOverflow` (clock saturation, distinct
///   from a guest-visible fault).
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
                if rsx_write_checkpoint_addr(checkpoint, &commit_result).is_some() {
                    return BootOutcome::RsxWriteCheckpoint;
                }
                if let Some(target) = target_pc {
                    if step.result.local_diagnostics.pc == Some(target) {
                        return BootOutcome::PcReached(target);
                    }
                }
                if step.result.fault.is_some() {
                    return BootOutcome::Fault;
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

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_core::{CommitError, CommitOutcome};
    use cellgov_mem::MemError;
    use manifest::CheckpointTrigger;
    use std::time::Duration;

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
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xC000_0040,
                region: "rsx",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
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
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xC000_0040,
                region: "rsx",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::ProcessExit, &err),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_successful_commit() {
        let ok: Result<CommitOutcome, CommitError> = Ok(CommitOutcome::default());
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &ok),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_non_memory_commit_errors() {
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::PayloadLengthMismatch { effect_index: 0 });
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            None
        );
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
    fn classify_tty_capture_clamps_len_at_4kib() {
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
                assert_eq!(bytes.len(), 4096);
            }
            other => panic!("expected InBounds, got {other:?}"),
        }
    }
}
