//! One measured boot: prepare, drive the step loop, emit the witness
//! block, and report the result line the parent reads.

use std::time::Instant;

use super::options::BenchOptions;
use super::result_line::format_bench_result;
use super::types::BenchBootResult;
use super::witnesses::print_witness_block;
use cellgov_boot::prepare::{prepare, PrepareOptions};
use cellgov_boot::step_loop::bench_step_loop;

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: `set_rsx_mirror_writes` is driven by the
/// manifest's declared checkpoint, not the runtime-overridable
/// `checkpoint`, so a `--checkpoint pc=ADDR` override does not change
/// the boot trajectory's init path.
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
) -> BenchBootResult {
    progress.phase(crate::progress::BootPhase::Loading.code());
    let sink = crate::game::console_sink();
    let prepared = prepare(PrepareOptions {
        title: opts.title,
        elf_path: opts.elf_path,
        elf_data,
        authority_id,
        control_flags1,
        firmware_dir: opts.firmware_dir,
        composed_mounts: opts.composed_mounts,
        eboot_dirs: opts.eboot_dirs,
        identity: opts.identity,
        strict_reserved: opts.strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        dump_mem_fault_ranges: &[],
        print_banner: false,
        runtime_max_steps: opts.max_steps,
        patch_bytes: &[],
        dump_mem_boot_addrs: &[],
        profile_pairs: false,
        budget_override: opts.budget_override,
        capture_state_trace: trace_path.is_some(),
        prescan: opts.prescan,
        guest_args: opts.guest_args,
        sink: std::rc::Rc::clone(&sink),
        keys: std::rc::Rc::new(crate::cli::keys::ProcessKeyVault),
    })
    .unwrap_or_else(|e| crate::cli::exit::die(&e.to_string()));
    let mut rt = prepared.rt;
    let authid_source = prepared.authid_source;
    let child_init = prepared.child_init;
    let step_budget = prepared.step_budget;
    let active_checkpoint = opts.checkpoint_override.unwrap_or(opts.plan.checkpoint);
    crate::game::run::configure_rsx_from_manifest(&mut rt, opts.title);

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let finish_line = crate::game::anchor_finish_line(
        &opts.title.content_id,
        opts.plan.cell,
        opts.retargets_trajectory(),
    );
    crate::progress::enter_step_loop(progress, crate::game::within_runtime_cap(finish_line, &rt));
    let outcome = bench_step_loop(
        &mut rt,
        active_checkpoint,
        &mut steps,
        &child_init,
        progress,
        &sink,
    )
    .unwrap_or_else(|e| crate::cli::exit::die(&e.to_string()));
    let wall = t0.elapsed();
    // Stop the bar before the witness block prints; see
    // `ProgressSink::finished`.
    progress.finished();

    print_witness_block(&rt, authid_source);

    // After the witness block: the write is host I/O, and a reader
    // that scrapes stderr must not have to wait on a disk.
    if let Some(path) = trace_path {
        std::fs::write(path, rt.trace().bytes()).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!("boot bench: writing state trace to {path}: {e}"))
        });
    }

    BenchBootResult {
        run_index: opts.run_index,
        steps,
        wall,
        budget: step_budget,
        outcome,
    }
}

/// Run a single bench invocation and print one `BENCH_RESULT` line.
pub fn bench_boot_one_run(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    trace_path: Option<&str>,
    progress: &dyn crate::progress::ProgressSink,
) -> BenchBootResult {
    let r = bench_boot(
        opts,
        elf_data,
        authority_id,
        control_flags1,
        trace_path,
        progress,
    );
    println!("{}", format_bench_result(&r));
    r
}
