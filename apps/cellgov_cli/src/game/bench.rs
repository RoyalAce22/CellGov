//! `bench-boot` / `bench-boot-once` / `bench-boot-pair` machinery.
//!
//! A pair runs two subprocesses back-to-back, parses a `BENCH_RESULT`
//! line from each, and reports an agreement percentage as the
//! reproducibility gate.

use std::str::FromStr;
use std::time::Instant;

use cellgov_compare::{BootOutcome, BootOutcomeParseError};
use cellgov_time::Budget;

use super::boot::{self, BootMode};
use super::manifest::{self, TitleManifest};
use super::step_loop::bench_step_loop;

/// Wall-time disagreement that trips the pair gate, as a percentage
/// of the faster run.
pub const BENCH_AGREEMENT_GATE_PCT: f64 = 5.0;

/// Inputs common to every bench-boot entry point. Mirrors
/// `boot::PrepareOptions` for the bench-specific subset.
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    pub firmware_dir: Option<&'a str>,
    pub boot_mode: BootMode,
    pub strict_reserved: bool,
    pub checkpoint_override: Option<manifest::CheckpointTrigger>,
    pub budget_override: Option<Budget>,
}

impl BenchOptions<'_> {
    /// Append the CLI argument form of this struct onto `cmd` for
    /// re-invocation as `bench-boot-once`.
    fn encode_to_command(&self, cmd: &mut std::process::Command) {
        cmd.arg("bench-boot-once")
            .arg("--title")
            .arg(self.title.name())
            .arg("--max-steps")
            .arg(self.max_steps.to_string())
            .arg("--boot-mode")
            .arg(self.boot_mode.as_cli_str());
        if let Some(d) = self.firmware_dir {
            cmd.arg("--firmware-dir").arg(d);
        }
        if self.strict_reserved {
            cmd.arg("--strict-reserved");
        }
        if let Some(cp) = self.checkpoint_override {
            let value = match cp {
                manifest::CheckpointTrigger::ProcessExit => "process-exit".to_string(),
                manifest::CheckpointTrigger::FirstRsxWrite => "first-rsx-write".to_string(),
                manifest::CheckpointTrigger::Pc(a) => format!("pc=0x{a:x}"),
            };
            cmd.arg("--checkpoint").arg(value);
        }
        if let Some(b) = self.budget_override {
            cmd.arg("--budget").arg(b.raw().to_string());
        }
        // ELF path intentionally omitted: bench-boot-once is title-driven,
        // and the child resolves the EBOOT via the same title manifest.
    }
}

/// One completed bench run: retired step count, wall duration, and
/// terminal [`BootOutcome`].
#[derive(Debug, Clone, Copy)]
pub struct BenchBootResult {
    pub steps: usize,
    pub wall: std::time::Duration,
    pub outcome: BootOutcome,
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

/// Gate verdict for [`bench_boot_pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGate {
    /// Both runs agree on steps + outcome; wall drift within
    /// [`BENCH_AGREEMENT_GATE_PCT`].
    Pass,
    /// Runs disagreed on retired step count or boot outcome.
    DeterminismBreak,
    /// Wall drift exceeded the gate.
    WallDriftExceeded,
    /// Wall measurement was zero / non-finite.
    WallUnmeasurable,
}

/// Result of one [`bench_boot_pair`] invocation.
#[derive(Debug, Clone, Copy)]
pub struct BenchPairOutcome {
    pub run1: BenchBootResult,
    pub run2: BenchBootResult,
    pub drift_pct: Option<f64>,
    pub gate: BenchGate,
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: the runtime's `set_gcm_rsx_checkpoint` /
/// `set_rsx_mirror_writes` toggles are driven by the *manifest's*
/// declared checkpoint, not the runtime-overridable `checkpoint`. A
/// `--checkpoint pc=ADDR` override on a title whose manifest declares
/// `FirstRsxWrite` keeps the GCM-checkpoint init path so the boot
/// trajectory matches the manifest's wired-in expectations.
pub fn bench_boot(opts: BenchOptions<'_>) -> BenchBootResult {
    let prepared = boot::prepare(boot::PrepareOptions {
        title: opts.title,
        elf_path: opts.elf_path,
        firmware_dir: opts.firmware_dir,
        boot_mode: opts.boot_mode,
        strict_reserved: opts.strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        module_start_max_steps: opts.max_steps,
        print_banner: false,
        runtime_max_steps: opts.max_steps,
        patch_bytes: &[],
        dump_mem_boot_addrs: &[],
        profile_pairs: false,
        budget_override: opts.budget_override,
        capture_state_trace: false,
    });
    let mut rt = prepared.rt;
    let active_checkpoint = opts
        .checkpoint_override
        .unwrap_or_else(|| opts.title.checkpoint_trigger());
    super::configure_rsx_from_manifest(&mut rt, opts.title);

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let outcome = bench_step_loop(&mut rt, active_checkpoint, &mut steps);
    let wall = t0.elapsed();

    BenchBootResult {
        steps,
        wall,
        outcome,
    }
}

/// Run a single bench invocation and print one `BENCH_RESULT` line.
///
/// Each measurement runs in its own subprocess: in-process back-to-back
/// runs drift ~60 percent in wall time on Windows due to 1 GB
/// guest-memory page-commit reuse.
pub fn bench_boot_one_run(opts: BenchOptions<'_>) -> BenchBootResult {
    let r = bench_boot(opts);
    println!(
        "BENCH_RESULT steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r.steps,
        r.wall.as_millis(),
        r.steps_per_sec(),
        r.outcome,
    );
    r
}

/// Subprocess invocation failure surfaced by [`spawn_one_run`].
#[derive(Debug)]
pub enum SpawnError {
    /// `Command::output` itself returned an I/O error.
    Io(std::io::Error),
    /// The subprocess exited with a nonzero status.
    SubprocessNonzero {
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// The subprocess exited cleanly but its stdout could not be
    /// parsed into a `BenchBootResult`.
    ParseFailed {
        error: ParseBenchError,
        stdout: String,
        stderr: String,
    },
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "subprocess spawn failed: {e}"),
            Self::SubprocessNonzero { status, .. } => {
                write!(f, "subprocess exited nonzero (status={status:?})")
            }
            Self::ParseFailed { error, .. } => {
                write!(f, "BENCH_RESULT parse failed: {error}")
            }
        }
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::ParseFailed { error, .. } => Some(error),
            Self::SubprocessNonzero { .. } => None,
        }
    }
}

impl SpawnError {
    /// Captured stdout, if any was attached to the failure.
    pub fn captured_stdout(&self) -> &str {
        match self {
            Self::Io(_) => "",
            Self::SubprocessNonzero { stdout, .. } | Self::ParseFailed { stdout, .. } => stdout,
        }
    }

    /// Captured stderr, if any was attached to the failure.
    pub fn captured_stderr(&self) -> &str {
        match self {
            Self::Io(_) => "",
            Self::SubprocessNonzero { stderr, .. } | Self::ParseFailed { stderr, .. } => stderr,
        }
    }
}

/// Spawn the current binary as `bench-boot-once` and parse its
/// `BENCH_RESULT` line. Subprocess stderr is forwarded so warnings
/// reach the parent on the success path.
fn spawn_one_run(opts: BenchOptions<'_>) -> Result<BenchBootResult, SpawnError> {
    let exe = std::env::current_exe().map_err(SpawnError::Io)?;
    let mut cmd = std::process::Command::new(&exe);
    opts.encode_to_command(&mut cmd);
    let output = cmd.output().map_err(SpawnError::Io)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(SpawnError::SubprocessNonzero {
            status: output.status.code(),
            stdout,
            stderr,
        });
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    match parse_bench_result(&stdout) {
        Ok(r) => Ok(r),
        Err(error) => Err(SpawnError::ParseFailed {
            error,
            stdout,
            stderr,
        }),
    }
}

/// Run [`bench_boot_one_run`] twice in separate subprocesses and
/// classify the pair against the gate.
pub fn bench_boot_pair(opts: BenchOptions<'_>) -> Result<BenchPairOutcome, SpawnError> {
    let checkpoint_label = match opts.checkpoint_override {
        Some(manifest::CheckpointTrigger::Pc(a)) => format!(" checkpoint=pc=0x{a:x}"),
        Some(manifest::CheckpointTrigger::ProcessExit) => " checkpoint=process-exit".to_string(),
        Some(manifest::CheckpointTrigger::FirstRsxWrite) => {
            " checkpoint=first-rsx-write".to_string()
        }
        None => String::new(),
    };
    let budget_label = opts
        .budget_override
        .map(|b| format!(" budget={b}"))
        .unwrap_or_default();
    println!(
        "bench-boot: title={} elf={} max_steps={}{checkpoint_label}{budget_label}",
        opts.title.name(),
        opts.elf_path,
        opts.max_steps
    );
    let r1 = spawn_one_run(opts)?;
    println!(
        "  run 1: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r1.steps,
        r1.wall.as_millis(),
        r1.steps_per_sec(),
        r1.outcome,
    );
    let r2 = spawn_one_run(opts)?;
    println!(
        "  run 2: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r2.steps,
        r2.wall.as_millis(),
        r2.steps_per_sec(),
        r2.outcome,
    );
    let drift_pct = wall_disagreement_percent(r1.wall, r2.wall);
    let gate = classify_pair(&r1, &r2, drift_pct);
    match gate {
        BenchGate::Pass => {
            let d = drift_pct.expect("Pass implies finite drift");
            println!("  agreement: {d:.2}% (gate: <= 5% => OK)");
        }
        BenchGate::WallDriftExceeded => {
            let d = drift_pct.expect("WallDriftExceeded implies finite drift");
            println!("  agreement: {d:.2}% (gate: <= 5% => FAIL)");
        }
        BenchGate::WallUnmeasurable => {
            println!("  agreement: unmeasurable (gate: <= 5% => FAIL)");
        }
        BenchGate::DeterminismBreak => {
            println!("  agreement: determinism break (steps/outcome differ)");
        }
    }
    Ok(BenchPairOutcome {
        run1: r1,
        run2: r2,
        drift_pct,
        gate,
    })
}

fn classify_pair(r1: &BenchBootResult, r2: &BenchBootResult, drift_pct: Option<f64>) -> BenchGate {
    if r1.steps != r2.steps || r1.outcome != r2.outcome {
        return BenchGate::DeterminismBreak;
    }
    match drift_pct {
        Some(d) if d > BENCH_AGREEMENT_GATE_PCT => BenchGate::WallDriftExceeded,
        Some(_) => BenchGate::Pass,
        None => BenchGate::WallUnmeasurable,
    }
}

/// Failure mode while parsing a `BENCH_RESULT` line out of subprocess
/// stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseBenchError {
    /// No line starting with `BENCH_RESULT ` was present.
    NoResultLine,
    /// More than one `BENCH_RESULT` line was present.
    DuplicateResultLine,
    /// The line was present but the `steps=` field was missing.
    MissingSteps,
    /// The `steps=` field was present but could not parse as `usize`.
    MalformedSteps(String),
    /// The line was present but the `wall_ms=` field was missing.
    MissingWallMs,
    /// The `wall_ms=` field was present but could not parse as `u64`.
    MalformedWallMs(String),
    /// The line was present but the `outcome=` field was missing.
    MissingOutcome,
    /// The `outcome=` field could not parse as a [`BootOutcome`].
    UnparseableOutcome {
        token: String,
        source: BootOutcomeParseError,
    },
}

impl std::fmt::Display for ParseBenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoResultLine => f.write_str("no BENCH_RESULT line"),
            Self::DuplicateResultLine => f.write_str("more than one BENCH_RESULT line"),
            Self::MissingSteps => f.write_str("BENCH_RESULT: missing steps= field"),
            Self::MalformedSteps(s) => write!(f, "BENCH_RESULT: malformed steps={s:?}"),
            Self::MissingWallMs => f.write_str("BENCH_RESULT: missing wall_ms= field"),
            Self::MalformedWallMs(s) => write!(f, "BENCH_RESULT: malformed wall_ms={s:?}"),
            Self::MissingOutcome => f.write_str("BENCH_RESULT: missing outcome= field"),
            Self::UnparseableOutcome { token, source } => {
                write!(f, "BENCH_RESULT: malformed outcome={token:?}: {source}")
            }
        }
    }
}

impl std::error::Error for ParseBenchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnparseableOutcome { source, .. } => Some(source),
            Self::NoResultLine
            | Self::DuplicateResultLine
            | Self::MissingSteps
            | Self::MalformedSteps(_)
            | Self::MissingWallMs
            | Self::MalformedWallMs(_)
            | Self::MissingOutcome => None,
        }
    }
}

/// Parse the `BENCH_RESULT steps=N wall_ms=M steps_per_sec=X outcome=O`
/// line out of captured stdout.
pub(crate) fn parse_bench_result(stdout: &str) -> Result<BenchBootResult, ParseBenchError> {
    let mut iter = stdout.lines().filter(|l| l.starts_with("BENCH_RESULT "));
    let line = iter.next().ok_or(ParseBenchError::NoResultLine)?;
    if iter.next().is_some() {
        return Err(ParseBenchError::DuplicateResultLine);
    }
    let mut steps: Option<usize> = None;
    let mut wall_ms: Option<u64> = None;
    let mut outcome_token: Option<String> = None;
    let mut reported_sps: Option<f64> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedSteps(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("wall_ms=") {
            wall_ms = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedWallMs(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("steps_per_sec=") {
            reported_sps = v.parse().ok();
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome_token = Some(v.to_string());
        } else {
            eprintln!(
                "parse_bench_result: warning: unknown token {tok:?} in BENCH_RESULT line; parser may be stale"
            );
        }
    }
    let steps = steps.ok_or(ParseBenchError::MissingSteps)?;
    let wall_ms = wall_ms.ok_or(ParseBenchError::MissingWallMs)?;
    let outcome_token = outcome_token.ok_or(ParseBenchError::MissingOutcome)?;
    let outcome = BootOutcome::from_str(&outcome_token).map_err(|source| {
        ParseBenchError::UnparseableOutcome {
            token: outcome_token.clone(),
            source,
        }
    })?;
    let wall = std::time::Duration::from_millis(wall_ms);
    let result = BenchBootResult {
        steps,
        wall,
        outcome,
    };
    // `steps_per_sec` is a redundant projection of (steps, wall_ms);
    // a drift beyond rounding tolerance means the formatter and the
    // recomputation have gone out of sync.
    if let Some(reported) = reported_sps {
        let computed = result.steps_per_sec();
        let tolerance = (computed * 0.01).max(1.0);
        debug_assert!(
            (reported - computed).abs() <= tolerance,
            "BENCH_RESULT steps_per_sec drift: reported={reported} computed={computed} (tolerance={tolerance})"
        );
    }
    Ok(result)
}

/// Relative wall-time disagreement between two runs, as a percentage
/// of the faster run. Returns `None` when either duration is zero so
/// the caller cannot silently pass an unmeasurable run through the
/// gate.
pub(crate) fn wall_disagreement_percent(
    a: std::time::Duration,
    b: std::time::Duration,
) -> Option<f64> {
    let aa = a.as_secs_f64();
    let bb = b.as_secs_f64();
    if !(aa > 0.0 && bb > 0.0) {
        return None;
    }
    let min = aa.min(bb);
    let max = aa.max(bb);
    // `min > 0` and both inputs come from `Duration::as_secs_f64`,
    // so the division is finite without a separate `is_finite` check.
    Some(100.0 * (max - min) / min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_disagreement_percent_is_zero_for_identical_durations() {
        use std::time::Duration;
        assert_eq!(
            wall_disagreement_percent(Duration::from_millis(1000), Duration::from_millis(1000)),
            Some(0.0)
        );
    }

    #[test]
    fn wall_disagreement_percent_is_relative_to_faster_run() {
        use std::time::Duration;
        let pct = wall_disagreement_percent(Duration::from_millis(100), Duration::from_millis(105))
            .expect("finite");
        assert!((pct - 5.0).abs() < 0.0001, "expected 5.0, got {pct}");
    }

    #[test]
    fn wall_disagreement_percent_is_symmetric() {
        use std::time::Duration;
        let a = wall_disagreement_percent(Duration::from_millis(200), Duration::from_millis(250));
        let b = wall_disagreement_percent(Duration::from_millis(250), Duration::from_millis(200));
        assert_eq!(a, b);
    }

    #[test]
    fn wall_disagreement_percent_returns_none_on_zero_duration() {
        use std::time::Duration;
        assert_eq!(
            wall_disagreement_percent(Duration::ZERO, Duration::from_millis(100)),
            None
        );
        assert_eq!(
            wall_disagreement_percent(Duration::from_millis(100), Duration::ZERO),
            None
        );
        assert_eq!(
            wall_disagreement_percent(Duration::ZERO, Duration::ZERO),
            None
        );
    }

    #[test]
    fn parse_bench_result_round_trips_every_boot_outcome() {
        let variants = [
            BootOutcome::ProcessExit,
            BootOutcome::Fault,
            BootOutcome::MaxSteps,
            BootOutcome::RsxWriteCheckpoint,
            BootOutcome::PcReached(0x10381ce8),
            BootOutcome::TimeOverflow,
        ];
        for v in variants {
            let line = format!("BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1000 outcome={v}\n");
            let r = parse_bench_result(&line)
                .unwrap_or_else(|e| panic!("round-trip parse failed for {v:?}: {e}"));
            assert_eq!(r.outcome, v, "round-trip mismatch for {v:?}");
        }
    }

    #[test]
    fn parse_bench_result_extracts_fields() {
        let stdout = "some preamble\nBENCH_RESULT steps=1402388 wall_ms=323 steps_per_sec=4342377 outcome=ProcessExit\ntrailing noise\n";
        let r = parse_bench_result(stdout).expect("parses");
        assert_eq!(r.steps, 1402388);
        assert_eq!(r.wall.as_millis(), 323);
        assert_eq!(r.outcome, BootOutcome::ProcessExit);
    }

    #[test]
    fn parse_bench_result_errors_on_missing_line() {
        let stdout = "just some noise\nbut no result line\n";
        assert_eq!(
            parse_bench_result(stdout).unwrap_err(),
            ParseBenchError::NoResultLine
        );
    }

    #[test]
    fn parse_bench_result_errors_on_duplicate_line() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n\
                      BENCH_RESULT steps=2 wall_ms=2 steps_per_sec=1 outcome=ProcessExit\n";
        assert_eq!(
            parse_bench_result(stdout).unwrap_err(),
            ParseBenchError::DuplicateResultLine
        );
    }

    #[test]
    fn parse_bench_result_errors_on_unknown_outcome() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=WhoKnows\n";
        match parse_bench_result(stdout).unwrap_err() {
            ParseBenchError::UnparseableOutcome { token, source: _ } => {
                assert_eq!(token, "WhoKnows");
            }
            other => panic!("expected UnparseableOutcome, got {other:?}"),
        }
    }

    #[test]
    fn parse_bench_result_errors_on_malformed_steps() {
        let stdout = "BENCH_RESULT steps=abc wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n";
        match parse_bench_result(stdout).unwrap_err() {
            ParseBenchError::MalformedSteps(s) => assert_eq!(s, "abc"),
            other => panic!("expected MalformedSteps, got {other:?}"),
        }
    }

    #[test]
    fn parse_bench_result_errors_on_missing_steps() {
        let stdout = "BENCH_RESULT wall_ms=1 steps_per_sec=1 outcome=ProcessExit\n";
        assert_eq!(
            parse_bench_result(stdout).unwrap_err(),
            ParseBenchError::MissingSteps
        );
    }

    #[test]
    fn parse_bench_result_errors_on_malformed_wall_ms() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=xyz steps_per_sec=1 outcome=ProcessExit\n";
        match parse_bench_result(stdout).unwrap_err() {
            ParseBenchError::MalformedWallMs(s) => assert_eq!(s, "xyz"),
            other => panic!("expected MalformedWallMs, got {other:?}"),
        }
    }

    #[test]
    fn parse_bench_result_errors_on_missing_wall_ms() {
        let stdout = "BENCH_RESULT steps=1 steps_per_sec=1 outcome=ProcessExit\n";
        assert_eq!(
            parse_bench_result(stdout).unwrap_err(),
            ParseBenchError::MissingWallMs
        );
    }

    #[test]
    fn parse_bench_result_errors_on_missing_outcome() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1\n";
        assert_eq!(
            parse_bench_result(stdout).unwrap_err(),
            ParseBenchError::MissingOutcome
        );
    }

    #[test]
    fn classify_pair_pass() {
        use std::time::Duration;
        let r1 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        let r2 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(102),
            outcome: BootOutcome::ProcessExit,
        };
        let drift = wall_disagreement_percent(r1.wall, r2.wall);
        assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::Pass);
    }

    #[test]
    fn classify_pair_determinism_break_on_step_mismatch() {
        use std::time::Duration;
        let r1 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        let r2 = BenchBootResult {
            steps: 11,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        let drift = wall_disagreement_percent(r1.wall, r2.wall);
        assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::DeterminismBreak);
    }

    #[test]
    fn classify_pair_determinism_break_on_outcome_mismatch() {
        use std::time::Duration;
        let r1 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        let r2 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::MaxSteps,
        };
        let drift = wall_disagreement_percent(r1.wall, r2.wall);
        assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::DeterminismBreak);
    }

    #[test]
    fn classify_pair_wall_drift_exceeded() {
        use std::time::Duration;
        let r1 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        let r2 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(200),
            outcome: BootOutcome::ProcessExit,
        };
        let drift = wall_disagreement_percent(r1.wall, r2.wall);
        assert_eq!(classify_pair(&r1, &r2, drift), BenchGate::WallDriftExceeded);
    }

    #[test]
    fn classify_pair_wall_unmeasurable() {
        use std::time::Duration;
        let r1 = BenchBootResult {
            steps: 10,
            wall: Duration::ZERO,
            outcome: BootOutcome::ProcessExit,
        };
        let r2 = BenchBootResult {
            steps: 10,
            wall: Duration::from_millis(100),
            outcome: BootOutcome::ProcessExit,
        };
        assert_eq!(classify_pair(&r1, &r2, None), BenchGate::WallUnmeasurable);
    }
}
