//! `cellgov firmware`, `cellgov title`, `cellgov keys`, `cellgov self`.

use std::path::PathBuf;

use crate::cli::exit::SCE_INPUT_USAGE_NOTE;

/// `--output`, the store root the installers write under.
#[derive(Debug, Clone, clap::Args)]
pub(crate) struct VfsOutput {
    /// Store root (default: the directory enclosing the PS3 VFS root).
    #[arg(long, value_name = "DIR", conflicts_with = "vfs_root")]
    pub output: Option<PathBuf>,
}

/// Which of the shared statuses a `verify` command gives, and for what.
const VERIFY_EXIT_CODES: &str = "Exit codes:
  0   every recorded artefact matched
  4   the tree diverged from what its record holds";

/// `cellgov firmware ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum FirmwareCommand {
    /// Install system software from a PS3UPDAT.PUP.
    Install(InstallContainerArgs),
    /// Name every installed firmware version.
    List,
    /// Report one installed version in full.
    Show {
        /// Version key of a `firmware/<key>/` entry.
        #[arg(id = "fw_version", value_name = "VERSION")]
        version: String,
    },
    /// Re-hash an installed version against its manifest.
    #[command(after_help = VERIFY_EXIT_CODES)]
    Verify {
        /// Version key of a `firmware/<key>/` entry.
        #[arg(id = "fw_version", value_name = "VERSION")]
        version: String,
    },
    /// Remove an installed firmware version.
    Uninstall(FirmwareUninstallArgs),
}

/// `cellgov firmware uninstall`
#[derive(Debug, clap::Args)]
pub(crate) struct FirmwareUninstallArgs {
    /// Version key of a `firmware/<key>/` entry.
    #[arg(id = "fw_version", value_name = "VERSION")]
    pub version: String,
    /// Re-hash the installed tree against its manifest first.
    #[arg(long)]
    pub verify: bool,
    /// Remove it even though a committed anchor names it, or though
    /// `--verify` found the tree diverged.
    #[arg(long)]
    pub force: bool,
    /// Print the removal plan and stop.
    #[arg(long)]
    pub dry_run: bool,
}

/// `cellgov title ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum TitleCommand {
    /// Install a base title from a PKG or a decrypted disc image.
    Install(TitleInstallArgs),
    /// Install a GD/HG update PKG over an installed base.
    InstallUpdate(InstallContainerArgs),
    /// Name every installed title, its base, and its updates.
    List,
    /// Report one installed title in full.
    Show {
        /// Title id of a store entry.
        #[arg(value_name = "TITLE_ID")]
        title_id: String,
    },
    /// Re-hash an installed title against its records.
    #[command(after_help = VERIFY_EXIT_CODES)]
    Verify {
        /// Title id of a store entry.
        #[arg(value_name = "TITLE_ID")]
        title_id: String,
        /// Check this version alone: `base`, or an update version key.
        #[arg(long, value_name = "V")]
        ver: Option<String>,
    },
    /// Remove an installed title, or one of its versions.
    Uninstall(UninstallArgs),
}

/// A container path plus the flags every installer shares.
#[derive(Debug, clap::Args)]
pub(crate) struct InstallContainerArgs {
    /// The container to install.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// Replace what is already installed there.
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub output: VfsOutput,
}

/// `cellgov title install`
#[derive(Debug, clap::Args)]
pub(crate) struct TitleInstallArgs {
    /// A PKG or a decrypted ISO; the container kind is read from the
    /// file.
    #[arg(value_name = "PKG|ISO")]
    pub path: PathBuf,
    /// RAP for a license-1/2 NPDRM title.
    #[arg(long, value_name = "PATH")]
    pub rap: Option<PathBuf>,
    /// Replace an existing install of this title.
    #[arg(long)]
    pub force: bool,
    /// Leave the system software a disc image ships uninstalled and
    /// unrecorded, instead of registering it as the firmware entry the
    /// title was certified against.
    #[arg(long)]
    pub no_firmware: bool,
    #[command(flatten)]
    pub output: VfsOutput,
}

/// `cellgov title uninstall`
///
/// Without a scope flag the title id names its base alone, which is
/// refused while updates are installed.
#[derive(Debug, clap::Args)]
pub(crate) struct UninstallArgs {
    /// Title id to remove.
    #[arg(value_name = "TITLE_ID")]
    pub title_id: String,
    /// Remove one version alone: `base`, or an update version key.
    #[arg(long, value_name = "V", conflicts_with_all = ["updates", "all"])]
    pub ver: Option<String>,
    /// Remove every installed update, keeping the base.
    #[arg(long, conflicts_with = "all")]
    pub updates: bool,
    /// Remove every update and the base.
    #[arg(long)]
    pub all: bool,
    /// Re-hash the live tree against the install record first.
    #[arg(long)]
    pub verify: bool,
    /// Leave the RAP in exdata; another title may share it.
    #[arg(long)]
    pub keep_rap: bool,
    /// Remove even when `--verify` finds a modified tree.
    #[arg(long)]
    pub force: bool,
    /// Print the removal plan and stop.
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub output: VfsOutput,
}

impl UninstallArgs {
    /// The scope the flags select.
    ///
    /// The three scope flags exclude one another at parse time.
    pub(crate) fn scope(&self) -> cellgov_install::game_uninstall::UninstallScope {
        use cellgov_install::game_uninstall::UninstallScope;
        match (&self.ver, self.updates, self.all) {
            // `base` names the base entry here as it does for
            // `title verify --ver` and `boot --game-ver`.
            (Some(v), false, false) if v == cellgov_boot::manifest::BASE_GAME_VER => {
                UninstallScope::Base
            }
            (Some(v), false, false) => UninstallScope::Update(v.clone()),
            (None, true, false) => UninstallScope::Updates,
            (None, false, true) => UninstallScope::All,
            (None, false, false) => UninstallScope::Base,
            _ => super::die_usage(
                "--ver, --updates and --all each name a different scope; pass one of them",
            ),
        }
    }
}

/// The outcomes `keys show` has beyond the shared 0-5 contract.
const KEYS_SHOW_EXIT_CODES: &str = "Exit codes particular to this command:
  40  the vault loaded, but a decrypt path would find a key missing";

/// `cellgov keys ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum KeysCommand {
    /// Inventory of the vault a decrypt would read.
    #[command(after_help = KEYS_SHOW_EXIT_CODES)]
    Show {
        /// Read this vault instead of the configured one.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
        #[command(flatten)]
        output: VfsOutput,
    },
    /// Normalize a keys file or directory into the store's vault.
    Import(KeysPathArgs),
    /// Delete the store's vault.
    Remove {
        #[command(flatten)]
        output: VfsOutput,
    },
}

/// `cellgov keys import`
#[derive(Debug, clap::Args)]
pub(crate) struct KeysPathArgs {
    /// Keys file or directory to read.
    #[arg(value_name = "PATH")]
    pub path: PathBuf,
    /// Drop the vault already installed instead of merging into it.
    #[arg(long)]
    pub replace: bool,
    #[command(flatten)]
    pub output: VfsOutput,
}

/// `cellgov self ...`
#[derive(Debug, clap::Subcommand)]
pub(crate) enum SelfCommand {
    /// Write a SELF's plaintext ELF to a file.
    Decrypt(SelfDecryptArgs),
}

/// `cellgov self decrypt`
#[derive(Debug, clap::Args)]
#[command(after_help = SCE_INPUT_USAGE_NOTE)]
pub(crate) struct SelfDecryptArgs {
    /// The SELF to decrypt.
    #[arg(value_name = "SELF")]
    pub self_path: PathBuf,
    /// Where to write the plaintext ELF (default: alongside the input).
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    /// Use this RAP instead of the one under the VFS root.
    #[arg(long, value_name = "PATH")]
    pub rap: Option<PathBuf>,
}
