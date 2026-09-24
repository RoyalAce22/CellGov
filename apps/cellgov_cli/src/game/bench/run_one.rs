//! One measured boot: prepare, drive the step loop, emit the witness
//! block, and report the result line the parent reads.

use std::time::Instant;

use cellgov_compare::bench::{format_bench_result, BenchBootResult};

use super::options::BenchOptions;
use super::witnesses::print_witness_block;
use cellgov_boot::prepare::{
    prepare, BootServices, DiagnosticOptions, ExecutionOptions, PrepareOptions, TitleOptions,
};
use cellgov_boot::step_loop::bench_step_loop;

/// Carries a measured boot refusal to the command boundary.
#[derive(Debug, thiserror::Error)]
pub enum BenchBootError {
    /// The boot library refused the measured run.
    #[error("{0}")]
    Boot(#[from] cellgov_boot::BootError),
    /// The optional state-trace write failed.
    #[error("boot bench: writing state trace to {path}: {source}")]
    StateTrace {
        /// The output path the command received.
        path: String,
        /// The host write failure.
        #[source]
        source: std::io::Error,
    },
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: `prepare` applies the manifest's `[rsx]`
/// settings, not the runtime-overridable `checkpoint`, so a
/// `--checkpoint pc=ADDR` override does not change the boot
/// trajectory's init path.
///
/// `trace_path` puts the runtime in `DeterminismCheck` mode, which
/// costs a state hash per step. No comparison reads a traced run's
/// wall time.
fn bench_boot(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    trace_path: Option<&str>,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchBootResult, BenchBootError> {
    progress.phase(crate::progress::BootPhase::Loading.code());
    let sink = crate::game::console_sink();
    let ignored = crate::game::set_watch_vars();
    if !ignored.is_empty() {
        sink.warn(&format!(
            "boot bench installs no debug watch; ignoring {}",
            ignored.join(", ")
        ));
    }
    let prepared = prepare(PrepareOptions {
        title: TitleOptions {
            manifest: opts.title,
            elf_path: opts.elf_path,
            elf_data,
            authority_id,
            control_flags1,
            firmware_dir: opts.firmware_dir,
            composed_mounts: opts.composed_mounts,
            eboot_dirs: opts.eboot_dirs,
            identity: opts.identity,
        },
        execution: ExecutionOptions {
            runtime_max_steps: opts.max_steps,
            budget_override: opts.budget_override,
            strict_reserved: opts.strict_reserved,
            capture_state_trace: trace_path.is_some(),
            guest_args: opts.guest_args,
            patch_bytes: &[],
        },
        diagnostics: DiagnosticOptions {
            print_banner: false,
            prescan: opts.prescan,
            profile_pairs: false,
            dump_at_pc: None,
            dump_skip: 0,
            dump_mem_boot_addrs: &[],
            dump_mem_fault_ranges: &[],
        },
        services: BootServices {
            sink: std::rc::Rc::clone(&sink),
            keys: std::rc::Rc::new(crate::cli::keys::ProcessKeyVault),
            taps: std::rc::Rc::new(cellgov_boot::NoTaps),
        },
    })?;
    let mut rt = prepared.rt;
    let authid_source = prepared.authid_source;
    let child_init = prepared.child_init;
    let step_budget = prepared.step_budget;
    let active_checkpoint = opts.checkpoint_override.unwrap_or(opts.plan.checkpoint);

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let finish_line = crate::game::anchor_finish_line(
        &opts.title.content_id,
        opts.plan.cell,
        opts.retargets_trajectory(),
    );
    crate::progress::enter_step_loop(progress, crate::game::within_runtime_cap(finish_line, &rt));
    let outcome = match bench_step_loop(
        &mut rt,
        active_checkpoint,
        &mut steps,
        &child_init,
        progress,
        &sink,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            warn_first_invariant_break(&rt, sink.as_ref());
            return Err(error.into());
        }
    };
    let wall = t0.elapsed();
    // Stop the bar before the witness block prints; see
    // `ProgressSink::finished`.
    progress.finished();

    warn_first_invariant_break(&rt, sink.as_ref());
    print_witness_block(&rt, authid_source);

    // After the witness block: the write is host I/O, and a reader
    // that scrapes stderr must not have to wait on a disk.
    if let Some(path) = trace_path {
        std::fs::write(path, rt.trace().bytes()).map_err(|source| BenchBootError::StateTrace {
            path: path.to_string(),
            source,
        })?;
    }

    Ok(BenchBootResult {
        run_index: opts.run_index,
        steps,
        wall,
        budget: step_budget,
        outcome,
    })
}

/// Report the first LV2 host invariant break of the boot as a warning.
///
/// The witness block counts the breaks per site, and this line names
/// the first break. The host records each break and prints nothing.
/// The line carries no `BENCH_` prefix, so the witness reader skips it.
fn warn_first_invariant_break(rt: &cellgov_core::Runtime, sink: &dyn cellgov_boot::BootSink) {
    if let Some(line) = rt.lv2_host().observability().first_invariant_break_line() {
        sink.warn(&line);
    }
}

/// Emits one parent-readable result from a measured boot.
///
/// # Errors
///
/// Returns an error if the measured boot cannot produce its result line.
pub fn bench_boot_one_run(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    trace_path: Option<&str>,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchBootResult, BenchBootError> {
    let r = bench_boot(
        opts,
        elf_data,
        authority_id,
        control_flags1,
        trace_path,
        progress,
    )?;
    println!("{}", format_bench_result(&r));
    Ok(r)
}
