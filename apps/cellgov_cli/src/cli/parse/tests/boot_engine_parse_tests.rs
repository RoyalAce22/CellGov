//! The `boot run` flags that pick the interpreter and commit paths.

use cellgov_boot::prepare::{EngineOptions, PpuShadow};
use clap::Parser;

use super::super::{BootCommand, Cli, Command};

/// The engine `boot run --title t <flags>` asks for.
fn engine(flags: &[&str]) -> EngineOptions {
    let mut argv = vec!["cellgov", "boot", "run", "--title", "t"];
    argv.extend_from_slice(flags);
    let cli = Cli::try_parse_from(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    match cli.command {
        Command::Boot(BootCommand::Run(args)) => args.engine(),
        _ => panic!("{argv:?} parsed as another command"),
    }
}

#[test]
fn no_engine_flag_asks_for_the_fused_shadow_and_the_fast_path() {
    assert_eq!(
        engine(&[]),
        EngineOptions {
            ppu_shadow: PpuShadow::Fused,
            trivial_commit_fast_path: true,
        }
    );
}

#[test]
fn each_ppu_shadow_value_names_its_passes() {
    for (value, passes) in [
        ("off", PpuShadow::Off),
        ("quickened", PpuShadow::Quickened),
        ("fused", PpuShadow::Fused),
    ] {
        assert_eq!(
            engine(&["--ppu-shadow", value]).ppu_shadow,
            passes,
            "--ppu-shadow {value}"
        );
    }
}

#[test]
fn no_trivial_commit_forbids_the_fast_path() {
    let engine = engine(&["--no-trivial-commit"]);
    assert!(!engine.trivial_commit_fast_path);
    assert_eq!(engine.ppu_shadow, PpuShadow::Fused);
}
