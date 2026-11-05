//! One measured boot: prepare, drive the step loop, emit the witness
//! block, and report the result line the parent reads.

use std::time::Instant;

use cellgov_compare::bench::{format_bench_result, BenchBootResult};

use super::options::BenchOptions;
use cellgov_boot::prepare::{
    prepare, BootServices, DiagnosticOptions, ExecutionOptions, PrepareOptions, TitleOptions,
};
use cellgov_boot::step_loop::{bench_step_loop, BenchWitnesses};

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
    /// The optional load-profile write failed.
    #[error("boot bench: writing load profile to {path}: {source}")]
    LoadProfile {
        /// The output path the command received.
        path: String,
        /// The host write failure.
        #[source]
        source: std::io::Error,
    },
}

/// The files a measured boot writes besides its result line.
#[derive(Debug, Clone, Copy, Default)]
pub struct BenchOutputs<'a> {
    /// Where the state trace goes. A traced run costs a state hash per
    /// step, so no comparison reads its wall time.
    pub state_trace: Option<&'a str>,
    /// Where the load-path profile goes. A profiled run counts each
    /// load, so no comparison reads its wall time either.
    pub load_profile: Option<&'a str>,
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: `prepare` applies the manifest's `[rsx]`
/// settings, not the runtime-overridable `checkpoint`, so a
/// `--checkpoint pc=ADDR` override does not change the boot
/// trajectory's init path.
///
/// A state trace puts the runtime in `DeterminismCheck` mode, and a
/// load profile puts every PPU unit in profile mode.
fn bench_boot(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    outputs: BenchOutputs<'_>,
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
            capture_state_trace: outputs.state_trace.is_some(),
            guest_args: opts.guest_args,
            patch_bytes: &[],
            engine: Default::default(),
        },
        diagnostics: DiagnosticOptions {
            print_banner: false,
            prescan: opts.prescan,
            profile_pairs: outputs.load_profile.is_some(),
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
    for line in BenchWitnesses::read(&rt, authid_source).lines() {
        eprintln!("{line}");
    }

    // After the witness block: the write is host I/O, and a reader
    // that scrapes stderr must not have to wait on a disk.
    if let Some(path) = outputs.state_trace {
        std::fs::write(path, rt.trace().bytes()).map_err(|source| BenchBootError::StateTrace {
            path: path.to_string(),
            source,
        })?;
    }
    if let Some(path) = outputs.load_profile {
        std::fs::write(path, load_summary(&rt).to_json()).map_err(|source| {
            BenchBootError::LoadProfile {
                path: path.to_string(),
                source,
            }
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

/// The load profiles of every PPU unit, summed.
fn load_summary(rt: &cellgov_core::Runtime) -> cellgov_ppu::profile::LoadSummary {
    let mut summary = cellgov_ppu::profile::LoadSummary::default();
    for (_, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            summary.merge(&ppu.load_profile().summary());
        }
    }
    summary
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
    outputs: BenchOutputs<'_>,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchBootResult, BenchBootError> {
    let r = bench_boot(
        opts,
        elf_data,
        authority_id,
        control_flags1,
        outputs,
        progress,
    )?;
    println!("{}", format_bench_result(&r));
    Ok(r)
}
