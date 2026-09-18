//! The `cellgov` command tree.
//!
//! Clap types live under `cli::parse` and nowhere else. Every command's
//! behavior is a function over the plain structs these modules declare.

mod boot;
mod dev;
mod diff;
mod entry;
mod globals;
mod store;
mod tree;
mod value;

pub(crate) use boot::{
    override_flags, BenchArgs, BenchGateArgs, BootRunArgs, BootSelection, TitleSelector,
};
#[cfg(feature = "decrypt")]
pub(crate) use dev::Lv2ExtractArgs;
pub(crate) use dev::{
    CliGenArgs, CompletionShell, CompletionsArgs, DevCommand, DisasmArgs, FixtureGenArgs,
    FuncsArgs, GenManifestArgs, PrxImportsArgs, RecordAnchorsArgs, Rpcs3AttributeArgs,
    TitlesGenArgs, MAX_DISASM_COUNT,
};
pub(crate) use diff::{
    CompareArgs, DiffCommand, ExploreArgs, ExploreCommand, ExploreTitleArgs, OutputFormat,
    ScenarioCommand,
};
#[cfg(test)]
pub(crate) use entry::try_parse;
pub(crate) use entry::{die_usage, parse_or_exit};
#[cfg(test)]
pub(crate) use globals::global_refusal;
pub(crate) use store::{
    FirmwareCommand, FirmwareInstallArgs, FirmwareUninstallArgs, InstallContainerArgs, KeysCommand,
    KeysPathArgs, SelfCommand, SelfDecryptArgs, TitleCommand, TitleInstallArgs, UninstallArgs,
    VfsOutput,
};
pub(crate) use tree::{BootCommand, Cli, Command, Globals};

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/boot_override_parse_tests.rs"]
mod boot_override_parse_tests;

#[cfg(test)]
#[path = "tests/explore_globals_tests.rs"]
mod explore_globals_tests;
