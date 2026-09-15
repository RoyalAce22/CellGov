//! The boot override flags, parsed on every boot command and spelled
//! back by `override_flags`.

use clap::Parser;

use super::*;
use cellgov_compare::BootOverrides;

fn parse(argv: &[&str]) -> Result<Cli, clap::Error> {
    let mut full = vec!["cellgov"];
    full.extend_from_slice(argv);
    Cli::try_parse_from(full)
}

fn every_override() -> BootOverrides {
    BootOverrides {
        skip_module_start: true,
        force_system_authid: true,
        prx_base: Some(0x3000_0000),
        disable_module_start_hle_stubs: true,
    }
}

/// `override_flags(overrides)` as argv tokens.
fn argv_of(overrides: &BootOverrides) -> Vec<String> {
    let mut out = Vec::new();
    for (flag, value) in override_flags(overrides) {
        out.push(flag.to_string());
        out.extend(value);
    }
    out
}

/// What `boot <subcommand> --title t <flags>` parses into.
fn parsed(subcommand: &str, flags: &[String]) -> BootOverrides {
    let mut argv = vec!["boot", subcommand, "--title", "t"];
    argv.extend(flags.iter().map(String::as_str));
    let cli = parse(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    match cli.command {
        Command::Boot(BootCommand::Run(args)) => args.overrides.overrides(),
        Command::Boot(BootCommand::Bench(args)) => args.bench.overrides.overrides(),
        Command::Boot(BootCommand::BenchOnce(args)) => args.overrides.overrides(),
        _ => panic!("{argv:?} parsed as another command"),
    }
}

#[test]
fn no_override_flag_parses_as_no_override() {
    for subcommand in ["run", "bench", "bench-once"] {
        assert!(parsed(subcommand, &[]).is_empty(), "boot {subcommand}");
    }
}

#[test]
fn the_spelled_flags_parse_back_into_the_same_set_on_every_boot_command() {
    let each = [
        BootOverrides {
            skip_module_start: true,
            ..BootOverrides::default()
        },
        BootOverrides {
            force_system_authid: true,
            ..BootOverrides::default()
        },
        BootOverrides {
            prx_base: Some(0x3fff_0000),
            ..BootOverrides::default()
        },
        BootOverrides {
            disable_module_start_hle_stubs: true,
            ..BootOverrides::default()
        },
        every_override(),
    ];
    for overrides in each {
        for subcommand in ["run", "bench", "bench-once"] {
            assert_eq!(
                parsed(subcommand, &argv_of(&overrides)),
                overrides,
                "boot {subcommand} {:?}",
                argv_of(&overrides)
            );
        }
    }
}

#[test]
fn a_prx_base_reads_as_hex_with_or_without_the_prefix() {
    for (value, base) in [
        ("30000000", 0x3000_0000),
        ("0x30000000", 0x3000_0000),
        ("0X3FFF0000", 0x3fff_0000),
    ] {
        let flags = ["--prx-base".to_string(), value.to_string()];
        assert_eq!(
            parsed("run", &flags).prx_base,
            Some(base),
            "--prx-base {value}"
        );
    }
}

#[test]
fn a_prx_base_that_is_not_hex_is_a_usage_error() {
    for value in ["", "zzz", "0x1_0000", "FFFFFFFFFFFFFFFFF"] {
        assert_eq!(
            parse(&["boot", "run", "--title", "t", "--prx-base", value])
                .expect_err(value)
                .kind(),
            clap::error::ErrorKind::ValueValidation,
            "--prx-base {value:?}"
        );
    }
}

#[test]
fn the_override_heading_lists_the_override_flags_and_nothing_else() {
    let cli = <Cli as clap::CommandFactory>::command();
    let boot = cli.find_subcommand("boot").expect("boot is a subcommand");
    for verb in ["run", "bench", "bench-once"] {
        let cmd = boot.find_subcommand(verb).expect(verb);
        let mut under: Vec<&str> = cmd
            .get_arguments()
            .filter(|a| a.get_help_heading() == Some(super::boot::BOOT_OVERRIDE_HEADING))
            .map(|a| a.get_id().as_str())
            .collect();
        under.sort_unstable();
        assert_eq!(
            under,
            [
                "disable_module_start_hle_stubs",
                "force_system_authid",
                "prx_base",
                "skip_module_start",
            ],
            "boot {verb}"
        );
    }
}
