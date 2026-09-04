//! The clap types: the root, its global flags, and the noun-verb tree.

use clap::{Parser, Subcommand};

use super::boot::{BenchArgs, BenchGateArgs, BootRunArgs};
use super::dev::DevCommand;
use super::diff::{DiffCommand, ExploreArgs, OutputFormat, ScenarioCommand};
use super::store::{FirmwareCommand, KeysCommand, SelfCommand, TitleCommand};

/// The deterministic oracle's one command-line entry point.
#[derive(Debug, Parser)]
#[command(
    name = "cellgov",
    bin_name = "cellgov",
    version,
    about = "Deterministic PS3 oracle: install a corpus, boot it, and diff the result.",
    long_about = None,
    propagate_version = true,
    disable_help_subcommand = false,
    after_help = crate::cli::exit_codes::CONTRACT,
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Command,
}

const VFS_ROOT_HELP: &str = if cfg!(feature = "decrypt") {
    "PS3 VFS root; NPDRM inputs resolve their RAP and the key vault under it \
     (default: CELLGOV_PS3_VFS_ROOT, then vfs/dev_hdd0)"
} else {
    "PS3 VFS root (default: CELLGOV_PS3_VFS_ROOT, then vfs/dev_hdd0)"
};

/// Flags every command accepts, in any position.
#[derive(Debug, Clone, Default, clap::Args)]
#[command(next_help_heading = "Global options")]
pub(crate) struct Globals {
    #[arg(long, global = true, value_name = "DIR", help = VFS_ROOT_HELP)]
    pub vfs_root: Option<std::path::PathBuf>,
    /// Report rendering.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
    /// Suppress progress and other non-essential stderr.
    #[arg(long, global = true)]
    pub quiet: bool,
    /// Print more detail about what the command did.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
    /// Never emit SGR colour sequences.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Never render a progress bar.
    #[arg(long, global = true)]
    pub no_progress: bool,
    /// Never prompt; a needed confirmation becomes a usage error.
    #[arg(long, global = true)]
    pub no_input: bool,
    /// Answer every confirmation prompt yes.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
}

impl Globals {
    /// The render decision these flags feed.
    ///
    /// `json` is always false: no command that renders a progress bar
    /// accepts `--format`.
    pub fn render(&self) -> cellgov_terminal::caps::RenderFlags {
        cellgov_terminal::caps::RenderFlags {
            no_progress: self.no_progress,
            no_color: self.no_color,
            quiet: self.quiet,
            json: false,
        }
    }
}

/// The top-level nouns.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// What this machine holds, and which cells have an anchor.
    Status,
    /// Installed PS3 system software.
    #[command(subcommand)]
    Firmware(FirmwareCommand),
    /// Installed games and their updates.
    #[command(subcommand)]
    Title(TitleCommand),
    /// The operator's key vault.
    #[command(subcommand)]
    Keys(KeysCommand),
    /// Operations on a SELF outside the store.
    #[command(subcommand)]
    #[command(name = "self")]
    SelfCmd(SelfCommand),
    /// Boot a title through the deterministic runtime.
    #[command(subcommand)]
    Boot(BootCommand),
    /// Compare two runs.
    #[command(subcommand)]
    Diff(DiffCommand),
    /// Explore a scenario's schedule space.
    Explore(ExploreArgs),
    /// Run a synthetic SPU/PPU scenario.
    #[command(subcommand)]
    Scenario(ScenarioCommand),
    /// Maintainer tooling.
    #[command(subcommand)]
    Dev(DevCommand),
}

/// `cellgov boot ...`
#[derive(Debug, Subcommand)]
pub(crate) enum BootCommand {
    /// Boot a title and report where it stopped.
    Run(Box<BootRunArgs>),
    /// Boot a title several times and gate the set against its anchor.
    Bench(Box<BenchGateArgs>),
    /// One bench measurement, with no run set around it and no anchor
    /// gate.
    BenchOnce(Box<BenchArgs>),
}
