//! Which commands each global flag applies to.
//!
//! clap propagates a global spelled before the subcommand into that
//! subcommand's matcher only after the parse. A `conflicts_with` on the
//! subcommand's own flag never sees it, so these checks run on the
//! parsed tree instead.

use super::dev::DevCommand;
use super::diff::{DiffCommand, OutputFormat};
use super::store::{FirmwareCommand, KeysCommand, TitleCommand};
use super::tree::{Cli, Command};

/// Why this invocation's globals do not fit its command, or `None`
/// when they do.
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
    if g.force_ansi && !renders_progress(&cli.command) {
        return Some(format!("--force-ansi applies to {FORCE_ANSI_READERS} only"));
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
    "the commands that read or write the store, or open a guest image: status, firmware, title, \
     keys, self, boot, and dev disasm / prx-imports / funcs / fixture-gen";

/// The commands [`reads_format`] answers for, as help text.
const FORMAT_READERS: &str = "status, the firmware and title list / show / verify commands, \
     diff compare, diff observations, and explore";

/// The commands [`reads_quiet`] answers for, as help text.
const QUIET_READERS: &str = "status, firmware install, title install, title install-update, \
     the boot family, and dev record-anchors";

/// The commands [`reads_verbose`] answers for, as help text.
const VERBOSE_READERS: &str = "firmware install";

/// The commands [`renders_progress`] answers for, as help text.
const FORCE_ANSI_READERS: &str = "firmware install, title install, title install-update, \
     the boot family, and dev record-anchors";

/// Whether `--quiet` silences anything `command` would print.
fn reads_quiet(command: &Command) -> bool {
    matches!(command, Command::Status) || renders_progress(command)
}

/// Whether `command` renders a progress bar.
pub(super) fn renders_progress(command: &Command) -> bool {
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
pub(super) fn reads_vfs_root(command: &Command) -> bool {
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
pub(super) fn reads_format(command: &Command) -> bool {
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
