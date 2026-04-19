//! Step-loop machinery for `run-game` and `bench-boot`.
//!
//! Two loops live here: `step_loop` is the diagnostic-heavy driver
//! the `run-game` command uses (ring buffers, TTY capture, progress
//! checkpoints, timing breakdown); `bench_step_loop` is the minimal
//! throughput driver for `bench-boot`. Both share the
//! RSX-checkpoint classifier so a refactor cannot let them drift
//! on what counts as a checkpoint hit -- that drift is the likely
//! future bug when two near-parallel loops live in separate files.

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
    /// Ring buffer of recent PCs for mini-trace on fault. The
    /// `usize` position counter increments monotonically and indexes
    /// via `% PC_RING_SIZE`. On 64-bit hosts (our only current
    /// target) wraparound is effectively impossible. On a 32-bit
    /// host the modulo still yields a valid index after wrap, but
    /// the pre-wrap "how many steps" reading becomes meaningless;
    /// the ring itself stays consistent.
    pub(super) pc_ring: [u64; PC_RING_SIZE],
    pub(super) pc_ring_pos: usize,
    /// Last TTY write buffer (raw bytes) for diagnostic artifact.
    pub(super) last_tty: Option<TtyCapture>,
    /// Set when sys_process_exit is dispatched.
    pub(super) last_exit: Option<ProcessExitInfo>,
    /// Ring buffer of recent LV2 syscall numbers for exit diagnostic.
    /// Same wraparound reasoning as `pc_ring_pos`.
    pub(super) syscall_ring: [(u64, u64); SYSCALL_RING_SIZE],
    pub(super) syscall_ring_pos: usize,
    /// Per-PC hit counts. Identifies busy-loop bodies when the run
    /// hits max-steps without faulting: the loop's PCs dominate the
    /// top entries.
    pub(super) pc_hits: &'a mut std::collections::HashMap<u64, u64>,
    /// The boot checkpoint the harness is looking for. See
    /// [`manifest::CheckpointTrigger`]. Controls whether a
    /// reserved-region write is treated as a checkpoint reach or as
    /// a normal fault (the commit pipeline discards either way).
    pub(super) checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls where `buf + len` overflowed memory
    /// bounds and the capture had to be skipped. Nonzero indicates
    /// a guest bug or corrupted caller, surfaced once in the final
    /// report rather than silently dropped per-call.
    pub(super) tty_oob_count: usize,
    /// `sys_tty_write` calls whose `args[1]` fd value did not fit
    /// in `u32`. Normal PS3 fds are 0/1/2; any wider value signals
    /// a corrupted caller. We narrow to `u32::MAX` so the print is
    /// obviously bogus; the counter ensures the operator knows how
    /// many times the sentinel fired.
    pub(super) bogus_fd_count: usize,
}

/// Classify a `commit_step` outcome as a checkpoint hit, if the
/// title's trigger is [`manifest::CheckpointTrigger::FirstRsxWrite`]
/// and the commit failed with a `ReservedWrite` to the RSX region.
///
/// Returns the triggering guest address when the checkpoint fires,
/// `None` otherwise. Pulled out as a free function so a unit test
/// can pin the detection shape without spinning up a full runtime,
/// and so both loops route through the same decision.
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

/// Decision for one `sys_tty_write` call, extracted so the bounds
/// and fd-narrowing logic is unit-testable without a full runtime.
///
/// Pure over (args, mem_bytes): no stdout, no counter mutation,
/// no allocation of TtyCapture. The step-loop call site turns
/// the decision into the side effects.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TtyCaptureDecision {
    /// Buffer fits entirely in mapped memory; carries the
    /// bytes, the narrowed fd, and a flag for whether the
    /// incoming `args[1]` value overflowed `u32`.
    InBounds {
        fd: u32,
        fd_was_bogus: bool,
        bytes: Vec<u8>,
    },
    /// `buf + len` overflows mapped memory. The helper carries
    /// the raw values back so the caller can log them verbatim.
    Oob {
        buf: usize,
        len: usize,
        mem_len: usize,
    },
}

/// Classify a `sys_tty_write` guest call without touching runtime
/// state. `args` is the raw syscall-args array; `mem_bytes` is the
/// currently-committed guest memory slice.
///
/// `checked_add` on `buf + len` is load-bearing: a guest value near
/// `usize::MAX` could wrap past the `<= mem.len()` check if plain
/// addition were used, letting OOB captures slip through.
pub(super) fn classify_tty_capture(args: &[u64; 9], mem_bytes: &[u8]) -> TtyCaptureDecision {
    let buf = args[2] as usize;
    // PS3 sys_tty_write caps output at 4096 bytes; match that so a
    // guest that passes a gigantic len does not allocate a huge
    // preview vector.
    let len = (args[3] as usize).min(4096);
    let end = buf.checked_add(len);
    if end.is_none_or(|e| e > mem_bytes.len()) {
        return TtyCaptureDecision::Oob {
            buf,
            len,
            mem_len: mem_bytes.len(),
        };
    }
    // Narrow the fd via try_from so a guest passing a >32-bit
    // value surfaces as u32::MAX (an obviously-bogus sentinel)
    // rather than silently aliasing to a plausible low fd.
    // sys_tty_write uses 0/1/2 in practice; any wider value
    // signals a corrupted caller.
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

/// Compute the untracked time inside the step loop.
///
/// `Ok(overhead)` is `t_loop - (step + commit + coverage)` when
/// the tracked buckets fit inside the loop. `Err(excess)` is the
/// amount by which the buckets overflow -- a timing-invariant
/// violation that means either the timed regions overlap, a
/// region double-counts, or `t_loop` starts after the loop
/// actually began.
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

                // Progress checkpoint every 10K steps.
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

                // Tally instruction coverage from the PC.
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
                // Track HLE/LV2 calls and capture TTY/exit before commit.
                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
                        // Detect sys_process_exit via HLE dispatch.
                        if let Some(binding) = ctx.hle_bindings.get(idx as usize) {
                            if binding.nid == 0xe6f2c1e7 {
                                ctx.last_exit = Some(ProcessExitInfo {
                                    code: args[1] as u32,
                                    call_pc: pc,
                                });
                            }
                        }
                    } else if args[0] == 403 {
                        // sys_tty_write: classify once via the
                        // pure helper so the bounds and fd-narrowing
                        // logic is unit-testable without a full
                        // runtime + step loop.
                        match classify_tty_capture(args, rt.memory().as_bytes()) {
                            TtyCaptureDecision::InBounds {
                                fd,
                                fd_was_bogus,
                                bytes,
                            } => {
                                if fd_was_bogus {
                                    ctx.bogus_fd_count += 1;
                                }
                                // Sanitize to ASCII so binary payloads
                                // (e.g. microtest result structs) do
                                // not emit control bytes or U+FFFD
                                // replacements that cp1252 / cp437
                                // Windows consoles mangle.
                                let preview = super::diag::ascii_safe_preview(&bytes);
                                print!("  tty[fd={fd}]: {preview}");
                                if !preview.ends_with('\n') {
                                    println!();
                                }
                                // Flush so the TTY line reaches stdout
                                // before any subsequent fault stack
                                // print lands on stderr.
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
                        // sys_process_exit: capture exit code and PC.
                        ctx.last_exit = Some(ProcessExitInfo {
                            code: args[1] as u32,
                            call_pc: pc,
                        });
                    }
                    // Track all syscalls (HLE and LV2) in ring buffer.
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

/// Minimal step loop: only the state needed to detect a termination
/// condition. A ProcessExit fires when the runtime reports no
/// runnable unit (the `sys_process_exit` path removes the primary
/// unit); a FirstRsxWrite fires when `commit_step` returns
/// `ReservedWrite` into the rsx region; a fault breaks with Fault;
/// and exhausting `max_steps` breaks with MaxSteps.
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
            // TimeOverflow is a harness-level resource exhaustion
            // (the time counter saturated), not a guest-visible
            // error. Keeping it distinct from Fault lets a
            // reproducibility pair tell "clock rolled over" from
            // "guest triggered a decode error or bad address."
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
        // args[0] is the syscall number (403 for tty_write) but
        // classify_tty_capture does not inspect it; set it anyway
        // so the fixture matches how the step loop constructs
        // the value.
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
        // fd = u32::MAX + 1: does not fit in u32, so the narrowed
        // value must be u32::MAX and fd_was_bogus must be true.
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
        // buf + len = 10, but mem is only 5 bytes. Classified as
        // Oob with the raw buf/len/mem_len echoed back so the
        // caller's log can name exact values.
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
        // buf = usize::MAX with any nonzero len overflows
        // `buf + len`; plain `+` would wrap past the bounds check
        // and let OOB captures slip through. Classify as Oob
        // without panicking.
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
        // len of 8000 must be capped at 4096 so a guest passing a
        // huge len does not allocate a huge preview vector. When
        // backing memory is large enough, the clamped slice is
        // exactly 4096 bytes.
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
