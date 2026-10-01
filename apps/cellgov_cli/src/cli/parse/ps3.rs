//! `cellgov ps3 ...`: the PS3 runner's verbs against a retail console.
//!
//! Each verb's flags are the runner's own, one for one; the
//! front-end parity test holds them to `runner_ps3::cli::Verb::flags`.

use std::path::PathBuf;

/// The statuses the runner's failures map to beyond the shared 0-5
/// contract.
pub(crate) const PS3_EXIT_CODES: &str = "Exit codes particular to this command:
  50  refused before changing the console: a lease, a stale result, a profile
      the console fails, an occupied game directory, an existing capture
  51  the console did not answer as the protocol requires
  52  the test left no result within the manifest's budget
  53  the fetched bytes are not one whole CGOV frame
  54  the capture succeeded, but cleanup left something on the console";

/// `cellgov ps3 ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum Ps3Command {
    /// Read the console's identity and check it against the claimed profile.
    #[command(after_help = PS3_EXIT_CODES)]
    Status(Ps3Identity),
    /// Copy a packaged microtest to the console.
    #[command(after_help = PS3_EXIT_CODES)]
    Deploy(Ps3DeployArgs),
    /// Start the deployed test and wait for its result.
    #[command(after_help = PS3_EXIT_CODES)]
    Run(Ps3RunArgs),
    /// Copy the result file to this machine.
    #[command(after_help = PS3_EXIT_CODES)]
    Fetch(Ps3FetchArgs),
    /// Remove the package and the result from the console.
    #[command(after_help = PS3_EXIT_CODES)]
    Cleanup(Ps3ManifestArgs),
    /// Deploy, run, fetch and clean up, writing a committed capture.
    #[command(after_help = PS3_EXIT_CODES)]
    Capture(Ps3CaptureArgs),
    /// Convert a fetched frame into an observation, offline.
    #[command(after_help = PS3_EXIT_CODES)]
    Convert(Ps3ConvertArgs),
    /// Remove a stale lease on a console.
    #[command(after_help = PS3_EXIT_CODES)]
    Unlock(Ps3UnlockArgs),
}

/// `--debugger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Ps3Debugger {
    /// No debugger holds the console.
    None,
    /// A debugger holds the console.
    Attached,
}

/// The identity flags every verb that talks to the console takes.
#[derive(Debug, Clone, Default, clap::Args)]
pub(crate) struct Ps3Identity {
    /// The console (default: CELLGOV_PS3_HOST).
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,
    /// The console profile this run claims (default: CELLGOV_PS3_PROFILE).
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
    /// The profiles file (default: tests/micro/console_profiles.toml under
    /// the workspace root).
    #[arg(long, value_name = "FILE")]
    pub profiles: Option<PathBuf>,
    /// The console's model, which the status page does not state.
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,
    /// The CFW name and build, which the status page does not state.
    #[arg(long, value_name = "CFW")]
    pub cfw: Option<String>,
    /// Whether a debugger holds the console.
    #[arg(long, value_enum)]
    pub debugger: Option<Ps3Debugger>,
}

/// `cellgov ps3 cleanup`, and the part every verb with a package shares.
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3ManifestArgs {
    #[command(flatten)]
    pub identity: Ps3Identity,
    /// The microtest's manifest.toml.
    #[arg(long, value_name = "FILE")]
    pub manifest: PathBuf,
}

/// `cellgov ps3 deploy`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3DeployArgs {
    #[command(flatten)]
    pub target: Ps3ManifestArgs,
    /// Empty an occupied game directory before the deploy.
    #[arg(long)]
    pub reclaim: bool,
}

/// `cellgov ps3 run`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3RunArgs {
    #[command(flatten)]
    pub target: Ps3ManifestArgs,
    /// The wait between polls for the result, in milliseconds
    /// (default: 500).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    pub poll_ms: Option<u64>,
}

/// `cellgov ps3 fetch`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3FetchArgs {
    #[command(flatten)]
    pub target: Ps3ManifestArgs,
    /// Where to write the result file.
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,
}

/// `cellgov ps3 capture`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3CaptureArgs {
    #[command(flatten)]
    pub target: Ps3ManifestArgs,
    /// The harness revision the capture records; the runner reads no git.
    #[arg(long, value_name = "SHA")]
    pub harness_revision: String,
    /// The capture directory (default: ps3/PROFILE/ beside the manifest).
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// The wait between polls for the result, in milliseconds
    /// (default: 500).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    pub poll_ms: Option<u64>,
    /// Empty an occupied game directory before the deploy.
    #[arg(long)]
    pub reclaim: bool,
    /// Leave the package on the console after the capture.
    #[arg(long)]
    pub keep_deployed: bool,
    /// Replace a committed capture; needs --reason.
    #[arg(long, requires = "reason")]
    pub recapture: bool,
    /// Why this capture replaces the committed one; the provenance records it.
    #[arg(long, value_name = "TEXT", requires = "recapture")]
    pub reason: Option<String>,
}

/// `cellgov ps3 convert`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3ConvertArgs {
    /// The fetched CGOV frame.
    #[arg(long, value_name = "FILE")]
    pub frame: PathBuf,
    /// The microtest's manifest.toml.
    #[arg(long, value_name = "FILE")]
    pub manifest: PathBuf,
    /// The console profile the capture ran under (default:
    /// CELLGOV_PS3_PROFILE).
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
    /// The profiles file (default: tests/micro/console_profiles.toml under
    /// the workspace root).
    #[arg(long, value_name = "FILE")]
    pub profiles: Option<PathBuf>,
    /// Where to write the observation (default: stdout).
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// `cellgov ps3 unlock`
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Ps3UnlockArgs {
    /// The console (default: CELLGOV_PS3_HOST).
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,
}
