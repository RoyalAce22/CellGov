//! The `cellgov` command tree.
//!
//! Clap types live in this module and nowhere else. Every command's
//! behavior is a function over the plain structs declared here.

mod boot;
mod dev;
mod diff;
mod store;
mod value;

use clap::{FromArgMatches, Parser, Subcommand};

pub(crate) use boot::{BenchArgs, BenchGateArgs, BootRunArgs, BootSelection, TitleSelector};
pub(crate) use dev::{
    CliGenArgs, CompletionShell, CompletionsArgs, DevCommand, DisasmArgs, FixtureGenArgs,
    FuncsArgs, GenManifestArgs, PrxImportsArgs, RecordAnchorsArgs, Rpcs3AttributeArgs,
    TitlesGenArgs, MAX_DISASM_COUNT,
};
pub(crate) use diff::{
    CompareArgs, DiffCommand, ExploreArgs, ExploreCommand, OutputFormat, ScenarioCommand,
};
pub(crate) use store::{
    FirmwareCommand, FirmwareUninstallArgs, InstallContainerArgs, KeysCommand, KeysPathArgs,
    SelfCommand, SelfDecryptArgs, TitleCommand, TitleInstallArgs, UninstallArgs, VfsOutput,
};

/// Print `msg` to stderr and exit with the usage status.
pub(crate) fn die_usage(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(crate::cli::exit_codes::USAGE)
}

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

/// Parse `argv` into the command tree.
///
/// This function does not always return:
///
/// - A usage error exits [`crate::cli::exit_codes::USAGE`].
/// - `--help` and `--version` print their text and exit 0.
pub(crate) fn parse_or_exit(argv: &[String]) -> Cli {
    let cli = match try_parse(argv) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    if let Some(refusal) = global_refusal(&cli) {
        die_usage(&refusal);
    }
    cli
}

/// Parse `argv` against the tree carrying each command's examples, so
/// `--help` and `docs/cli.md` show the same invocations.
pub(crate) fn try_parse(argv: &[String]) -> Result<Cli, clap::Error> {
    let matches = crate::cli::reference::command_tree().try_get_matches_from(argv)?;
    // `Cli::from_arg_matches` raises an unformatted error: no usage
    // line, no "try --help". `Parser::try_parse_from` formats every such
    // error through the command, so this path formats it here instead.
    Cli::from_arg_matches(&matches)
        .map_err(|e| e.format(&mut crate::cli::reference::command_tree()))
}

/// Why this invocation's globals do not fit its command, or `None`
/// when they do.
///
/// clap propagates a global spelled before the subcommand into that
/// subcommand's matcher only after parsing, so a `conflicts_with` on
/// the subcommand's own flag never sees it. These checks run on the
/// parsed tree instead.
pub(crate) fn global_refusal(cli: &Cli) -> Option<String> {
    let g = &cli.globals;
    if g.vfs_root.is_some() && !reads_vfs_root(&cli.command) {
        return Some(format!("--vfs-root applies to {VFS_ROOT_READERS} only"));
    }
    if g.vfs_root.is_some() && names_its_own_store_root(&cli.command) {
        return Some(
            "--output and --vfs-root both name a root; pass one. --output is the store \
             root, --vfs-root the PS3 mount one level inside it."
                .to_string(),
        );
    }
    if g.format != OutputFormat::Human && !reads_format(&cli.command) {
        return Some(format!("--format applies to {FORMAT_READERS} only"));
    }
    if g.quiet && !reads_quiet(&cli.command) {
        return Some(format!("--quiet applies to {QUIET_READERS} only"));
    }
    if g.verbose && !reads_verbose(&cli.command) {
        return Some(format!("--verbose applies to {VERBOSE_READERS} only"));
    }
    None
}

/// Whether `--output` on this command already names a store root.
fn names_its_own_store_root(command: &Command) -> bool {
    match command {
        Command::Firmware(FirmwareCommand::Install(a)) => a.output.output.is_some(),
        Command::Firmware(
            FirmwareCommand::List
            | FirmwareCommand::Show { .. }
            | FirmwareCommand::Verify { .. }
            | FirmwareCommand::Uninstall(_),
        ) => false,
        Command::Title(title) => match title {
            TitleCommand::Install(a) => a.output.output.is_some(),
            TitleCommand::InstallUpdate(a) => a.output.output.is_some(),
            TitleCommand::Uninstall(a) => a.output.output.is_some(),
            TitleCommand::List | TitleCommand::Show { .. } | TitleCommand::Verify { .. } => false,
        },
        Command::Keys(keys) => match keys {
            KeysCommand::Show { output, .. } | KeysCommand::Remove { output } => {
                output.output.is_some()
            }
            KeysCommand::Import(a) => a.output.output.is_some(),
        },
        Command::Status
        | Command::SelfCmd(_)
        | Command::Boot(_)
        | Command::Diff(_)
        | Command::Explore(_)
        | Command::Scenario(_)
        | Command::Dev(_) => false,
    }
}

/// The commands [`reads_vfs_root`] answers for, as help text.
const VFS_ROOT_READERS: &str =
    "the commands that open a guest image or write to the store: firmware, title, keys, self, boot, \
     and dev disasm / prx-imports / funcs / fixture-gen";

/// The commands [`reads_format`] answers for, as help text.
const FORMAT_READERS: &str = "status, the firmware and title list / show / verify commands, \
     diff compare, diff observations, and explore";

/// The commands [`reads_quiet`] answers for, as help text.
const QUIET_READERS: &str = "status, firmware install, title install, title install-update, \
     the boot family, and dev record-anchors";

/// The commands [`reads_verbose`] answers for, as help text.
const VERBOSE_READERS: &str = "firmware install";

/// Whether `--quiet` silences anything `command` would print.
fn reads_quiet(command: &Command) -> bool {
    matches!(command, Command::Status) || renders_progress(command)
}

/// Whether `command` renders a progress bar.
fn renders_progress(command: &Command) -> bool {
    match command {
        Command::Firmware(fw) => matches!(fw, FirmwareCommand::Install(_)),
        Command::Title(title) => matches!(
            title,
            TitleCommand::Install(_) | TitleCommand::InstallUpdate(_)
        ),
        Command::Boot(_) => true,
        Command::Dev(dev) => matches!(dev, DevCommand::RecordAnchors(_)),
        Command::Status
        | Command::Keys(_)
        | Command::SelfCmd(_)
        | Command::Diff(_)
        | Command::Explore(_)
        | Command::Scenario(_) => false,
    }
}

/// Whether `command` prints anything more under `--verbose`.
fn reads_verbose(command: &Command) -> bool {
    matches!(command, Command::Firmware(FirmwareCommand::Install(_)))
}

/// Whether `command` resolves a PS3 VFS root.
fn reads_vfs_root(command: &Command) -> bool {
    match command {
        Command::Status
        | Command::Firmware(_)
        | Command::Title(_)
        | Command::Keys(_)
        | Command::SelfCmd(_)
        | Command::Boot(_) => true,
        Command::Dev(dev) => matches!(
            dev,
            DevCommand::Disasm(_)
                | DevCommand::PrxImports(_)
                | DevCommand::Funcs(_)
                | DevCommand::FixtureGen(_)
        ),
        Command::Diff(_) | Command::Explore(_) | Command::Scenario(_) => false,
    }
}

/// Whether `command` renders a report `--format` selects between.
fn reads_format(command: &Command) -> bool {
    match command {
        Command::Diff(diff) => matches!(
            diff,
            DiffCommand::Compare(_) | DiffCommand::Observations { .. }
        ),
        Command::Status | Command::Explore(_) => true,
        Command::Firmware(fw) => matches!(
            fw,
            FirmwareCommand::List | FirmwareCommand::Show { .. } | FirmwareCommand::Verify { .. }
        ),
        Command::Title(title) => matches!(
            title,
            TitleCommand::List | TitleCommand::Show { .. } | TitleCommand::Verify { .. }
        ),
        Command::Keys(_)
        | Command::SelfCmd(_)
        | Command::Boot(_)
        | Command::Scenario(_)
        | Command::Dev(_) => false,
    }
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
