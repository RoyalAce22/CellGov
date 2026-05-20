//! Step drivers for `run-game` (diagnostic) and `bench-boot` (throughput).
//!
//! Both loops share [`classify_step_outcome`] for verdict precedence.

use std::time::Instant;

use cellgov_core::{Runtime, StepError};

use super::diag::{
    append_orphan_exit_info, fetch_raw_at, format_commit_fault, format_deadlock, format_fault,
    format_max_steps, format_process_exit, print_trace_line, ProcessExitInfo, TtyCapture,
};
use super::manifest;

mod block_reason;
mod ring;
mod timing;
pub(super) mod tty;
mod verdict;
pub(super) use block_reason::block_reason_label;
pub(super) use ring::{RingCursor, PC_RING_SIZE, SYSCALL_RING_SIZE};
pub(super) use timing::{compute_untracked, pct, StepTiming};
pub(super) use tty::{classify_tty_capture, TtyCaptureDecision};
pub(super) use verdict::{classify_step_outcome, StepVerdict};

pub(super) struct StepLoopCtx<'a> {
    pub(super) steps: &'a mut usize,
    pub(super) distinct_pcs: &'a mut std::collections::BTreeSet<u64>,
    pub(super) hle_calls: &'a mut std::collections::BTreeMap<u32, usize>,
    pub(super) insn_coverage: &'a mut std::collections::BTreeMap<&'static str, usize>,
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
    pub(super) pc_hits: &'a mut std::collections::BTreeMap<u64, u64>,
    pub(super) checkpoint: manifest::CheckpointTrigger,
    /// `sys_tty_write` calls dropped because `buf + len` exceeded mapped memory.
    pub(super) tty_oob_count: usize,
    /// `sys_tty_write` calls whose fd exceeded `u32::MAX` (narrowed to sentinel).
    pub(super) bogus_fd_count: usize,
    /// Address+length pairs to hex-dump from guest memory at fault
    /// time. Empty by default; set via `run-game --dump-mem-fault`.
    pub(super) dump_mem_fault_ranges: &'a [(u64, u64)],
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

                // PC ring and distinct-PC set track attempted execution;
                // they advance before commit (kept on a discarded batch).
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
                    print_trace_line(rt, step.unit, &step.result, *ctx.steps);
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
                            ctx.dump_mem_fault_ranges,
                        );
                        append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                        break (diag, BootOutcome::Fault);
                    }
                    StepVerdict::PcReached(_) => {
                        unreachable!("step_loop never sets target_pc")
                    }
                    StepVerdict::Continue => {}
                }

                // Post-commit counters: only advance when the batch was applied.
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
                    let _ = pc;
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
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
                                let preview = super::diag::ascii_safe_preview(&bytes);
                                print!("  tty[fd={fd}{bogus_marker}]: {preview}");
                                if !preview.ends_with('\n') {
                                    println!();
                                }
                                // Flush stdout so a stderr fault stack does not interleave.
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
                    // Monotonic clock + disjoint regions imply tracked <= loop_start.elapsed().
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
                        ),
                        BootOutcome::ProcessExit,
                    );
                }
                // Every unit drained without a process/thread-exit dispatch.
                break (
                    format!(
                        "ALL_UNITS_FINISHED after {} steps without sys_process_exit",
                        ctx.steps
                    ),
                    BootOutcome::Fault,
                );
            }
            Err(StepError::AllBlocked) => {
                let mut diag = format_deadlock(rt, *ctx.steps, &ctx.pc_ring, &ctx.pc_ring_cursor);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                break (diag, BootOutcome::Fault);
            }
            Err(StepError::MaxStepsExceeded) => {
                let mut diag = format_max_steps(
                    rt,
                    *ctx.steps,
                    &ctx.pc_ring,
                    &ctx.pc_ring_cursor,
                    &ctx.syscall_ring,
                    &ctx.syscall_ring_cursor,
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
