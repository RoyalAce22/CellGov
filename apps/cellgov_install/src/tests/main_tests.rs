//! Subcommand argument parsing and the CLI's RAP resolution.

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
fn parse_verbose_accepts_both_spellings() {
    for flag in ["-v", "--verbose"] {
        let a = parse_install_args(&argv(&["x.pup", flag])).expect("parse");
        assert!(a.verbose, "{flag} sets verbose");
    }
    assert!(
        !parse_install_args(&argv(&["x.pup"]))
            .expect("parse")
            .verbose
    );
}

#[test]
fn parse_absorbs_the_render_flags() {
    let a = parse_install_args(&argv(&["x.pup", "--no-progress", "--no-color", "--quiet"]))
        .expect("parse");
    assert!(a.render.no_progress);
    assert!(a.render.no_color);
    assert!(a.render.quiet);
}

/// A PUP carries 20-odd packages and almost none of them prune or skip
/// anything, so a line that spells the zeros out on every one buries
/// the packages that did something.
#[cfg(feature = "decrypt")]
#[test]
fn a_package_summary_names_only_the_counts_it_has() {
    let mut p = PackageSummary {
        package: "dev_flash_013.tar".to_string(),
        written: 104,
        pruned: 0,
        skipped: 0,
    };
    assert_eq!(package_summary_line(&p), "dev_flash_013.tar: 104 files");

    p.pruned = 75;
    assert_eq!(
        package_summary_line(&p),
        "dev_flash_013.tar: 104 files, 75 pruned"
    );

    p.skipped = 2;
    assert_eq!(
        package_summary_line(&p),
        "dev_flash_013.tar: 104 files, 75 pruned, 2 entries addressing no file"
    );
}

/// A package that legitimately carries no dev_flash content is a real
/// case: `dev_flash_000` of retail 4.93 extracts zero files.
#[cfg(feature = "decrypt")]
#[test]
fn a_package_that_extracted_nothing_still_reports_its_zero() {
    let p = PackageSummary {
        package: "dev_flash_000.tar".to_string(),
        written: 0,
        pruned: 0,
        skipped: 0,
    };
    assert_eq!(package_summary_line(&p), "dev_flash_000.tar: 0 files");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_single_omission_does_not_read_as_a_plural() {
    assert_eq!(plural(1, "file", "files"), "1 file");
    assert_eq!(plural(0, "file", "files"), "0 files");
    assert_eq!(plural(2, "file", "files"), "2 files");
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
fn install_iso_refuses_the_retired_dkey_flag_by_name() {
    let args: Vec<String> = [
        "cellgov_install",
        "install-iso",
        "x.iso",
        "--dkey",
        "x.dkey",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let r = parse_install_iso_args(&args);
    assert!(matches!(r, Err(FirmwareCliError::UnknownArgument(ref a)) if a == "--dkey"));
}

#[test]
fn install_iso_parses_output_and_force() {
    let args: Vec<String> = [
        "cellgov_install",
        "install-iso",
        "x.iso",
        "--output",
        "/d",
        "--force",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let a = parse_install_iso_args(&args).expect("parse");
    assert_eq!(a.path, PathBuf::from("x.iso"));
    assert_eq!(a.output_dir, PathBuf::from("/d"));
    assert!(a.force);
}

// `install-iso` and `install-update` share one parser, which takes the
// missing-path refusal as a parameter; only a per-subcommand assertion
// catches the two call sites being handed each other's error.
#[test]
fn install_iso_with_no_path_is_refused_by_its_own_name() {
    let r = parse_install_iso_args(&["cellgov_install".into(), "install-iso".into()]);
    assert!(matches!(r, Err(FirmwareCliError::MissingIsoPath)));
}

#[test]
fn install_iso_defaults_its_output_to_the_vfs_root() {
    let args: Vec<String> = ["cellgov_install", "install-iso", "x.iso"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let a = parse_install_iso_args(&args).expect("parse");
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT));
    assert!(!a.force);
}

/// A partial install with one failed package and one failed entry.
#[cfg(feature = "decrypt")]
fn partial_install() -> FirmwareInstallError {
    use cellgov_install::firmware_install::PackageFailure;
    use cellgov_install::tar::{ExtractError, TarParseError};

    FirmwareInstallError::PartialInstall {
        files: 3,
        packages: 2,
        packages_failed: vec![PackageFailure::InnerTar {
            package: "dev_flash_010.tar".to_string(),
            source: TarParseError::NotUstarHeader { offset: 0x200 },
        }],
        extract_errors: vec![ExtractError::PathTraversal {
            guest_path: "../escape.self".to_string(),
            host_path: PathBuf::from("escape.self"),
        }],
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn a_partial_install_names_every_package_and_entry_it_lost() {
    let lines = install_failure_detail(&partial_install());
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("dev_flash_010.tar"), "{lines:?}");
    assert!(lines[1].contains("escape.self"), "{lines:?}");
}

/// A cleanup that cannot discard the staging root renders only the
/// wrapped fault's summary counts -- and that is the case where the
/// operator has residue on disk and most needs the names.
#[cfg(feature = "decrypt")]
#[test]
fn staging_residue_does_not_swallow_the_partial_install_it_wraps() {
    let expected = install_failure_detail(&partial_install());
    let wrapped = FirmwareInstallError::StagingResidue {
        path: PathBuf::from("vfs/firmware/.firmware-staging"),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        cause: Box::new(partial_install()),
    };
    assert_eq!(install_failure_detail(&wrapped), expected);
}

#[cfg(feature = "decrypt")]
#[test]
fn a_failure_that_renders_its_own_cause_adds_no_detail_lines() {
    let e = FirmwareInstallError::ProducedNothing { packages: 4 };
    assert!(install_failure_detail(&e).is_empty());
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
fn the_rap_probe_reads_the_directory_install_game_commits_into() {
    let d = cellgov_install::store::StoreLayout::new("vfs").live_exdata_dir();
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

fn keys_argv(parts: &[&str]) -> Vec<String> {
    let mut v = vec!["cellgov_install".to_string(), "keys".to_string()];
    v.extend(parts.iter().map(|s| s.to_string()));
    v
}

fn hex_of(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}

#[test]
fn keys_without_a_subcommand_is_refused_by_name() {
    let r = parse_keys_args(&keys_argv(&[]));
    assert!(matches!(r, Err(FirmwareCliError::KeysMissingSubcommand)));
}

#[test]
fn keys_with_an_unknown_subcommand_is_refused_naming_it() {
    let r = parse_keys_args(&keys_argv(&["frobnicate"]));
    assert!(matches!(r, Err(FirmwareCliError::KeysUnknownSubcommand(ref s)) if s == "frobnicate"));
}

#[test]
fn keys_show_defaults_to_the_vfs_root_and_takes_an_optional_path() {
    assert_eq!(
        parse_keys_args(&keys_argv(&["show"])).expect("parse"),
        KeysCommand::Show {
            path: None,
            vfs_root: PathBuf::from(DEFAULT_INSTALL_OUTPUT),
        }
    );
    assert_eq!(
        parse_keys_args(&keys_argv(&["show", "/k/keys.txt", "--output", "/v"])).expect("parse"),
        KeysCommand::Show {
            path: Some(PathBuf::from("/k/keys.txt")),
            vfs_root: PathBuf::from("/v"),
        }
    );
}

#[test]
fn keys_import_requires_a_path_and_accepts_output_and_replace_in_any_order() {
    let r = parse_keys_args(&keys_argv(&["import"]));
    assert!(matches!(r, Err(FirmwareCliError::KeysImportMissingPath)));
    for parts in [
        vec!["import", "/k", "--replace", "--output", "/v"],
        vec!["import", "--output", "/v", "/k", "--replace"],
    ] {
        assert_eq!(
            parse_keys_args(&keys_argv(&parts)).expect("parse"),
            KeysCommand::Import {
                path: PathBuf::from("/k"),
                vfs_root: PathBuf::from("/v"),
                replace: true,
            }
        );
    }
    assert_eq!(
        parse_keys_args(&keys_argv(&["import", "/k"])).expect("parse"),
        KeysCommand::Import {
            path: PathBuf::from("/k"),
            vfs_root: PathBuf::from(DEFAULT_INSTALL_OUTPUT),
            replace: false,
        }
    );
}

#[test]
fn keys_remove_takes_only_the_output_flag() {
    assert_eq!(
        parse_keys_args(&keys_argv(&["remove", "--output", "/v"])).expect("parse"),
        KeysCommand::Remove {
            vfs_root: PathBuf::from("/v"),
        }
    );
    let r = parse_keys_args(&keys_argv(&["remove", "/k"]));
    assert!(matches!(
        r,
        Err(FirmwareCliError::KeysExtraPositional {
            subcommand: "remove",
            ..
        })
    ));
}

#[test]
fn keys_flags_without_a_value_and_unknown_flags_are_refused_by_name() {
    let r = parse_keys_args(&keys_argv(&["show", "--output"]));
    assert!(matches!(
        r,
        Err(FirmwareCliError::OutputFlagMissingValue { .. })
    ));
    let r = parse_keys_args(&keys_argv(&["show", "--replace"]));
    assert!(matches!(r, Err(FirmwareCliError::UnknownArgument(ref a)) if a == "--replace"));
    let r = parse_keys_args(&keys_argv(&["import", "/k", "/k2"]));
    assert!(matches!(
        r,
        Err(FirmwareCliError::KeysExtraPositional {
            subcommand: "import",
            ..
        })
    ));
}

#[test]
fn keys_import_writes_a_toml_that_reloads_with_the_same_slots_and_remove_deletes_it() {
    let dir = scratch();
    let vfs = dir.join("vfs");
    let source = dir.join("keys.txt");
    std::fs::write(
        &source,
        format!(
            "pkg_aes = {}\npup_hmac = {}\n",
            hex_of(0x11, 16),
            hex_of(0x22, 64)
        ),
    )
    .unwrap();

    let imported = import_keys(&source, &vfs, false).expect("import");
    assert_eq!(imported.pkg_aes().unwrap(), &[0x11u8; 16]);

    let file = installed_keys_dir(&vfs).join(INSTALLED_KEYS_FILE);
    assert!(file.is_file(), "{} written", file.display());
    let reloaded = KeyVault::load_from_path(&file).expect("reload");
    assert_eq!(reloaded.pkg_aes().unwrap(), &[0x11u8; 16]);
    assert_eq!(reloaded.pup_hmac().unwrap(), &[0x22u8; 64]);
    assert!(reloaded.np_klic_key().is_err(), "only the imported slots");
    assert_eq!(
        KeyVault::locate_from(None, &vfs).expect("located"),
        file,
        "the decrypting subcommands find what import wrote"
    );

    assert!(remove_keys(&vfs).expect("remove"), "something was removed");
    assert!(!installed_keys_dir(&vfs).exists());
    assert!(!remove_keys(&vfs).expect("remove again"), "nothing left");
}

#[test]
fn keys_import_merges_into_the_installed_vault_unless_replace() {
    let dir = scratch();
    let vfs = dir.join("vfs");
    let first = dir.join("first.txt");
    let second = dir.join("second.txt");
    std::fs::write(&first, format!("pkg_aes = {}\n", hex_of(0x11, 16))).unwrap();
    std::fs::write(&second, format!("np_klic_key = {}\n", hex_of(0x33, 16))).unwrap();

    import_keys(&first, &vfs, false).expect("first import");
    let merged = import_keys(&second, &vfs, false).expect("second import merges");
    assert_eq!(merged.pkg_aes().unwrap(), &[0x11u8; 16]);
    assert_eq!(merged.np_klic_key().unwrap(), &[0x33u8; 16]);

    let replaced = import_keys(&second, &vfs, true).expect("replace");
    assert!(
        replaced.pkg_aes().is_err(),
        "--replace drops the earlier slot"
    );
    assert_eq!(replaced.np_klic_key().unwrap(), &[0x33u8; 16]);
    let file = installed_keys_dir(&vfs).join(INSTALLED_KEYS_FILE);
    let on_disk = KeyVault::load_from_path(&file).expect("reload");
    assert!(on_disk.pkg_aes().is_err());
}

#[test]
fn keys_import_refuses_a_disagreeing_slot_naming_both_definitions() {
    let dir = scratch();
    let vfs = dir.join("vfs");
    let first = dir.join("first.txt");
    let other = dir.join("other.txt");
    std::fs::write(&first, format!("pkg_aes = {}\n", hex_of(0x11, 16))).unwrap();
    std::fs::write(&other, format!("pkg_aes = {}\n", hex_of(0x12, 16))).unwrap();

    import_keys(&first, &vfs, false).expect("first import");
    let err = import_keys(&other, &vfs, false).expect_err("a conflict is refused");
    let FirmwareCliError::Keys(KeyVaultError::Conflict {
        what,
        first,
        second,
    }) = &err
    else {
        panic!("expected a Conflict, got {err:?}");
    };
    assert_eq!(what, "pkg_aes");
    assert!(first.path.ends_with(INSTALLED_KEYS_FILE), "{first}");
    assert!(second.path.ends_with("other.txt"), "{second}");
    let on_disk = KeyVault::load_from_path(&installed_keys_dir(&vfs).join(INSTALLED_KEYS_FILE))
        .expect("reload");
    assert_eq!(
        on_disk.pkg_aes().unwrap(),
        &[0x11u8; 16],
        "the installed vault is untouched by a refused import"
    );
}

#[test]
fn keys_import_of_a_file_holding_no_key_is_refused_and_writes_nothing() {
    let dir = scratch();
    let vfs = dir.join("vfs");
    let source = dir.join("notes.txt");
    std::fs::write(&source, format!("frobnicate = {}\n", hex_of(0x11, 16))).unwrap();
    let err = import_keys(&source, &vfs, false).expect_err("nothing usable");
    assert!(
        matches!(err, FirmwareCliError::KeysNothingUsable { .. }),
        "got {err:?}"
    );
    assert!(!installed_keys_dir(&vfs).exists());
}

#[test]
fn keys_import_of_a_directory_of_disc_keys_is_refused_and_names_them_set_aside() {
    let dir = scratch();
    let source = dir.join("dkeys");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("Some Game (USA).dkey"), hex_of(0xD1, 16)).unwrap();
    std::fs::write(source.join("Other Game (Japan).key"), [0xD2u8; 16]).unwrap();
    let vfs = dir.join("vfs");

    let err = import_keys(&source, &vfs, false).expect_err("no slot takes a disc key");
    assert!(
        matches!(err, FirmwareCliError::KeysNothingUsable { .. }),
        "got {err:?}"
    );
    assert!(!err.to_string().contains("disc key"), "{err}");
    assert!(!installed_keys_dir(&vfs).exists());

    let reasons: Vec<String> = KeyVault::load_from_path(&source)
        .expect("load")
        .ignored()
        .iter()
        .map(|i| format!("{}: {}", i.at, i.reason))
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains(".dkey")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains(".key")), "{reasons:?}");
}

#[test]
fn keys_import_of_an_absent_path_is_refused_by_name() {
    let dir = scratch();
    let err = import_keys(&dir.join("absent"), &dir.join("vfs"), false).expect_err("absent");
    assert!(
        matches!(err, FirmwareCliError::Keys(KeyVaultError::Missing { .. })),
        "got {err:?}"
    );
}

#[test]
fn the_key_inventory_names_every_slot_and_what_the_decrypt_paths_still_lack() {
    let vault = KeyVault::parse(
        Path::new("k.txt"),
        format!(
            "pkg_aes = {}\nfrobnicate = {}\n",
            hex_of(0x11, 16),
            hex_of(0x12, 16)
        )
        .as_bytes(),
    )
    .expect("parse");
    let report = render_key_inventory(Path::new("k.txt"), &vault);
    assert!(report.starts_with("key vault: k.txt\n"), "{report}");
    assert!(
        report.contains("pkg_aes: 16 bytes, from k.txt:1"),
        "{report}"
    );
    for slot in Slot::ALL.iter().filter(|s| **s != Slot::PkgAes) {
        assert!(
            report.contains(&format!("{}: missing", slot.name())),
            "{slot}: {report}"
        );
    }
    assert!(report.contains("scepkg: 0 keyset(s)"), "{report}");
    assert!(
        report.contains("app: revisions (none), 0 unlabeled"),
        "{report}"
    );
    assert!(!report.contains("disc"), "{report}");
    assert!(report.contains("k.txt:2: name \"frobnicate\""), "{report}");
    assert!(
        report.contains("missing for decrypt: pup_hmac, "),
        "{report}"
    );
    assert!(!report.contains("decrypt paths: ready"), "{report}");
    assert!(
        !report.contains(&hex_of(0x11, 16)),
        "no key bytes: {report}"
    );
}
