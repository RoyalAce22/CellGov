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

pub(crate) use boot::{BenchArgs, BenchGateArgs, BootRunArgs, BootSelection, TitleSelector};
pub(crate) use dev::{
    CliGenArgs, CompletionShell, CompletionsArgs, DevCommand, DisasmArgs, FixtureGenArgs,
    FuncsArgs, GenManifestArgs, PrxImportsArgs, RecordAnchorsArgs, Rpcs3AttributeArgs,
    TitlesGenArgs, MAX_DISASM_COUNT,
};
pub(crate) use diff::{
    CompareArgs, DiffCommand, ExploreArgs, ExploreCommand, OutputFormat, ScenarioCommand,
};
#[cfg(test)]
pub(crate) use entry::try_parse;
pub(crate) use entry::{die_usage, parse_or_exit};
#[cfg(test)]
pub(crate) use globals::global_refusal;
pub(crate) use store::{
    FirmwareCommand, FirmwareUninstallArgs, InstallContainerArgs, KeysCommand, KeysPathArgs,
    SelfCommand, SelfDecryptArgs, TitleCommand, TitleInstallArgs, UninstallArgs, VfsOutput,
};
pub(crate) use tree::{BootCommand, Cli, Command, Globals};

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
