//! Install-subcommand argument parsing and output-directory preflight checks.

use super::*;
use crate::scratch_dir::scratch;

fn argv(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_install".to_string(), "install".to_string()];
    v.extend(parts.iter().map(|s| s.to_string()));
    v
}

#[test]
fn parse_default_output_is_the_vfs_root() {
    let a = parse_install_args(&argv(&["/tmp/PS3UPDAT.PUP"])).expect("parse");
    assert_eq!(a.pup_path, PathBuf::from("/tmp/PS3UPDAT.PUP"));
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    assert!(!a.force);
}

#[test]
fn parse_override_output() {
    let a = parse_install_args(&argv(&["x.pup", "--output", "/elsewhere"])).expect("parse");
    assert_eq!(a.output_dir, PathBuf::from("/elsewhere"));
    assert!(!a.force);
}

#[test]
fn parse_force_flag() {
    let a = parse_install_args(&argv(&["x.pup", "--force"])).expect("parse");
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    assert!(a.force);
}

#[test]
fn parse_force_and_output_in_either_order() {
    let a = parse_install_args(&argv(&["x.pup", "--force", "--output", "/d"]))
        .expect("parse force-first");
    assert_eq!(a.output_dir, PathBuf::from("/d"));
    assert!(a.force);

    let a = parse_install_args(&argv(&["x.pup", "--output", "/d", "--force"]))
        .expect("parse output-first");
    assert_eq!(a.output_dir, PathBuf::from("/d"));
    assert!(a.force);
}

#[test]
fn parse_missing_pup_errors() {
    let r = parse_install_args(&["cellgov_install".into(), "install".into()]);
    assert!(r.is_err());
}

#[test]
fn parse_output_without_value_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--output"]));
    assert!(r.is_err());
}

#[test]
fn parse_unknown_flag_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--garbage"]));
    assert!(r.is_err());
}

#[test]
fn check_output_dir_missing_is_ok() {
    let dir = scratch();
    assert!(check_output_dir(&dir.join("absent"), false).is_ok());
}

#[test]
fn check_output_dir_empty_is_ok() {
    let dir = scratch();
    assert!(check_output_dir(&dir, false).is_ok());
}

#[test]
fn check_output_dir_nonempty_without_force_errors() {
    let dir = scratch();
    std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
    assert!(matches!(
        check_output_dir(&dir, false),
        Err(FirmwareCliError::OutputDirNotEmpty { .. })
    ));
}

#[test]
fn check_output_dir_on_a_non_directory_reports_the_read_failure() {
    let dir = scratch();
    let file = dir.join("not_a_dir");
    std::fs::write(&file, b"x").unwrap();
    // The path exists, so the preflight gets past the `exists` arm and
    // has to name the `read_dir` refusal rather than treat it as empty.
    assert!(matches!(
        check_output_dir(&file, false),
        Err(FirmwareCliError::OutputDirReadFailed { .. })
    ));
}

#[test]
fn check_output_dir_nonempty_with_force_is_ok() {
    let dir = scratch();
    std::fs::write(dir.join("preexisting.txt"), b"x").unwrap();
    assert!(check_output_dir(&dir, true).is_ok());
}

#[test]
fn install_exclusion_prunes_emulators_and_dollar_entries() {
    assert!(is_install_excluded("dev_flash/ps1emu/ps1_emu.self"));
    assert!(is_install_excluded("dev_flash/ps2emu/ps2_emu.self"));
    assert!(is_install_excluded("dev_flash/pspemu/flash0/font/x.pgf"));
    assert!(is_install_excluded("ps2emu/ps2_netemu.self"));
    // Fullwidth-dollar (U+FF04) dead-entry marker is dropped.
    assert!(is_install_excluded("dev_flash/vsh/\u{ff04}dead.self"));
}

#[test]
fn install_exclusion_keeps_real_firmware_paths() {
    assert!(!is_install_excluded("dev_flash/sys/external/liblv2.sprx"));
    assert!(!is_install_excluded("dev_flash/vsh/module/mcore_tk.self"));
    // A plain ASCII '$' must not trip the fullwidth-dollar gate.
    assert!(!is_install_excluded("dev_flash/vsh/resource/a$b.txt"));
    // "pspemu" matches only as a leading path component, not a substring.
    assert!(!is_install_excluded("dev_flash/data/pspemu_notes.txt"));
    // A sibling mount is not dev_flash content, so the dev_flash-rooted
    // prune list must not reach into it.
    assert!(!is_install_excluded("dev_flash2/ps2emu/x.self"));
}

#[test]
fn install_exclusion_prunes_through_the_packaging_prefixes_the_extractor_strips() {
    // The extractor routes all four of these to dev_flash/ps2emu/...,
    // so the prune has to see them as the same entry.
    assert!(is_install_excluded("ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("/ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("000/ps2emu/ps2_netemu.self"));
    assert!(is_install_excluded("000/dev_flash/ps2emu/ps2_netemu.self"));
}

#[test]
fn firmware_mounts_covers_dev_flash_and_both_siblings() {
    let mounts: Vec<&str> = firmware_mounts().collect();
    assert_eq!(mounts, vec!["dev_flash", "dev_flash2", "dev_flash3"]);
}

#[test]
fn preflight_refuses_an_occupied_sibling_mount_and_names_it() {
    let dir = scratch();
    // dev_flash itself is empty; only the sibling mount is occupied.
    std::fs::create_dir_all(dir.join("dev_flash3")).unwrap();
    std::fs::write(dir.join("dev_flash3/leftover.bin"), b"x").unwrap();

    let err = preflight_firmware_mounts(&dir, false).expect_err("refuses");
    let FirmwareCliError::OutputDirNotEmpty { path } = &err else {
        panic!("expected OutputDirNotEmpty, got {err}");
    };
    assert!(
        path.ends_with("dev_flash3"),
        "the refusal must name the occupied mount, got {}",
        path.display()
    );
}

#[test]
fn preflight_ignores_mounts_a_firmware_install_does_not_write() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("dev_hdd0/game/NPUA80001")).unwrap();
    std::fs::write(dir.join("dev_hdd0/game/NPUA80001/x.bin"), b"g").unwrap();
    std::fs::create_dir_all(dir.join("dev_bdvd")).unwrap();
    std::fs::write(dir.join("dev_bdvd/PS3_DISC.SFB"), b"d").unwrap();
    assert!(preflight_firmware_mounts(&dir, false).is_ok());
}

#[test]
fn an_unreadable_firmware_tree_is_named_rather_than_yielding_a_short_manifest() {
    let dir = scratch();
    let absent = dir.join("dev_flash");
    assert!(matches!(
        build_firmware_manifest(b"pup", 0, &absent),
        Err(FirmwareCliError::FirmwareTreeReadFailed { .. })
    ));

    let not_a_dir = dir.join("dev_flash.txt");
    std::fs::write(&not_a_dir, b"x").unwrap();
    assert!(matches!(
        build_firmware_manifest(b"pup", 0, &not_a_dir),
        Err(FirmwareCliError::FirmwareTreeReadFailed { .. })
    ));
}

#[test]
fn the_manifest_walk_collects_prx_and_sprx_from_every_depth_in_sorted_order() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("sys/external")).unwrap();
    std::fs::write(dir.join("sys/external/b.sprx"), b"B").unwrap();
    std::fs::write(dir.join("sys/external/a.PRX"), b"A").unwrap();
    std::fs::write(dir.join("sys/external/notes.txt"), b"N").unwrap();
    std::fs::write(dir.join("top.prx"), b"T").unwrap();

    let mut paths = Vec::new();
    collect_sprx_paths(&dir, &mut paths).expect("walk");
    let rel: Vec<String> = paths
        .iter()
        .map(|p| {
            p.strip_prefix(&*dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        rel,
        vec!["sys/external/a.PRX", "sys/external/b.sprx", "top.prx"]
    );
}

#[test]
fn preflight_force_clears_every_firmware_mount() {
    for occupied in ["dev_flash", "dev_flash2", "dev_flash3"] {
        let dir = scratch();
        std::fs::create_dir_all(dir.join(occupied)).unwrap();
        std::fs::write(dir.join(occupied).join("leftover.bin"), b"x").unwrap();
        assert!(
            preflight_firmware_mounts(&dir, false).is_err(),
            "{occupied} must block without --force"
        );
        assert!(
            preflight_firmware_mounts(&dir, true).is_ok(),
            "--force must clear {occupied}"
        );
    }
}
