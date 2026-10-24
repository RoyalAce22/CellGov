//! The `boot bench` run set: N subprocess measurements, the gate over
//! what they must reproduce, and the report they print.

use cellgov_compare::bench::{
    classify_runs, determinism_disagreements, AnchorVerdict, BenchBootResult, BenchGate,
    MeasuredRun,
};

use super::anchor::{check_anchor, incomparable_reasons};
use super::divergence::locate_divergence;
use super::options::BenchOptions;
use super::spawn::{spawn_one_run, SpawnError};
use super::throughput::{print_throughput, throughput_verdict, ThroughputPolicy};
use super::types::BenchRunsOutcome;
use cellgov_boot::manifest::CellKey;

/// Run [`bench_boot_one_run`](super::bench_boot_one_run) `policy.runs` times in separate
/// subprocesses, gate on what the runs must reproduce, and report
/// throughput.
///
/// # Errors
///
/// Returns an error if the run count is zero or a child does not
/// produce a valid result.
pub fn bench_boot_runs(
    opts: BenchOptions<'_>,
    policy: ThroughputPolicy,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchRunsOutcome, SpawnError> {
    if policy.runs == 0 {
        return Err(SpawnError::Command(crate::cli::exit::CommandError::failed(
            "boot bench: a run set requires at least one measurement",
        )));
    }
    println!(
        "boot bench: title={} elf={} max_steps={} runs={}{}",
        opts.title.name(),
        opts.elf_path,
        opts.max_steps,
        policy.runs,
        override_tokens(&opts),
    );
    let (runs, streams) = measure_runs(opts, policy.runs, progress)?;
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
    if let Some(line) = streams[0]
        .lines()
        .find(|line| line.contains("invariant break at"))
    {
        eprintln!("{line}");
    }
    let anchor = anchor_verdict(&opts, &first, &streams[0]);
    print_anchor_line(&anchor, &opts);

    let throughput = throughput_verdict(&runs);
    print_throughput(throughput, policy);
    let gate = classify_runs(
        &determinism_failures,
        &anchor,
        throughput.is_measured(),
        policy.strict,
    );
    progress.finished();
    if gate == BenchGate::DeterminismBreak {
        println!("  determinism: BREAK");
        for failure in &determinism_failures {
            println!("    {failure}");
        }
        for line in locate_divergence(opts, &runs)? {
            println!("    {line}");
        }
    }
    Ok(BenchRunsOutcome {
        runs,
        throughput,
        gate,
        anchor,
        determinism_failures,
    })
}

/// The banner's optional trailing tokens, each carrying its own leading
/// space so the banner has no gap when none is present.
fn override_tokens(opts: &BenchOptions<'_>) -> String {
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
    for name in opts.identity.overrides.names() {
        overrides.push_str(&format!(" {name}"));
    }
    overrides
}

/// Spawn `runs` measurements, printing a line for each; returns each
/// run's result and its stderr.
fn measure_runs(
    opts: BenchOptions<'_>,
    runs: usize,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<(Vec<BenchBootResult>, Vec<String>), SpawnError> {
    progress.phase(crate::progress::BenchPairPhase::Measuring.code());
    // Both counters track the same set of runs.
    progress.totals(runs, runs as u64);
    let mut results: Vec<BenchBootResult> = Vec::with_capacity(runs);
    let mut streams: Vec<String> = Vec::with_capacity(runs);
    for index in 0..runs {
        progress.item_started(&format!("run {} of {}", index + 1, runs));
        let mut this_run = opts;
        this_run.run_index = index;
        let (result, stderr) = spawn_one_run(this_run)?;
        progress.advanced(1);
        progress.item_finished();
        println!(
            "  run {}: steps={} wall_ms={:.3} steps_per_sec={} outcome={}",
            index + 1,
            result.steps,
            result.wall.as_secs_f64() * 1e3,
            result.steps_per_sec(),
            result.outcome,
        );
        results.push(result);
        streams.push(stderr);
    }
    Ok((results, streams))
}

/// What run 1 says against the cell's committed anchor.
fn anchor_verdict(opts: &BenchOptions<'_>, first: &BenchBootResult, stream: &str) -> AnchorVerdict {
    if !opts.check_anchor {
        return AnchorVerdict::Skipped;
    }
    let reasons = incomparable_reasons(opts);
    if !reasons.is_empty() {
        return AnchorVerdict::NotComparable(reasons);
    }
    // `incomparable_reasons` names a missing cell as one of its
    // reasons, so an empty list implies a cell.
    let Some(cell) = opts.plan.cell else {
        debug_assert!(false, "an unnameable cell had no incomparable reason");
        return AnchorVerdict::NotComparable(vec!["the run has no cell identity".to_string()]);
    };
    check_anchor(
        &opts.title.content_id,
        cell,
        &MeasuredRun {
            checkpoint: opts
                .checkpoint_override
                .unwrap_or(opts.plan.checkpoint)
                .kind(),
            steps: first.steps as u64,
            budget: first.budget,
            outcome: first.outcome,
            stderr: stream,
        },
    )
}

fn print_anchor_line(anchor: &AnchorVerdict, opts: &BenchOptions<'_>) {
    match anchor {
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
}
