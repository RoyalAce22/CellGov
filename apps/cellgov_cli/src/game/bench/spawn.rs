//! Runs the current binary as `boot bench-once` and reads the one
//! result line back out of its streams.

use cellgov_compare::bench::{parse_bench_result, BenchBootResult, ParseBenchError};

use super::options::BenchOptions;

/// Subprocess invocation failure surfaced by [`spawn_one_run`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    /// The child ended from Ctrl-C.
    #[error(transparent)]
    Command(#[from] crate::cli::exit::CommandError),
    #[error("subprocess spawn failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("subprocess exited nonzero (status={status:?})")]
    SubprocessNonzero {
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("BENCH_RESULT parse failed: {error}")]
    ParseFailed {
        #[source]
        error: ParseBenchError,
        stdout: String,
        stderr: String,
    },
}

impl SpawnError {
    pub fn captured_stdout(&self) -> &str {
        match self {
            Self::Io(_) | Self::Command(_) => "",
            Self::SubprocessNonzero { stdout, .. } | Self::ParseFailed { stdout, .. } => stdout,
        }
    }

    pub fn captured_stderr(&self) -> &str {
        match self {
            Self::Io(_) | Self::Command(_) => "",
            Self::SubprocessNonzero { stderr, .. } | Self::ParseFailed { stderr, .. } => stderr,
        }
    }
}

/// Spawn the current binary as `boot bench-once` and parse its
/// `BENCH_RESULT` line. Subprocess stderr is forwarded so warnings
/// reach the parent on the success path.
///
/// Each measurement runs in its own process. Back-to-back runs inside
/// one process drift ~60 percent in wall time on Windows, from 1 GB
/// guest-memory page-commit reuse.
///
/// # Errors
///
/// Returns an error if the child does not produce a valid result.
pub(super) fn spawn_one_run(
    opts: BenchOptions<'_>,
) -> Result<(BenchBootResult, String), SpawnError> {
    let exe = std::env::current_exe().map_err(SpawnError::Io)?;
    let mut cmd = std::process::Command::new(&exe);
    opts.encode_to_command(&mut cmd);
    let output = cmd.output().map_err(SpawnError::Io)?;
    crate::cli::exit::propagate_interrupt(output.status)?;
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
        Ok(parsed) => {
            for warning in &parsed.warnings {
                eprintln!("warning: {warning}");
            }
            let r = parsed.result;
            // The parent asked for this index on the command line. A
            // child that reports another index means `--run-index` no
            // longer reaches it. Every line would then read 0, and the
            // per-run attribution would be silently false.
            debug_assert_eq!(
                r.run_index, opts.run_index,
                "the child reported an index the parent did not ask for"
            );
            Ok((r, stderr))
        }
        Err(error) => Err(SpawnError::ParseFailed {
            error,
            stdout,
            stderr,
        }),
    }
}
