use std::time::Duration;

use cellgov_boot::manifest;
use cellgov_compare::{BootOverrides, RunIdentity};
use cellgov_time::Budget;
use clap::Parser as _;

use super::super::test_fixtures::{bench_manifest, bench_options, set_of, test_cell};
use super::*;
use crate::cli::parse::{BootCommand, Cli, Command};

#[test]
fn every_printed_localization_argv_parses_back_into_the_run() {
    let title = bench_manifest(Some(40_000));
    let cell = test_cell();
    let guest_args = vec!["--trace".to_string(), "argument with spaces".to_string()];
    let identity = RunIdentity {
        overrides: BootOverrides {
            skip_module_start: true,
            force_system_authid: true,
            prx_base: Some(0x3000_0000),
            disable_module_start_hle_stubs: true,
        },
        ..RunIdentity::default()
    };
    let mut opts = bench_options(&title, Some(&cell), &guest_args);
    opts.identity = &identity;
    opts.selection.fw = Some("4.93");
    opts.selection.game_ver = Some("02.51");
    opts.selection.vfs_root = Some("store root/dev_hdd0");
    opts.strict_reserved = true;
    opts.checkpoint_override = Some(manifest::CheckpointTrigger::Pc(0x1_0000));
    opts.budget_override = Some(Budget::new(512));
    opts.prescan = true;
    opts.run_index = 7;

    let mut runs = set_of(&[Duration::from_millis(100)]);
    runs[0].steps = LOCALIZE_MAX_STEPS + 1;
    let lines = locate_divergence(opts, &runs).expect("localization does not interrupt");

    for i in 0..2 {
        let trace = format!("run{i}.state");
        let argv = localization_command_argv(opts, i, &trace);
        assert_eq!(lines[i + 1], format!("  {}", render_command(&argv)));
        let cli = Cli::try_parse_from(&argv)
            .unwrap_or_else(|error| panic!("localization argv {argv:?} does not parse: {error}"));
        assert!(cli.globals.no_progress);
        assert_eq!(
            cli.globals.vfs_root.as_deref(),
            Some(std::path::Path::new("store root/dev_hdd0"))
        );
        let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
            panic!("localization argv {argv:?} did not select `boot bench-once`");
        };
        assert_eq!(child.selector.title.as_deref(), Some(title.name()));
        assert_eq!(child.selection.fw.as_deref(), Some("4.93"));
        assert_eq!(child.selection.game_ver.as_deref(), Some("02.51"));
        assert_eq!(child.max_steps, Some(40_000));
        assert_eq!(child.budget, Some(512));
        assert_eq!(
            child.checkpoint,
            Some(manifest::CheckpointTrigger::Pc(0x1_0000))
        );
        assert!(child.prescan);
        assert!(child.strict_reserved);
        assert_eq!(child.guest_arg, guest_args);
        assert_eq!(child.save_state_trace.as_deref(), Some(trace.as_str()));
        assert_eq!(child.run_index, Some(1007 + i));
        assert_eq!(child.overrides.overrides(), identity.overrides);
    }
}
