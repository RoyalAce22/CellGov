//! The `boot bench` run set: N subprocess measurements, the gate over
//! what they must reproduce, and the report they print.

use super::anchor::{check_anchor, incomparable_reasons, AnchorVerdict, MeasuredRun};
use super::divergence::{determinism_disagreements, locate_divergence};
use super::options::BenchOptions;
use super::spawn::{spawn_one_run, SpawnError};
use super::throughput::{
    print_throughput, throughput_verdict, ThroughputPolicy, ThroughputVerdict,
};
use super::types::{BenchBootResult, BenchGate, BenchRunsOutcome};
use crate::game::manifest::CellKey;

/// Run [`bench_boot_one_run`](super::bench_boot_one_run) `policy.runs` times in separate
/// subprocesses, gate on what the runs must reproduce, and report
/// throughput.
///
/// # Panics
///
/// Panics if `policy.runs` is zero. A set of no runs has nothing to
/// compare, and the argument parser refuses the value.
pub fn bench_boot_runs(
    opts: BenchOptions<'_>,
    policy: ThroughputPolicy,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchRunsOutcome, SpawnError> {
    assert!(
        policy.runs > 0,
        "invariant: a run set takes at least one measurement"
    );
    // Optional trailing tokens, each carrying its own leading space
    // so the banner has no gap when both are absent.
    let mut overrides = String::new();
    if let Some(cp) = opts.checkpoint_override {
        overrides.push_str(&format!(" checkpoint={}", cp.as_cli_str()));
    }
    if let Some(b) = opts.budget_override {
        overrides.push_str(&format!(" budget={b}"));
    }
    if !opts.guest_args.is_empty() {
        // Debug quoting: guest argv entries may contain spaces.
        overrides.push_str(&format!(" guest_args={:?}", opts.guest_args));
    }
    println!(
        "boot bench: title={} elf={} max_steps={} runs={}{overrides}",
        opts.title.name(),
        opts.elf_path,
        opts.max_steps,
        policy.runs,
    );
    progress.phase(crate::progress::BenchPairPhase::Measuring.code());
    // Both counters track the same set of runs.
    progress.totals(policy.runs, policy.runs as u64);
    let mut runs: Vec<BenchBootResult> = Vec::with_capacity(policy.runs);
    let mut streams: Vec<String> = Vec::with_capacity(policy.runs);
    for index in 0..policy.runs {
        progress.item_started(&format!("run {} of {}", index + 1, policy.runs));
        let mut this_run = opts;
        this_run.run_index = index;
        let (result, stderr) = spawn_one_run(this_run)?;
        progress.advanced(1);
        progress.item_finished();
        println!(
            "  run {}: steps={} wall_ms={:.3} steps_per_sec={:.0} outcome={}",
            index + 1,
            result.steps,
            result.wall.as_secs_f64() * 1e3,
            result.steps_per_sec(),
            result.outcome,
        );
        runs.push(result);
        streams.push(stderr);
    }
    progress.phase(crate::progress::BenchPairPhase::Comparing.code());

    let determinism_failures = determinism_disagreements(&runs, &streams);
    // A set of one run has no second run to compare against, so the
    // hard gate covers nothing.
    if policy.runs == 1 {
        println!("  determinism: NOT CHECKED -- a set of one run reproduces nothing");
    }
    // Only run 1's stream reaches the anchor;
    // `determinism_disagreements` above already checked that every run
    // produced the same stream.
    let first = runs[0];
    let anchor = if !opts.check_anchor {
        AnchorVerdict::Skipped
    } else {
        let reasons = incomparable_reasons(&opts);
        match (reasons.is_empty(), opts.plan.cell) {
            // `incomparable_reasons` names a missing cell as one of its
            // reasons, so an empty list implies a cell.
            (true, Some(cell)) => check_anchor(
                &opts.title.content_id,
                cell,
                &MeasuredRun {
                    checkpoint: opts.checkpoint_override.unwrap_or(opts.plan.checkpoint),
                    steps: first.steps as u64,
                    budget: first.budget,
                    outcome: first.outcome.to_string(),
                    stderr: &streams[0],
                },
            ),
            (true, None) => unreachable!("an unnameable cell is itself an incomparable reason"),
            (false, _) => AnchorVerdict::NotComparable(reasons),
        }
    };
    match &anchor {
        AnchorVerdict::Skipped => {}
        AnchorVerdict::NotComparable(reasons) => {
            println!(
                "  anchor: NOT COMPARED against {} -- {}",
                opts.title.content_id,
                reasons.join("; ")
            );
        }
        AnchorVerdict::NotRecorded(cell) => println!(
            "  anchor: NOT RECORDED for {cell} (gates nothing) -- record it with \
             `dev record-anchors --title {}`",
            opts.title.name()
        ),
        AnchorVerdict::Match => println!(
            "  anchor: matches {} {}",
            opts.title.content_id,
            opts.plan.cell.map_or_else(String::new, CellKey::label)
        ),
        AnchorVerdict::Drift(f) => {
            println!(
                "  anchor: {} disagreement(s) vs {} {}",
                f.len(),
                opts.title.content_id,
                opts.plan.cell.map_or_else(String::new, CellKey::label)
            )
        }
    }

    let throughput = throughput_verdict(&runs);
    print_throughput(throughput, policy);
    let gate = classify_runs(&determinism_failures, &anchor, throughput, policy);
    progress.finished();
    if gate == BenchGate::DeterminismBreak {
        println!("  determinism: BREAK");
        for failure in &determinism_failures {
            println!("    {failure}");
        }
        for line in locate_divergence(opts, &runs) {
            println!("    {line}");
        }
    }
    Ok(BenchRunsOutcome {
        runs,
        throughput,
        gate,
        anchor_failures: match anchor {
            AnchorVerdict::Drift(f) => f,
            _ => Vec::new(),
        },
        determinism_failures,
    })
}

/// Order matters. A determinism break makes the witness stream
/// meaningless. An anchor disagreement outranks the throughput
/// verdict, so a contended host cannot mask a real regression behind a
/// timing failure.
///
/// Throughput reaches the gate only under `policy.strict`: a busy host
/// inflates the spread of a run that regressed nothing.
pub(super) fn classify_runs(
    determinism_failures: &[String],
    anchor: &AnchorVerdict,
    throughput: ThroughputVerdict,
    policy: ThroughputPolicy,
) -> BenchGate {
    if !determinism_failures.is_empty() {
        return BenchGate::DeterminismBreak;
    }
    if matches!(anchor, AnchorVerdict::Drift(_)) {
        return BenchGate::AnchorDrift;
    }
    if policy.strict && !throughput.is_measured() {
        return BenchGate::SpreadExceeded;
    }
    BenchGate::Pass
}

#[cfg(test)]
#[path = "tests/runs_tests.rs"]
mod tests;
