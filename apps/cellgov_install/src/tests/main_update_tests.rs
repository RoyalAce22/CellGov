//! `install-update` argument parsing.

use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    std::iter::once("cellgov_install")
        .chain(std::iter::once("install-update"))
        .chain(parts.iter().copied())
        .map(str::to_string)
        .collect()
}

#[test]
fn a_missing_pkg_path_is_refused_by_subcommand_name() {
    let r = parse_install_update_args(&argv(&[]));
    assert!(matches!(r, Err(FirmwareCliError::MissingUpdatePkgPath)));
}

#[test]
fn the_output_root_defaults_to_the_vfs_root() {
    let a = parse_install_update_args(&argv(&["patch.pkg"])).expect("parse");
    assert_eq!(a.path, PathBuf::from("patch.pkg"));
    assert_eq!(a.output_dir, PathBuf::from(DEFAULT_GAME_INSTALL_OUTPUT));
    assert!(!a.force);
}

#[test]
fn output_and_force_parse_in_either_order() {
    for parts in [
        vec!["patch.pkg", "--output", "/d", "--force"],
        vec!["patch.pkg", "--force", "--output", "/d"],
    ] {
        let a = parse_install_update_args(&argv(&parts)).expect("parse");
        assert_eq!(a.output_dir, PathBuf::from("/d"));
        assert!(a.force);
    }
}

#[test]
fn the_progress_render_flags_are_accepted() {
    parse_install_update_args(&argv(&["patch.pkg", "--no-progress", "--quiet"]))
        .expect("render flags are shared with the other install subcommands");
}

#[test]
fn an_unknown_flag_is_named() {
    let r = parse_install_update_args(&argv(&["patch.pkg", "--rap", "x.rap"]));
    assert!(matches!(r, Err(FirmwareCliError::UnknownArgument(ref a)) if a == "--rap"));
}

#[test]
fn output_without_a_value_is_refused() {
    let r = parse_install_update_args(&argv(&["patch.pkg", "--output"]));
    assert!(matches!(
        r,
        Err(FirmwareCliError::OutputFlagMissingValue { kind: "directory" })
    ));
}
