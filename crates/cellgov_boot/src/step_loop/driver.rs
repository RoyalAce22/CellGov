//! `boot run` step driver: full diagnostics, ring buffers, TTY
//! capture, and per-step coverage tracking. The bench loop with
//! the same shared verdict classifier lives in [`super::bench`].

use std::time::Instant;

use cellgov_core::{Runtime, StepError};

use crate::diag::{
    append_orphan_exit_info, fetch_raw_at, format_commit_fault, format_deadlock, format_fault,
    format_max_steps, format_process_exit, report_trace_line, unit_memory, ProcessExitInfo,
    TtyCapture,
};
use crate::step_loop::ctx::StepLoopCtx;
use crate::step_loop::timing::compute_untracked;
use crate::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use crate::step_loop::verdict::{classify_step_outcome, StepVerdict};
use crate::step_loop::STEP_REPORT_BATCH;
use crate::BootError;

/// Drive the runtime to a terminal state.
///
/// The returned string is the diagnostic report for that terminal
/// state.
///
/// # Errors
///
/// A spawned child's init pass that could not finish; see
/// [`crate::ChildInitPlans`].
pub fn step_loop(
    rt: &mut Runtime,
    ctx: &mut StepLoopCtx<'_>,
) -> Result<(String, cellgov_compare::BootOutcome), BootError> {
    // The tail report takes `steps` modulo the batch. That remainder
    // is the partial batch only when the count starts at zero.
    debug_assert_eq!(*ctx.steps, 0, "the step counter enters the loop at zero");
    let terminal = drive(rt, ctx);
    // The loop reports whole batches, so the bar needs the partial
    // batch it ended on to reach the step count the run reports.
    ctx.progress
        .advanced((*ctx.steps % STEP_REPORT_BATCH) as u64);
    terminal
}

/// Split from the wrapper so every exit passes the tail report.
fn drive(
    rt: &mut Runtime,
    ctx: &mut StepLoopCtx<'_>,
) -> Result<(String, cellgov_compare::BootOutcome), BootError> {
    use cellgov_compare::BootOutcome;
    loop {
        // A parked child's staged init pass runs before the scheduler
        // sees it: an entry still pending at `rt.step()` reads as
        // AllBlocked once every other unit blocks.
        //
        // The pass retires `rt.step()` calls that `ctx.steps` never
        // counts, and the bar counts what `ctx.steps` counts; see
        // `super::bench`.
        if rt.has_pending_child_init() {
            crate::child_init::run_pending_child_inits(rt, ctx.child_init, &ctx.sink)?;
        }

        let t0 = Instant::now();
        let step_result = rt.step();
        let t1 = Instant::now();

        match step_result {
            Ok(step) => {
                *ctx.steps += 1;

                // The PC ring tracks attempted execution; it advances
                // before commit (kept on a discarded batch).
                if let Some(pc) = step.result.local_diagnostics.pc {
                    ctx.pc_ring.push((rt.unit_space(step.unit), pc));
                }

                if (*ctx.steps).is_multiple_of(STEP_REPORT_BATCH) {
                    ctx.progress.advanced(STEP_REPORT_BATCH as u64);
                }

                if ctx.trace {
                    report_trace_line(rt, step.unit, &step.result, *ctx.steps, ctx.sink.as_ref());
                }

                let t2 = Instant::now();
                let commit_result = rt.commit_step(&step.result, &step.effects);
                let t3 = Instant::now();

                // Inertness gate. Commit/wake paths push invariant
                // breaks AFTER the dispatch-time drain, so entries can
                // be pending here; `clear_observability` carries them
                // across the reset for the next dispatch's trace drain.
                if ctx.obs_null_sink {
                    rt.lv2_host_mut().clear_observability();
                }

                match classify_step_outcome(&step.result, &commit_result, ctx.checkpoint, None) {
                    StepVerdict::RsxCheckpoint(addr) => {
                        return Ok((
                            format!(
                                "RSX_WRITE_CHECKPOINT at 0x{addr:x} after {} steps",
                                ctx.steps
                            ),
                            BootOutcome::RsxWriteCheckpoint,
                        ));
                    }
                    StepVerdict::CommitFault => {
                        let err = commit_result
                            .as_ref()
                            .expect_err("classified as CommitFault implies Err");
                        let mut diag =
                            format_commit_fault(rt, err, *ctx.steps, step.unit, &ctx.pc_ring);
                        append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                        return Ok((diag, BootOutcome::Fault));
                    }
                    StepVerdict::StepFault => {
                        let fault = step
                            .result
                            .fault
                            .as_ref()
                            .expect("classified as StepFault implies Some");
                        let mut diag = format_fault(
                            rt,
                            step.unit,
                            &step.result,
                            fault,
                            *ctx.steps,
                            &ctx.pc_ring,
                            ctx.dump_mem_fault_ranges,
                        );
                        append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                        return Ok((diag, BootOutcome::Fault));
                    }
                    StepVerdict::PcReached(_) => {
                        unreachable!("step_loop never sets target_pc")
                    }
                    StepVerdict::Continue => {}
                }

                // Post-commit counters: only advance when the batch was applied.
                if let Some(pc) = step.result.local_diagnostics.pc {
                    *ctx.pc_hits
                        .entry((rt.unit_space(step.unit), pc))
                        .or_insert(0) += 1;
                }

                let t_cov_start = Instant::now();
                if let Some(pc) = step.result.local_diagnostics.pc {
                    if let Some(raw) = fetch_raw_at(unit_memory(rt, step.unit), pc) {
                        let name = match cellgov_ppu::decode::decode(raw) {
                            Ok(insn) => <&'static str>::from(&insn),
                            Err(_) => "DECODE_ERROR",
                        };
                        *ctx.insn_coverage.entry(name).or_insert(0) += 1;
                    }
                }
                let t_cov_end = Instant::now();

                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    // The buffer is read through the caller's own space:
                    // a child process passes child-space addresses.
                    let mem = rt
                        .space_memory(rt.unit_space(step.unit))
                        .expect("a unit that just stepped executes in a live space");
                    handle_syscall_args(args, ctx, pc, mem);
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
                    return Ok((
                        format_process_exit(
                            exit,
                            ctx.last_tty.as_ref(),
                            *ctx.steps,
                            &ctx.pc_ring,
                            &ctx.syscall_ring,
                        ),
                        BootOutcome::ProcessExit,
                    ));
                }
                return Ok((
                    format!(
                        "ALL_UNITS_FINISHED after {} steps without sys_process_exit",
                        ctx.steps
                    ),
                    BootOutcome::Fault,
                ));
            }
            Err(StepError::AllBlocked) => {
                let mut diag = format_deadlock(rt, *ctx.steps, &ctx.pc_ring);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                return Ok((diag, BootOutcome::Fault));
            }
            Err(StepError::MaxStepsExceeded) => {
                let mut diag = format_max_steps(rt, *ctx.steps, &ctx.pc_ring, &ctx.syscall_ring);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                return Ok((diag, BootOutcome::MaxSteps));
            }
            Err(StepError::TimeOverflow) => {
                let mut diag = format!("TIME_OVERFLOW after {} steps", ctx.steps);
                append_orphan_exit_info(&mut diag, ctx.last_exit.as_ref());
                return Ok((diag, BootOutcome::TimeOverflow));
            }
            Err(StepError::SchedulerNotReinstalled) => {
                unreachable!(
                    "boot driver does not call Runtime::restore_into; \
                     reaching this arm means a new caller added a \
                     restore path without rethinking the dispatch."
                );
            }
        }
    }
}

fn handle_syscall_args(
    args: &[u64; 9],
    ctx: &mut StepLoopCtx<'_>,
    pc: u64,
    mem: &cellgov_mem::GuestMemory,
) {
    if args[0] >= 0x10000 {
        let idx = (args[0] - 0x10000) as u32;
        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
    } else if args[0] == cellgov_ps3_abi::lv2::syscall::TTY_WRITE {
        handle_tty_capture(args, ctx, pc, mem);
    } else if args[0] == cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT
        || args[0] == cellgov_ps3_abi::lv2::syscall::PPU_THREAD_EXIT
    {
        ctx.last_exit = Some(ProcessExitInfo {
            code: args[1] as u32,
            call_pc: pc,
        });
    }
    ctx.syscall_ring.push((args[0], pc));
}

fn handle_tty_capture(
    args: &[u64; 9],
    ctx: &mut StepLoopCtx<'_>,
    pc: u64,
    mem: &cellgov_mem::GuestMemory,
) {
    match classify_tty_capture(args, mem) {
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
            let preview = crate::diag::ascii_safe_preview(&bytes);
            let terminator = if preview.ends_with('\n') { "" } else { "\n" };
            ctx.sink.guest_text(&format!(
                "  tty[fd={fd}{bogus_marker}]: {preview}{terminator}"
            ));
            ctx.last_tty = Some(TtyCapture {
                fd,
                raw_bytes: bytes,
                call_pc: pc,
            });
        }
        TtyCaptureDecision::Oob { buf, len, reason } => {
            ctx.tty_oob_count += 1;
            ctx.sink.warn(&format!(
                "  tty_oob: sys_tty_write buf=0x{buf:x}+0x{len:x}: {reason}; capture dropped at step {}",
                *ctx.steps
            ));
        }
    }
}
