//! Every boot override moves a `boot run` off its anchor's trajectory,
//! and reaches the composed identity the boot reads its overrides from.

use clap::Parser as _;

use super::*;
use crate::cli::parse::{BootCommand, Cli, Command};

fn run_args(extra: &[&str]) -> BootRunArgs {
    let mut argv = vec!["cellgov", "boot", "run", "--title", "synthetic"];
    argv.extend_from_slice(extra);
    let cli = Cli::try_parse_from(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    let Command::Boot(BootCommand::Run(args)) = cli.command else {
        panic!("{argv:?} did not select `boot run`");
    };
    *args
}

#[test]
fn every_boot_override_retargets_the_finish_line() {
    for extra in [
        &["--skip-module-start"][..],
        &["--force-system-authid"],
        &["--prx-base", "0x30000000"],
        &["--disable-module-start-hle-stubs"],
    ] {
        assert!(run_retargets_anchor(&run_args(extra), false), "{extra:?}");
    }
}

/// A title the store does not hold, booted from the VFS root.
fn unstored_title() -> cellgov_boot::manifest::TitleManifest {
    use cellgov_boot::manifest::{CheckpointTrigger, Distribution, GameSource, TitleManifest};
    TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        system_ver: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

#[test]
fn the_composed_identity_carries_the_override_set_the_command_was_given() {
    let root = cellgov_testkit::scratch::scratch_labeled("boot-override-identity");
    std::fs::create_dir_all(root.join(".cellgov").join("installs")).unwrap();
    let external = root.join("external");
    std::fs::create_dir_all(&external).unwrap();
    let vfs = root.join("dev_hdd0");
    let title = unstored_title();
    let selection = BootSelection {
        fw: None,
        game_ver: None,
        firmware_dir: Some(external),
    };
    for overrides in [
        BootOverrides::default(),
        BootOverrides {
            skip_module_start: true,
            force_system_authid: true,
            prx_base: Some(0x3000_0000),
            disable_module_start_hle_stubs: true,
        },
    ] {
        let composition = try_resolve_composition(&selection, &vfs, &title, overrides)
            .unwrap_or_else(|e| panic!("{overrides:?}: {e}"));
        assert_eq!(composition.identity.overrides, overrides);
    }
}
