//! `cellgov` -- install a PS3 game, boot it through the
//! deterministic runtime, and diff the result.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "CLI binary: stdout/stderr are the user-facing output channel"
)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

#[cfg(feature = "decrypt")]
mod caller_census;
mod cli;
mod composition;
mod disasm;
mod dump_prx_imports;
mod env_vars;
mod funcs;
mod game;
mod lv2_census;
mod lv2_discover;
#[cfg(feature = "decrypt")]
mod lv2_extract;
mod oracle_gap;
mod paths;
mod progress;
mod stub_class;

use std::path::Path;

use cli::parse::{
    self, BootCommand, Cli, Command, DevCommand, DiffCommand, FirmwareCommand, Globals,
    KeysCommand, ScenarioCommand, SelfCommand, TitleCommand,
};
use cli::scenarios::{report, run_scenario, SCENARIOS};
use cli::store::confirm::Answers;
use cli::store::read;

fn main() {
    let argv = collect_args_or_die();
    let Cli { globals, command } = parse::parse_or_exit(&argv);
    dispatch(&command, &globals);
}

fn dispatch(command: &Command, globals: &Globals) {
    let vfs_flag = globals.vfs_root.as_deref();
    let answers = Answers {
        yes: globals.yes,
        no_input: globals.no_input,
    };
    match command {
        Command::Status => {
            read::status(&read::store_root(vfs_flag), globals.format, globals.quiet);
        }
        Command::Firmware(FirmwareCommand::List) => {
            read::firmware_list(&read::store_root(vfs_flag), globals.format);
        }
        Command::Firmware(FirmwareCommand::Show { version }) => {
            read::firmware_show(&read::store_root(vfs_flag), version, globals.format);
        }
        Command::Firmware(FirmwareCommand::Verify { version }) => {
            read::firmware_verify(&read::store_root(vfs_flag), version, globals.format);
        }
        Command::Firmware(FirmwareCommand::VerifyCorpus { corpus }) => {
            read::firmware_verify_corpus(&read::store_root(vfs_flag), corpus, globals.format);
        }
        Command::Firmware(FirmwareCommand::Kernels) => {
            read::firmware_kernels(&read::store_root(vfs_flag), globals.format);
        }
        Command::Firmware(FirmwareCommand::Uninstall(args)) => {
            cli::store::uninstall::firmware(args, &read::store_root(vfs_flag), answers);
        }
        Command::Title(TitleCommand::List) => {
            read::title_list(&read::store_root(vfs_flag), globals.format);
        }
        Command::Title(TitleCommand::Show { title_id }) => {
            read::title_show(&read::store_root(vfs_flag), title_id, globals.format);
        }
        Command::Title(TitleCommand::Verify { title_id, ver }) => {
            read::title_verify(
                &read::store_root(vfs_flag),
                title_id,
                ver.as_deref(),
                globals.format,
            );
        }
        Command::Firmware(FirmwareCommand::Install(args)) => {
            let store = store_root(&args.output, vfs_flag);
            cli::store::firmware::install(args, &store, globals.render(), globals.verbose);
        }
        Command::Title(TitleCommand::Install(args)) => {
            let store = store_root(&args.output, vfs_flag);
            cli::store::title::install(args, &store, globals.render());
        }
        Command::Title(TitleCommand::InstallUpdate(args)) => {
            let store = store_root(&args.output, vfs_flag);
            cli::store::title::install_update(args, &store, globals.render());
        }
        Command::Title(TitleCommand::Uninstall(args)) => {
            let store = store_root(&args.output, vfs_flag);
            cli::store::uninstall::title(args, &store, answers);
        }
        Command::Keys(keys) => {
            let output = match keys {
                KeysCommand::Show { output, .. } | KeysCommand::Remove { output } => output,
                KeysCommand::Import(args) => &args.output,
            };
            let store = store_root(output, vfs_flag);
            cli::store::keys_cmd::run(keys, &store);
        }
        Command::SelfCmd(SelfCommand::Decrypt(args)) => {
            let vfs_root = cli::title::resolve_ps3_vfs_root(vfs_flag);
            let store = cli::keys::install_root_of(&vfs_root);
            cli::store::self_decrypt::run(args, &vfs_root, &store);
        }
        Command::Boot(BootCommand::Run(args)) => {
            cli::boot_cmd::run_game(args, vfs_flag, globals.render());
        }
        Command::Boot(BootCommand::Bench(args)) => {
            cli::boot_cmd::bench_boot(args, vfs_flag, globals.render());
        }
        Command::Boot(BootCommand::BenchOnce(args)) => {
            cli::boot_cmd::bench_boot_once(args, vfs_flag, globals.render());
        }
        Command::Diff(DiffCommand::Compare(args)) => {
            cli::compare::run(args, globals.format, SCENARIOS);
        }
        Command::Diff(DiffCommand::Observations { a, b }) => {
            cli::compare::run_compare_observations(a, b, globals.format);
        }
        Command::Diff(DiffCommand::Diverge { a, b }) => cli::compare::run_diverge(a, b),
        Command::Diff(DiffCommand::Zoom { a, b, step }) => cli::compare::run_zoom(a, b, *step),
        Command::Explore(args) => cli::explore::run(args, globals.format, SCENARIOS, vfs_flag),
        Command::Scenario(ScenarioCommand::List) => {
            for name in SCENARIOS {
                println!("{name}");
            }
        }
        Command::Scenario(ScenarioCommand::Run { name }) => match run_scenario(name) {
            Some((label, result)) => println!("{}", report(label, &result)),
            None => cli::exit::die(&format!(
                "unknown scenario: {name}\navailable: {}",
                SCENARIOS.join(", ")
            )),
        },
        Command::Scenario(ScenarioCommand::Dump { name }) => cli::dump::run(name, SCENARIOS),
        Command::Dev(dev) => dispatch_dev(dev, vfs_flag, globals),
    }
}

fn dispatch_dev(dev: &DevCommand, vfs_flag: Option<&Path>, globals: &Globals) {
    match dev {
        DevCommand::Disasm(args) => disasm::run(args, vfs_flag),
        DevCommand::PrxImports(args) => dump_prx_imports::run(args, vfs_flag),
        DevCommand::Funcs(args) => funcs::run(args, vfs_flag),
        DevCommand::Lv2Discover(args) => lv2_discover::run(args, vfs_flag, globals.format),
        DevCommand::Lv2Census(args) => lv2_census::run(args, vfs_flag),
        #[cfg(feature = "decrypt")]
        DevCommand::Lv2Extract(args) => lv2_extract::run(args, vfs_flag, globals.format),
        #[cfg(feature = "decrypt")]
        DevCommand::CallerCensus(args) => caller_census::run(args, vfs_flag),
        DevCommand::Rpcs3Attribute(args) => cli::rpcs3_attribute::run(args),
        DevCommand::FixtureGen(args) => cli::fixture_gen::run(args, vfs_flag),
        DevCommand::TitlesGen(args) => cli::titles_gen::run(args),
        DevCommand::CliGen(args) => cli::cli_gen::run(args),
        DevCommand::WorkspaceGen(args) => cli::workspace_gen::run(args),
        DevCommand::Completions(args) => cli::cli_gen::completions(args),
        DevCommand::GenManifest(args) => cli::gen_manifest::run(args, vfs_flag),
        DevCommand::RecordAnchors(args) => cli::record_anchors::run(args, globals.render()),
        DevCommand::OracleGap => oracle_gap::run(vfs_flag),
    }
}

/// The store root a command writes under. Without `--output` it is
/// the directory enclosing the resolved PS3 VFS root, so one root flag
/// serves the whole binary.
fn store_root(output: &parse::VfsOutput, vfs_flag: Option<&Path>) -> std::path::PathBuf {
    match &output.output {
        Some(dir) => dir.clone(),
        None => cli::keys::install_root_of(&cli::title::resolve_ps3_vfs_root(vfs_flag)),
    }
}

/// Materialize argv as `Vec<String>`, dying with a structured error
/// where `std::env::args` would panic on a non-UTF-8 argument.
fn collect_args_or_die() -> Vec<String> {
    let mut out = Vec::new();
    for (i, raw) in std::env::args_os().enumerate() {
        match raw.into_string() {
            Ok(s) => out.push(s),
            Err(os) => parse::die_usage(&format!(
                "argv[{i}]: not valid UTF-8 ({os:?}); cellgov accepts only UTF-8 arguments"
            )),
        }
    }
    out
}

#[cfg(test)]
#[path = "tests/main_tests.rs"]
mod tests;
