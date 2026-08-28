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
    assert!(matches!(r, Err(FirmwareCliError::MissingPupPath)));
}

#[test]
fn parse_output_without_value_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--output"]));
    assert!(matches!(
        r,
        Err(FirmwareCliError::OutputFlagMissingValue { .. })
    ));
}

#[test]
fn parse_unknown_flag_errors() {
    let r = parse_install_args(&argv(&["x.pup", "--garbage"]));
    assert!(matches!(r, Err(FirmwareCliError::UnknownArgument(ref a)) if a == "--garbage"));
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
    assert!(dir.join("dev_hdd0/game/NPUA80001/x.bin").is_file());
    assert!(dir.join("dev_bdvd/PS3_DISC.SFB").is_file());
}

#[cfg(feature = "decrypt")]
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

#[cfg(feature = "decrypt")]
/// Real PS3 firmware ships a zero-byte `.sprx` placeholder. Hashing it
/// as a pre-decrypted module would put the empty-bytes digest in the
/// manifest under revision 0, so the boot verifier would be handed an
/// empty file described as a loadable module.
#[test]
fn a_sprx_that_is_neither_an_sce_container_nor_an_elf_is_left_out_of_the_manifest() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("vsh/module")).unwrap();
    std::fs::write(dir.join("vsh/module/placeholder.sprx"), b"").unwrap();
    std::fs::write(dir.join("vsh/module/garbage.sprx"), b"not a module").unwrap();
    let mut bare_elf = ELF_MAGIC.to_vec();
    bare_elf.extend_from_slice(b"pre-decrypted body");
    std::fs::write(dir.join("vsh/module/plain.prx"), &bare_elf).unwrap();

    let manifest = build_firmware_manifest(b"pup", 0, &dir).expect("manifest");
    let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["vsh/module/plain.prx"],
        "only the bare ELF is a module"
    );

    // The one entry is the ELF's own bytes, not the empty-bytes hash a
    // recorded placeholder would carry.
    let empty_digest = manifest::Sha256(Sha256::digest(b"").into());
    assert_ne!(manifest.files[0].sha256, empty_digest);
    assert_eq!(
        manifest.files[0].sha256,
        manifest::Sha256(Sha256::digest(&bare_elf).into())
    );
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

fn decrypt_argv(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_install".to_string(), "decrypt-self".to_string()];
    v.extend(parts.iter().map(|s| s.to_string()));
    v
}

#[test]
fn decrypt_self_defaults_to_the_vfs_root_and_no_explicit_rap() {
    let a = parse_decrypt_self_args(&decrypt_argv(&["EBOOT.BIN"])).expect("parse");
    assert_eq!(a.self_path, PathBuf::from("EBOOT.BIN"));
    assert_eq!(a.vfs_root, PathBuf::from(DEFAULT_INSTALL_OUTPUT));
    assert!(a.rap_path.is_none());
    assert!(a.output_path.is_none());
}

#[test]
fn decrypt_self_accepts_rap_and_vfs_root_in_any_order() {
    for parts in [
        vec![
            "e.bin",
            "--rap",
            "k.rap",
            "--vfs-root",
            "/v",
            "--output",
            "o.elf",
        ],
        vec![
            "e.bin",
            "--output",
            "o.elf",
            "--vfs-root",
            "/v",
            "--rap",
            "k.rap",
        ],
    ] {
        let a = parse_decrypt_self_args(&decrypt_argv(&parts)).expect("parse");
        assert_eq!(a.rap_path, Some(PathBuf::from("k.rap")));
        assert_eq!(a.vfs_root, PathBuf::from("/v"));
        assert_eq!(a.output_path, Some(PathBuf::from("o.elf")));
    }
}

#[test]
fn decrypt_self_flags_without_a_value_are_refused_by_name() {
    for flag in ["--rap", "--vfs-root", "--output"] {
        let err = parse_decrypt_self_args(&decrypt_argv(&["e.bin", flag]))
            .err()
            .unwrap_or_else(|| panic!("{flag} with no value must not parse"));
        let named = match flag {
            "--rap" => matches!(err, FirmwareCliError::RapFlagMissingValue),
            "--vfs-root" => matches!(err, FirmwareCliError::VfsRootFlagMissingValue),
            _ => matches!(err, FirmwareCliError::OutputFlagMissingValue { .. }),
        };
        assert!(named, "{flag} must name its own missing value, got {err:?}");
    }
}

/// The lookup key is the content id from the NPD header, so the
/// directory has to match what `install-game` wrote and what the boot
/// path reads.
#[test]
fn exdata_dir_is_the_layout_install_game_commits_into() {
    let d = game_install::exdata_dir(Path::new("vfs/dev_hdd0"));
    let rendered = d.to_string_lossy().replace('\\', "/");
    assert_eq!(rendered, "vfs/dev_hdd0/home/00000001/exdata");
}

#[test]
fn a_rap_that_is_not_sixteen_bytes_is_refused_by_name() {
    let dir = scratch();
    let rap = dir.join("short.rap");
    std::fs::write(&rap, b"nope").unwrap();
    let err = rap_from_file(&rap).expect_err("wrong size");
    assert!(
        matches!(err, FirmwareCliError::RapWrongSize { len: 4, .. }),
        "expected RapWrongSize, got {err:?}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("short.rap"), "names the file: {rendered}");
    // A bare `contains('4')` would also match a digit in the scratch
    // path, so pin the phrasing around the size.
    assert!(
        rendered.contains("is 4 bytes"),
        "names the size: {rendered}"
    );
    assert!(
        rendered.contains("expected exactly 16"),
        "names the requirement: {rendered}"
    );
}

/// An absent RAP is the ordinary uninstalled case: the NPDRM layer
/// turns `None` into the license-3 fallback or a named refusal, so
/// reading a missing file must not be an error here.
#[test]
fn an_absent_rap_resolves_to_no_key_rather_than_an_error() {
    let dir = scratch();
    assert_eq!(rap_from_file(&dir.join("absent.rap")).unwrap(), None);
}

#[test]
fn a_sixteen_byte_rap_is_read_verbatim_from_disk() {
    let dir = scratch();
    let zeroes = dir.join("zeroes.rap");
    let ones = dir.join("ones.rap");
    std::fs::write(&zeroes, [0u8; 16]).unwrap();
    std::fs::write(&ones, [0x11u8; 16]).unwrap();

    let from_zeroes = rap_from_file(&zeroes).unwrap().expect("read");
    let from_ones = rap_from_file(&ones).unwrap().expect("read");
    assert_eq!(from_zeroes, Rap([0u8; 16]));
    assert_eq!(from_ones, Rap([0x11u8; 16]));
}

#[test]
fn preflight_guards_every_firmware_mount_and_force_waives_the_guard() {
    for occupied in ["dev_flash", "dev_flash2", "dev_flash3"] {
        let dir = scratch();
        let leftover = dir.join(occupied).join("leftover.bin");
        std::fs::create_dir_all(dir.join(occupied)).unwrap();
        std::fs::write(&leftover, b"x").unwrap();

        let err = preflight_firmware_mounts(&dir, false)
            .expect_err("an occupied mount must block without --force");
        let FirmwareCliError::OutputDirNotEmpty { path } = &err else {
            panic!("expected OutputDirNotEmpty for {occupied}, got {err}");
        };
        assert!(path.ends_with(occupied), "the refusal names {occupied}");

        assert!(
            preflight_firmware_mounts(&dir, true).is_ok(),
            "--force must waive the guard on {occupied}"
        );
        // The preflight only decides whether the install may proceed.
        // It removes nothing, so --force waives the guard rather than
        // clearing the mount.
        assert!(leftover.is_file(), "{occupied} left untouched by preflight");
    }
}

/// Only absence may resolve to "no key". Any other read failure looks
/// identical from the resolver's `Option` and would let a license-3
/// SELF decrypt on the free key as if no RAP had been asked for.
#[test]
fn a_rap_that_is_present_but_unreadable_is_named_rather_than_read_as_absent() {
    let dir = scratch();
    let not_a_file = dir.join("a_directory.rap");
    std::fs::create_dir_all(&not_a_file).unwrap();

    let err = rap_from_file(&not_a_file).expect_err("an unreadable RAP is not absence");
    assert!(
        matches!(err, FirmwareCliError::RapReadFailed { .. }),
        "expected RapReadFailed, got {err:?}"
    );
    assert!(
        err.to_string().contains("a_directory.rap"),
        "the refusal names the file: {err}"
    );
}

#[test]
fn an_explicit_rap_that_does_not_exist_is_refused_rather_than_resolved_to_no_key() {
    let dir = scratch();
    let named = dir.join("absent.rap");

    let err = resolve_rap(Some(&named), &dir, "UP9000-NPUA80001_00-XXXX")
        .expect_err("a named --rap that is not there is a refusal");
    let FirmwareCliError::ExplicitRapMissing { path } = &err else {
        panic!("expected ExplicitRapMissing, got {err:?}");
    };
    assert_eq!(path, &named);
}

/// The exdata probe is the one lookup allowed to miss quietly: a title
/// that is simply not installed is the ordinary case, and license-3
/// falls back to the free key from there.
#[test]
fn an_exdata_probe_that_misses_is_the_uninstalled_case_not_a_refusal() {
    let dir = scratch();
    assert_eq!(
        resolve_rap(None, &dir, "UP9000-NPUA80001_00-XXXX").unwrap(),
        None
    );
}

#[test]
fn an_explicit_rap_is_used_in_place_of_the_content_id_keyed_exdata_file() {
    let dir = scratch();
    let exdata = dir.join("exdata");
    std::fs::create_dir_all(&exdata).unwrap();
    std::fs::write(exdata.join("CID.rap"), [0u8; 16]).unwrap();
    let explicit = dir.join("other.rap");
    std::fs::write(&explicit, [0x11u8; 16]).unwrap();

    let probed = resolve_rap(None, &exdata, "CID").unwrap();
    let named = resolve_rap(Some(&explicit), &exdata, "CID").unwrap();
    assert_eq!(probed, Some(Rap([0u8; 16])));
    assert_eq!(named, Some(Rap([0x11u8; 16])));
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn a_decrypting_subcommand_on_a_build_without_decrypt_is_refused_naming_both() {
    for sub in ["install", "install-game", "install-iso", "decrypt-self"] {
        let rendered = FirmwareCliError::DecryptFeatureDisabled {
            subcommand: sub.to_string(),
        }
        .to_string();
        assert!(
            rendered.starts_with(sub),
            "names the subcommand: {rendered}"
        );
        assert!(
            rendered.contains("--features decrypt"),
            "names the rebuild: {rendered}"
        );
    }
}

#[test]
fn the_transient_plaintext_path_is_keyed_by_process_id() {
    // Two --dkey installs into one VFS root must not share a temp file:
    // File::create truncates, and the other run may still be mapped on
    // it. The pid is what keeps them apart.
    let dir = Path::new("vfs/.cellgov");
    let a = temp_decrypt_path(dir, 4242);
    let b = temp_decrypt_path(dir, 4243);
    assert_ne!(a, b);
    assert!(a.starts_with(dir));
    assert_eq!(
        a.file_name().and_then(|n| n.to_str()),
        Some("disc-decrypt-4242.tmp")
    );
}
