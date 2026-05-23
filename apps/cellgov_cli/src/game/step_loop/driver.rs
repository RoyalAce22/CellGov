//! `run-game` step driver: full diagnostics, ring buffers, TTY
//! capture, and per-step coverage tracking. The bench loop with
//! the same shared verdict classifier lives in [`super::bench`].

use std::time::Instant;

use cellgov_core::{Runtime, StepError};

use crate::game::diag::{
    append_orphan_exit_info, fetch_raw_at, format_commit_fault, format_deadlock, format_fault,
    format_max_steps, format_process_exit, print_trace_line, ProcessExitInfo, TtyCapture,
};
use crate::game::step_loop::ctx::StepLoopCtx;
use crate::game::step_loop::timing::compute_untracked;
use crate::game::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use crate::game::step_loop::verdict::{classify_step_outcome, StepVerdict};

pub(in crate::game) fn step_loop(
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
                            step.unit,
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
                            Ok(insn) => <&'static str>::from(&insn),
                            Err(_) => "DECODE_ERROR",
                        };
                        *ctx.insn_coverage.entry(name).or_insert(0) += 1;
                    }
                }
                let t_cov_end = Instant::now();

                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let mem = rt.memory().as_bytes();
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

fn handle_syscall_args(args: &[u64; 9], ctx: &mut StepLoopCtx<'_>, pc: u64, mem: &[u8]) {
    if args[0] >= 0x10000 {
        let idx = (args[0] - 0x10000) as u32;
        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
    } else if args[0] == cellgov_ps3_abi::syscall::TTY_WRITE {
        handle_tty_capture(args, ctx, pc, mem);
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

fn handle_tty_capture(args: &[u64; 9], ctx: &mut StepLoopCtx<'_>, pc: u64, mem: &[u8]) {
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
            let preview = crate::game::diag::ascii_safe_preview(&bytes);
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
}
