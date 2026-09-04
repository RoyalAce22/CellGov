//! Runs the current binary as `boot bench-once` and reads the one
//! result line back out of its streams.

use super::options::BenchOptions;
use super::result_line::{parse_bench_result, ParseBenchError};
use super::types::BenchBootResult;

/// Subprocess invocation failure surfaced by [`spawn_one_run`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
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
            Self::Io(_) => "",
            Self::SubprocessNonzero { stdout, .. } | Self::ParseFailed { stdout, .. } => stdout,
        }
    }

    pub fn captured_stderr(&self) -> &str {
        match self {
            Self::Io(_) => "",
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
/// Returns the parsed result alongside the subprocess stderr, which
/// carries the `BENCH_*` witness lines the anchor check reads.
pub(super) fn spawn_one_run(
    opts: BenchOptions<'_>,
) -> Result<(BenchBootResult, String), SpawnError> {
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
        Ok(r) => {
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
