use std::path::Path;

use cellgov_time::Budget;
use clap::Parser as _;

use super::super::test_fixtures::{bench_manifest, bench_options, test_cell};
use super::*;
use crate::cli::parse::{BootCommand, Cli, Command};
use crate::game::manifest;

#[test]
fn the_child_receives_the_selection_flags_not_the_resolved_firmware_dir() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.firmware_dir = Some("resolved/4.91/dev_flash/sys/external");
    opts.selection = SelectionArgs {
        fw: Some("4.91"),
        game_ver: Some("02.51"),
        firmware_dir: None,
        vfs_root: Some("elsewhere/dev_hdd0"),
    };
    let mut cmd = std::process::Command::new("cellgov_cli");
    opts.encode_to_command(&mut cmd);
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let forwarded = |flag: &str, value: &str| {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    };
    assert!(
        forwarded("--vfs-root", "elsewhere/dev_hdd0"),
        "got {args:?}"
    );
    assert!(forwarded("--fw", "4.91"), "got {args:?}");
    assert!(forwarded("--game-ver", "02.51"), "got {args:?}");
    assert!(
        !args.iter().any(|a| a == "--firmware-dir"),
        "the resolved module directory must not reach the child: {args:?}"
    );
}

/// Every forwarded flag must survive the round trip. A run set
/// re-enters the binary as a child process, and it forwards the
/// selection flags for the child to resolve on its own. A spelling
/// the child parses differently makes the runs measure different
/// things while the gate still reports agreement. See
/// `docs/architecture/title_harness.md`, "Title anchors and
/// witnesses".
#[test]
fn the_encoded_child_invocation_parses_back_into_the_same_run() {
    let title = bench_manifest(Some(4_000));
    let identity = cellgov_compare::RunIdentity::default();
    let guest_args = vec!["--trace".to_string(), "argv1".to_string()];
    let cell = CellKey {
        fw: "4.91".to_string(),
        game_ver: Some("02.51".to_string()),
    };
    let opts = BenchOptions {
        title: &title,
        elf_path: "EBOOT.BIN",
        max_steps: 4_000,
        plan: AnchorPlan {
            cell: Some(&cell),
            max_steps: 4_000,
            checkpoint: manifest::CheckpointTrigger::ProcessExit,
        },
        // The resolved directory, which must not reach the child.
        firmware_dir: Some("resolved/4.91/dev_flash/sys/external"),
        composed_mounts: &[],
        identity: &identity,
        selection: SelectionArgs {
            fw: Some("4.91"),
            game_ver: Some("02.51"),
            firmware_dir: None,
            vfs_root: Some("elsewhere/dev_hdd0"),
        },
        strict_reserved: true,
        checkpoint_override: Some(manifest::CheckpointTrigger::Pc(0x1_0000)),
        budget_override: Some(Budget::new(512)),
        prescan: true,
        guest_args: &guest_args,
        check_anchor: true,
        run_index: 0,
    };

    let mut cmd = std::process::Command::new("cellgov");
    opts.encode_to_command(&mut cmd);
    let mut argv = vec!["cellgov".to_string()];
    argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));

    let cli = Cli::try_parse_from(&argv)
        .unwrap_or_else(|e| panic!("child argv {argv:?} does not parse: {e}"));
    assert_eq!(
        cli.globals.vfs_root.as_deref(),
        Some(Path::new("elsewhere/dev_hdd0")),
    );
    // `bench-once`, never `bench`: the gating set must not spawn
    // another gating set.
    let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
        panic!("child argv {argv:?} did not select `boot bench-once`");
    };
    assert_eq!(child.run_index, Some(0));
    assert_eq!(child.selector.title.as_deref(), Some(title.name()));
    assert_eq!(child.selector.content_id, None);
    assert_eq!(child.selector.title_manifest, None);
    assert_eq!(child.selection.fw.as_deref(), Some("4.91"));
    assert_eq!(child.selection.game_ver.as_deref(), Some("02.51"));
    assert_eq!(child.selection.firmware_dir, None);
    assert_eq!(child.max_steps, Some(4_000));
    assert_eq!(child.budget, Some(512));
    assert_eq!(
        child.checkpoint,
        Some(manifest::CheckpointTrigger::Pc(0x1_0000)),
    );
    assert!(child.prescan);
    assert!(child.strict_reserved);
    assert_eq!(child.guest_arg, guest_args);
    // A child bar renders into the pipe the parent parses.
    assert!(cli.globals.no_progress);
}

#[test]
fn an_unmanaged_firmware_tree_reaches_the_child_as_the_flag() {
    let title = bench_manifest(Some(4_000));
    let identity = cellgov_compare::RunIdentity::default();
    let opts = BenchOptions {
        title: &title,
        elf_path: "EBOOT.BIN",
        max_steps: 4_000,
        plan: AnchorPlan {
            cell: None,
            max_steps: 4_000,
            checkpoint: manifest::CheckpointTrigger::ProcessExit,
        },
        firmware_dir: Some("elsewhere/sys/external"),
        composed_mounts: &[],
        identity: &identity,
        selection: SelectionArgs {
            firmware_dir: Some("elsewhere/sys/external"),
            ..SelectionArgs::default()
        },
        strict_reserved: false,
        checkpoint_override: None,
        budget_override: None,
        prescan: false,
        guest_args: &[],
        check_anchor: true,
        run_index: 0,
    };

    let mut cmd = std::process::Command::new("cellgov");
    opts.encode_to_command(&mut cmd);
    let mut argv = vec!["cellgov".to_string()];
    argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));

    let cli = Cli::try_parse_from(&argv)
        .unwrap_or_else(|e| panic!("child argv {argv:?} does not parse: {e}"));
    let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
        panic!("child argv {argv:?} did not select `boot bench-once`");
    };
    assert_eq!(
        child.selection.firmware_dir.as_deref(),
        Some(Path::new("elsewhere/sys/external")),
    );
    assert_eq!(child.selection.fw, None);
}
