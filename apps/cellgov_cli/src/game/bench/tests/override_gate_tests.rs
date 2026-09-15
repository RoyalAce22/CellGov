//! A run set under a boot override forwards it to each child, and the
//! anchor gate compares none of them.

use clap::Parser as _;

use super::super::anchor::incomparable_reasons;
use super::super::test_fixtures::{bench_manifest, bench_options, test_cell};
use crate::cli::parse::{BootCommand, Cli, Command};
use cellgov_compare::{BootOverrides, RunIdentity};

fn every_override() -> BootOverrides {
    BootOverrides {
        skip_module_start: true,
        force_system_authid: true,
        prx_base: Some(0x3000_0000),
        disable_module_start_hle_stubs: true,
    }
}

fn one_of_each() -> [(&'static str, BootOverrides); 4] {
    [
        (
            "--skip-module-start",
            BootOverrides {
                skip_module_start: true,
                ..BootOverrides::default()
            },
        ),
        (
            "--force-system-authid",
            BootOverrides {
                force_system_authid: true,
                ..BootOverrides::default()
            },
        ),
        (
            "--prx-base 0x30000000",
            BootOverrides {
                prx_base: Some(0x3000_0000),
                ..BootOverrides::default()
            },
        ),
        (
            "--disable-module-start-hle-stubs",
            BootOverrides {
                disable_module_start_hle_stubs: true,
                ..BootOverrides::default()
            },
        ),
    ]
}

#[test]
fn every_boot_override_retargets_the_run_and_the_gate_names_it() {
    let cell = test_cell();
    let title = bench_manifest(None);
    for (label, overrides) in one_of_each() {
        let identity = RunIdentity {
            overrides,
            ..RunIdentity::default()
        };
        let mut opts = bench_options(&title, Some(&cell), &[]);
        opts.identity = &identity;
        assert!(opts.retargets_trajectory(), "{label}");
        let reasons = incomparable_reasons(&opts);
        assert_eq!(
            reasons.len(),
            1,
            "{label}: the run matches its cell in every other way: {reasons:?}"
        );
        assert!(reasons[0].contains(label), "{label}: {reasons:?}");
    }
}

#[test]
fn a_run_under_every_override_names_each_one() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let identity = RunIdentity {
        overrides: every_override(),
        ..RunIdentity::default()
    };
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.identity = &identity;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 4, "{reasons:?}");
    for (label, _) in one_of_each() {
        assert!(
            reasons.iter().any(|r| r.contains(label)),
            "{label}: {reasons:?}"
        );
    }
}

#[test]
fn the_child_invocation_carries_the_override_set() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let identity = RunIdentity {
        overrides: every_override(),
        ..RunIdentity::default()
    };
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.identity = &identity;

    let mut cmd = std::process::Command::new("cellgov");
    opts.encode_to_command(&mut cmd);
    let mut argv = vec!["cellgov".to_string()];
    argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));

    let cli = Cli::try_parse_from(&argv)
        .unwrap_or_else(|e| panic!("child argv {argv:?} does not parse: {e}"));
    let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
        panic!("child argv {argv:?} did not select `boot bench-once`");
    };
    assert_eq!(child.overrides.overrides(), every_override());
}

#[test]
fn a_run_under_no_override_forwards_no_override_flag() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let opts = bench_options(&title, Some(&cell), &[]);
    let mut cmd = std::process::Command::new("cellgov");
    opts.encode_to_command(&mut cmd);
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    for (flag, _) in one_of_each() {
        let flag = flag.split(' ').next().expect("a flag");
        assert!(!args.iter().any(|a| a == flag), "{flag} in {args:?}");
    }
}
