//! `bench-boot` / `bench-boot-once` / `bench-boot-pair` machinery.
//!
//! Two-process harness for boot throughput measurement. A pair runs
//! two subprocesses back-to-back, parses a `BENCH_RESULT` line from
//! each, and reports an agreement percentage as the reproducibility
//! gate.

use std::time::Instant;

use super::boot;
use super::manifest::{self, TitleManifest};
use super::step_loop::bench_step_loop;

/// Result of one [`bench_boot`] invocation.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BenchBootResult {
    pub steps: usize,
    pub wall: std::time::Duration,
    pub outcome: cellgov_compare::BootOutcome,
}

impl BenchBootResult {
    pub fn steps_per_sec(&self) -> f64 {
        let secs = self.wall.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            self.steps as f64 / secs
        }
    }
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination. Shares `boot::prepare` with `run-game`; only
/// the step loop differs.
pub fn bench_boot(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
    budget_override: Option<u64>,
) -> BenchBootResult {
    let prepared = boot::prepare(boot::PrepareOptions {
        title,
        elf_path,
        firmware_dir,
        strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        module_start_max_steps: max_steps,
        print_banner: false,
        runtime_max_steps: max_steps,
        patch_bytes: &[],
        dump_mem_addrs: &[],
        profile_pairs: false,
        budget_override,
    });
    let mut rt = prepared.rt;
    let checkpoint = checkpoint_override.unwrap_or_else(|| title.checkpoint_trigger());
    if checkpoint == manifest::CheckpointTrigger::FirstRsxWrite {
        rt.set_gcm_rsx_checkpoint(true);
    }
    if title.rsx_mirror() {
        rt.set_gcm_rsx_checkpoint(true);
        rt.set_rsx_mirror_writes(true);
    }

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let outcome = bench_step_loop(&mut rt, checkpoint, &mut steps);
    let wall = t0.elapsed();

    BenchBootResult {
        steps,
        wall,
        outcome,
    }
}

/// Run a single bench invocation and print one `BENCH_RESULT` line
/// to stdout.
///
/// Each measurement runs in its own subprocess: in-process
/// back-to-back runs drift ~60 percent in wall time on Windows due
/// to 1 GB guest-memory page-commit reuse. A fresh heap, page
/// tables, and CPU caches are required for the two runs to be
/// comparable.
pub fn bench_boot_one_run(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
    budget_override: Option<u64>,
) -> BenchBootResult {
    let r = bench_boot(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
        budget_override,
    );
    println!(
        "BENCH_RESULT steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r.steps,
        r.wall.as_millis(),
        r.steps_per_sec(),
        format_bench_outcome(r.outcome),
    );
    r
}

/// Render a [`cellgov_compare::BootOutcome`] for a `BENCH_RESULT`
/// line. The canonical string form shared with [`parse_bench_result`].
pub(crate) fn format_bench_outcome(outcome: cellgov_compare::BootOutcome) -> String {
    use cellgov_compare::BootOutcome;
    match outcome {
        BootOutcome::ProcessExit => "ProcessExit".into(),
        BootOutcome::RsxWriteCheckpoint => "RsxWriteCheckpoint".into(),
        BootOutcome::Fault => "Fault".into(),
        BootOutcome::MaxSteps => "MaxSteps".into(),
        BootOutcome::PcReached(addr) => format!("PcReached(0x{addr:x})"),
        BootOutcome::TimeOverflow => "TimeOverflow".into(),
    }
}

/// Run `bench_boot_one_run` twice in separate subprocesses and
/// print a pair report. Rejects the pair when wall times disagree
/// by more than 5 percent.
pub fn bench_boot_pair(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
    budget_override: Option<u64>,
) -> (BenchBootResult, BenchBootResult) {
    let checkpoint_label = match checkpoint_override {
        Some(manifest::CheckpointTrigger::Pc(a)) => format!(" checkpoint=pc=0x{a:x}"),
        Some(manifest::CheckpointTrigger::ProcessExit) => " checkpoint=process-exit".to_string(),
        Some(manifest::CheckpointTrigger::FirstRsxWrite) => {
            " checkpoint=first-rsx-write".to_string()
        }
        None => String::new(),
    };
    let budget_label = budget_override
        .map(|b| format!(" budget={b}"))
        .unwrap_or_default();
    println!(
        "bench-boot: title={} elf={elf_path} max_steps={max_steps}{checkpoint_label}{budget_label}",
        title.name()
    );
    let r1 = spawn_one_run(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
        budget_override,
    );
    println!(
        "  run 1: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r1.steps,
        r1.wall.as_millis(),
        r1.steps_per_sec(),
        format_bench_outcome(r1.outcome),
    );
    let r2 = spawn_one_run(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
        budget_override,
    );
    println!(
        "  run 2: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r2.steps,
        r2.wall.as_millis(),
        r2.steps_per_sec(),
        format_bench_outcome(r2.outcome),
    );
    let agreement = agreement_percent(r1.wall, r2.wall);
    let gate = if agreement <= 5.0 { "OK" } else { "FAIL" };
    println!("  agreement: {agreement:.2}% (gate: <= 5% => {gate})");
    (r1, r2)
}

/// Fork-and-exec the current binary as `bench-boot-once` and parse
/// its `BENCH_RESULT` line from stdout. Stderr is inherited.
fn spawn_one_run(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
    budget_override: Option<u64>,
) -> BenchBootResult {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("bench-boot-once")
        .arg("--title")
        .arg(title.name())
        .arg("--max-steps")
        .arg(max_steps.to_string());
    if let Some(d) = firmware_dir {
        cmd.arg("--firmware-dir").arg(d);
    }
    if strict_reserved {
        cmd.arg("--strict-reserved");
    }
    if let Some(cp) = checkpoint_override {
        let value = match cp {
            manifest::CheckpointTrigger::ProcessExit => "process-exit".to_string(),
            manifest::CheckpointTrigger::FirstRsxWrite => "first-rsx-write".to_string(),
            manifest::CheckpointTrigger::Pc(a) => format!("pc=0x{a:x}"),
        };
        cmd.arg("--checkpoint").arg(value);
    }
    if let Some(b) = budget_override {
        cmd.arg("--budget").arg(b.to_string());
    }
    cmd.arg(elf_path);
    let output = cmd.output().expect("subprocess runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A nonzero-exit subprocess that emitted BENCH_RESULT before
    // crashing would feed bogus data into the agreement gate.
    if !output.status.success() {
        eprintln!(
            "bench-boot: subprocess exited nonzero (status={:?}); refusing to accept its BENCH_RESULT",
            output.status.code()
        );
        eprintln!("stdout:\n{stdout}");
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(3);
    }
    parse_bench_result(&stdout).unwrap_or_else(|| {
        eprintln!("bench-boot: subprocess did not emit BENCH_RESULT line");
        eprintln!("stdout:\n{stdout}");
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(3);
    })
}

/// Parse a `BENCH_RESULT steps=N wall_ms=M steps_per_sec=X outcome=O`
/// line out of captured stdout.
///
/// # Errors
///
/// Returns `None` when no such line is present or any required field
/// is missing or malformed.
pub(crate) fn parse_bench_result(stdout: &str) -> Option<BenchBootResult> {
    let line = stdout.lines().find(|l| l.starts_with("BENCH_RESULT "))?;
    let mut steps: Option<usize> = None;
    let mut wall_ms: Option<u64> = None;
    let mut outcome: Option<cellgov_compare::BootOutcome> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = v.parse().ok();
        } else if let Some(v) = tok.strip_prefix("wall_ms=") {
            wall_ms = v.parse().ok();
        } else if tok.starts_with("steps_per_sec=") {
            // Derivable from steps and wall_ms; recognized so the
            // unknown-token warning below does not fire on it.
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome = match v {
                "ProcessExit" => Some(cellgov_compare::BootOutcome::ProcessExit),
                "RsxWriteCheckpoint" => Some(cellgov_compare::BootOutcome::RsxWriteCheckpoint),
                "Fault" => Some(cellgov_compare::BootOutcome::Fault),
                "MaxSteps" => Some(cellgov_compare::BootOutcome::MaxSteps),
                "TimeOverflow" => Some(cellgov_compare::BootOutcome::TimeOverflow),
                other => {
                    // PcReached(0xADDR) is the one variant that
                    // carries a payload; malformed payloads return
                    // None so the line fails loudly.
                    if let Some(addr_hex) = other
                        .strip_prefix("PcReached(0x")
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        u64::from_str_radix(addr_hex, 16)
                            .ok()
                            .map(cellgov_compare::BootOutcome::PcReached)
                    } else {
                        None
                    }
                }
            };
        } else {
            // Surfaces a formatter/parser drift; parsing continues
            // so required fields may still yield a result.
            eprintln!(
                "parse_bench_result: warning: unknown token {tok:?} in BENCH_RESULT line; parser may be stale"
            );
        }
    }
    Some(BenchBootResult {
        steps: steps?,
        wall: std::time::Duration::from_millis(wall_ms?),
        outcome: outcome?,
    })
}

/// Relative wall-time difference between two runs, as a percentage
/// of the faster run.
pub(crate) fn agreement_percent(a: std::time::Duration, b: std::time::Duration) -> f64 {
    let aa = a.as_secs_f64();
    let bb = b.as_secs_f64();
    if aa == 0.0 || bb == 0.0 {
        return 0.0;
    }
    let min = aa.min(bb);
    let max = aa.max(bb);
    100.0 * (max - min) / min
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agreement_percent_is_zero_for_identical_durations() {
        use std::time::Duration;
        assert_eq!(
            agreement_percent(Duration::from_millis(1000), Duration::from_millis(1000)),
            0.0
        );
    }

    #[test]
    fn agreement_percent_is_relative_to_faster_run() {
        use std::time::Duration;
        let pct = agreement_percent(Duration::from_millis(100), Duration::from_millis(105));
        assert!((pct - 5.0).abs() < 0.0001, "expected 5.0, got {pct}");
    }

    #[test]
    fn agreement_percent_is_symmetric() {
        use std::time::Duration;
        let a = agreement_percent(Duration::from_millis(200), Duration::from_millis(250));
        let b = agreement_percent(Duration::from_millis(250), Duration::from_millis(200));
        assert_eq!(a, b);
    }

    #[test]
    fn agreement_percent_returns_zero_on_empty_duration() {
        use std::time::Duration;
        assert_eq!(
            agreement_percent(Duration::ZERO, Duration::from_millis(100)),
            0.0
        );
    }

    #[test]
    fn parse_bench_result_extracts_fields() {
        let stdout = "some preamble\nBENCH_RESULT steps=1402388 wall_ms=323 steps_per_sec=4342377 outcome=ProcessExit\ntrailing noise\n";
        let r = parse_bench_result(stdout).expect("parses");
        assert_eq!(r.steps, 1402388);
        assert_eq!(r.wall.as_millis(), 323);
        assert_eq!(r.outcome, cellgov_compare::BootOutcome::ProcessExit);
    }

    #[test]
    fn parse_bench_result_handles_rsx_checkpoint_outcome() {
        let stdout =
            "BENCH_RESULT steps=12345 wall_ms=77 steps_per_sec=160000 outcome=RsxWriteCheckpoint\n";
        let r = parse_bench_result(stdout).expect("parses");
        assert_eq!(r.outcome, cellgov_compare::BootOutcome::RsxWriteCheckpoint);
    }

    #[test]
    fn parse_bench_result_handles_pc_reached_outcome() {
        let stdout = "BENCH_RESULT steps=1402388 wall_ms=250 steps_per_sec=5609552 outcome=PcReached(0x10381ce8)\n";
        let r = parse_bench_result(stdout).expect("parses");
        assert_eq!(
            r.outcome,
            cellgov_compare::BootOutcome::PcReached(0x10381ce8)
        );
        assert_eq!(r.steps, 1402388);
    }

    #[test]
    fn parse_bench_result_handles_time_overflow_outcome() {
        let stdout = "BENCH_RESULT steps=100 wall_ms=1 steps_per_sec=100000 outcome=TimeOverflow\n";
        let r = parse_bench_result(stdout).expect("parses");
        assert_eq!(r.outcome, cellgov_compare::BootOutcome::TimeOverflow);
    }

    #[test]
    fn parse_bench_result_none_on_malformed_pc_reached() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=PcReached(abc\n";
        assert!(parse_bench_result(stdout).is_none());
    }

    #[test]
    fn format_bench_outcome_pc_reached_hex() {
        let s = format_bench_outcome(cellgov_compare::BootOutcome::PcReached(0x10381ce8));
        assert_eq!(s, "PcReached(0x10381ce8)");
    }

    #[test]
    fn format_bench_outcome_time_overflow() {
        let s = format_bench_outcome(cellgov_compare::BootOutcome::TimeOverflow);
        assert_eq!(s, "TimeOverflow");
    }

    #[test]
    fn parse_bench_result_none_when_no_result_line() {
        let stdout = "just some noise\nbut no result line\n";
        assert!(parse_bench_result(stdout).is_none());
    }

    #[test]
    fn parse_bench_result_none_on_unknown_outcome() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=WhoKnows\n";
        assert!(parse_bench_result(stdout).is_none());
    }
}
