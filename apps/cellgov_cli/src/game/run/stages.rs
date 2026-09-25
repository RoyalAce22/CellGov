//! The `boot run` stage sequence.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cellgov_boot::diag::{
    report_hle_summary, report_insn_coverage, report_shadow_stats, report_top_pcs,
};
use cellgov_boot::manifest::TitleManifest;
use cellgov_boot::prepare::{prepare, BootServices, PrepareOptions, PreparedBoot};
use cellgov_boot::step_loop::{
    step_loop, PcRing, RunAnomalies, StepLoopCtx, StepTiming, SyscallRing,
};
use cellgov_boot::taps::{StateHashCensus, WithPpuTap};
use cellgov_boot::{BootSink, ChildInitPlans, DebugTaps};
use cellgov_compare::BootOutcome;
use cellgov_core::{AddressSpaceId, Runtime};

use super::artifacts::{save_artifacts, RunError, RunFacts};
use super::options::{RunArtifacts, RunExecution, RunReporting};
use super::report::{self, InsnPair, RunSpans, FREQUENCY_ROWS};

/// Terminal-state summary from [`run_game`].
pub struct RunSummary {
    /// The terminal state the step loop reached.
    pub outcome: BootOutcome,
    /// Whether a counter makes that result suspect; see
    /// [`RunAnomalies::had_critical_anomaly`].
    pub had_critical_anomaly: bool,
}

/// Boot a PS3 ELF and drive the PPU step loop until a terminal state.
///
/// # Errors
///
/// Returns an error if any stage refuses the run.
pub fn run_game(
    execution: RunExecution<'_>,
    artifacts: RunArtifacts<'_>,
    reporting: RunReporting<'_>,
) -> Result<RunSummary, RunError> {
    debug_assert_dumpable(reporting.boot.dump_mem_fault_ranges);
    if let Some(refusal) =
        state_trace_mismatch(artifacts.state_trace, execution.limits.capture_state_trace)
    {
        return Err(RunError::StateTraceConfiguration(refusal));
    }
    let mut spans = RunSpans::start()?;
    let sink = crate::game::console_sink();
    let title = execution.title.manifest;
    let identity = execution.title.identity;

    let census = reporting
        .state_hash_census
        .then(|| Rc::new(StateHashCensus::new(CENSUS_SAMPLE_EVERY)));
    let prepared = prepare_boot(execution, &reporting, &sink, census.as_ref())?;
    spans.mark_prepared();
    let PreparedBoot {
        mut rt,
        elf_data,
        timings,
        step_budget,
        child_init,
        ..
    } = prepared;
    if reporting.profile {
        report::print_out(&report::startup_timing_lines(&timings));
    }

    let loop_out = drive_step_loop(&mut rt, title, &child_init, &reporting, &sink)?;
    spans.mark_stepped();
    let dirty_pages = rt.memory().dirty_page_count();

    let counters = report_outcome(&mut rt, &loop_out, sink.as_ref());
    if let Some(timing) = &loop_out.timing {
        report::print_out(&report::step_profile_lines(
            timing,
            loop_out.t_loop,
            loop_out.steps,
        ));
    }
    if reporting.boot.profile_pairs {
        report_unit_profiles(&mut rt);
    }
    if let Some(census) = &census {
        report::print_out(&report::census_lines(&census.report()));
    }

    save_artifacts(
        &mut rt,
        &artifacts,
        &RunFacts {
            title,
            identity,
            elf_data: &elf_data,
            outcome: loop_out.boot_outcome,
            steps: loop_out.steps,
            step_budget,
        },
        sink.as_ref(),
    )?;

    if let Some(line) = spans.line(loop_out.steps, dirty_pages) {
        eprintln!("{line}");
    }

    Ok(RunSummary {
        outcome: loop_out.boot_outcome,
        had_critical_anomaly: counters.had_critical_anomaly(),
    })
}

/// Why the state-trace path and the capture flag disagree, or `None`
/// when they agree.
///
/// `capture_state_trace` selects `RuntimeMode::DeterminismCheck` in
/// `cellgov_boot::prepare`, which adds the per-step state hash and
/// changes the default budget. A mismatch takes one of two shapes:
///
/// - A path with the capture off saves a hash-free file from a
///   different trajectory.
/// - The capture with no path pays for the hash and saves nothing.
fn state_trace_mismatch(state_trace: Option<&str>, capture_state_trace: bool) -> Option<String> {
    match (state_trace, capture_state_trace) {
        (Some(path), false) => Some(format!(
            "save-state-trace: asked for {path} with capture_state_trace off; \
             the stream would carry no per-step state hash and the run would \
             take the fault-driven budget"
        )),
        (None, true) => Some(
            "capture_state_trace is on with no save-state-trace path; the run \
             would take the determinism-check budget and write no trace"
                .to_string(),
        ),
        _ => None,
    }
}

/// The `--dump-mem-fault` parser rejects both shapes, so a range the
/// hex dumper cannot walk is a caller bug.
fn debug_assert_dumpable(ranges: &[(u64, u64)]) {
    for (i, &(addr, len)) in ranges.iter().enumerate() {
        debug_assert!(
            len > 0,
            "dump_mem_fault_ranges[{i}]: zero length at addr 0x{addr:x}"
        );
        debug_assert!(
            addr.checked_add(len.saturating_sub(1)).is_some(),
            "dump_mem_fault_ranges[{i}]: addr 0x{addr:x} + len 0x{len:x} overflows u64"
        );
    }
}

/// Dispatches between two samples of the state-hash census.
const CENSUS_SAMPLE_EVERY: u64 = 1 << 20;

fn prepare_boot(
    execution: RunExecution<'_>,
    reporting: &RunReporting<'_>,
    sink: &Rc<dyn BootSink>,
    census: Option<&Rc<StateHashCensus>>,
) -> Result<PreparedBoot, RunError> {
    eprintln!(
        "boot run: title = {} ({})",
        execution.title.manifest.name(),
        execution.title.manifest.display_name()
    );
    let mut taps = crate::game::debug_taps_from_env()
        .map_err(|error| RunError::DebugTaps(error.to_string()))?;
    if let Some(census) = census {
        taps = Rc::new(WithPpuTap::new(taps, Rc::clone(census) as _)) as Rc<dyn DebugTaps>;
    }
    reporting
        .progress
        .phase(crate::progress::BootPhase::Loading.code());
    Ok(prepare(PrepareOptions {
        title: execution.title,
        execution: execution.limits,
        diagnostics: reporting.boot,
        services: BootServices {
            sink: Rc::clone(sink),
            keys: Rc::new(crate::cli::keys::ProcessKeyVault),
            taps,
        },
    })?)
}

/// What one step-loop drive produced.
struct LoopOutput {
    /// The loop's own account of why it stopped.
    outcome: String,
    /// That account as the harness's terminal state.
    boot_outcome: BootOutcome,
    steps: usize,
    hle_calls: BTreeMap<u32, usize>,
    insn_coverage: BTreeMap<&'static str, usize>,
    pc_hits: BTreeMap<(AddressSpaceId, u64), u64>,
    /// Per-bucket wall time, when the caller asks for a profile.
    timing: Option<StepTiming>,
    t_loop: Duration,
    tty_oob_dropped: usize,
    tty_bogus_fd: usize,
}

fn drive_step_loop(
    rt: &mut Runtime,
    title: &TitleManifest,
    child_init: &ChildInitPlans,
    reporting: &RunReporting<'_>,
    sink: &Rc<dyn BootSink>,
) -> Result<LoopOutput, RunError> {
    let mut steps: usize = 0;
    let mut hle_calls = BTreeMap::new();
    let mut insn_coverage = BTreeMap::new();
    let mut pc_hits = BTreeMap::new();
    let mut timing = reporting.profile.then(StepTiming::default);
    let loop_start = Instant::now();
    let mut ctx = StepLoopCtx {
        steps: &mut steps,
        hle_calls: &mut hle_calls,
        insn_coverage: &mut insn_coverage,
        trace: reporting.trace,
        timing: &mut timing,
        loop_start,
        pc_ring: PcRing::new(),
        last_tty: None,
        last_exit: None,
        syscall_ring: SyscallRing::new(),
        pc_hits: &mut pc_hits,
        checkpoint: title.checkpoint_trigger(),
        tty_oob_count: 0,
        bogus_fd_count: 0,
        dump_mem_fault_ranges: reporting.boot.dump_mem_fault_ranges,
        obs_null_sink: crate::cli::env::parse_env_bool(crate::env_vars::OBS_NULL_SINK)?,
        child_init,
        progress: reporting.progress,
        sink: Rc::clone(sink),
    };
    crate::progress::enter_step_loop(
        reporting.progress,
        crate::game::within_runtime_cap(reporting.finish_line, rt),
    );
    let (outcome, boot_outcome) = match step_loop(rt, &mut ctx) {
        Ok(output) => output,
        Err(error) => {
            report_first_invariant_break(rt, sink.as_ref());
            return Err(error.into());
        }
    };
    let t_loop = loop_start.elapsed();
    // Stop the bar here: it clears within a tick of this call, so the
    // caller's result report prints on a clear terminal. See
    // `ProgressSink::finished`.
    reporting.progress.finished();
    let tty_oob_dropped = ctx.tty_oob_count;
    let tty_bogus_fd = ctx.bogus_fd_count;
    Ok(LoopOutput {
        outcome,
        boot_outcome,
        steps,
        hle_calls,
        insn_coverage,
        pc_hits,
        timing,
        t_loop,
        tty_oob_dropped,
        tty_bogus_fd,
    })
}

/// Report where the run ended and what the runtime counted on the way.
fn report_outcome(rt: &mut Runtime, loop_out: &LoopOutput, sink: &dyn BootSink) -> RunAnomalies {
    let counters = RunAnomalies::read(rt, loop_out.tty_oob_dropped, loop_out.tty_bogus_fd);
    println!("outcome: {}", loop_out.outcome);
    println!("steps: {}", loop_out.steps);
    report::print_out(&report::anomaly_lines(&counters));
    report_first_invariant_break(rt, sink);
    report_hle_summary(&loop_out.hle_calls, sink);
    report_insn_coverage(&loop_out.insn_coverage, sink);
    report_top_pcs(rt, &loop_out.pc_hits, sink);
    report_shadow_stats(rt, sink);
    super::unmodelled::print(rt);
    counters
}

/// Report the first LV2 host invariant break of the run as a warning.
///
/// The host records each break and prints nothing. Without this call,
/// the run reports the count of breaks and no detail.
fn report_first_invariant_break(rt: &Runtime, sink: &dyn BootSink) {
    if let Some(line) = rt.lv2_host().observability().first_invariant_break_line() {
        sink.warn(&line);
    }
}

/// Report the per-unit instruction and adjacent-pair tallies the
/// `--profile-pairs` units collected.
fn report_unit_profiles(rt: &mut Runtime) {
    for (id, unit) in rt.units_mut() {
        report::print_err(&report::frequency_block(
            &format!("unit {}: instruction frequency", id.raw()),
            &unit.drain_profile_insns(),
            FREQUENCY_ROWS,
        ));
    }
    for (id, unit) in rt.units_mut() {
        let rows: Vec<(InsnPair, u64)> = unit
            .drain_profile_pairs()
            .into_iter()
            .map(|((a, b), count)| (InsnPair(a, b), count))
            .collect();
        report::print_err(&report::frequency_block(
            &format!("unit {}: adjacent pair frequency", id.raw()),
            &rows,
            FREQUENCY_ROWS,
        ));
    }
}

#[cfg(test)]
#[path = "tests/stages_tests.rs"]
mod tests;
