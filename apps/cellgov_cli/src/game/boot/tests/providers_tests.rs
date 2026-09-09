//! EBOOT-directory derivation for the content and mount providers.

use std::path::Path;

use super::eboot_dir;

#[test]
fn an_eboot_under_a_directory_yields_that_directory() {
    assert_eq!(
        eboot_dir("store/game/USRDIR/EBOOT.BIN"),
        Some(Path::new("store/game/USRDIR")),
    );
}

#[test]
fn a_forward_slash_drive_path_yields_its_directory_on_every_host() {
    // `forwardable_eboot_path` spells a resolved path this way on
    // Windows; a POSIX host parses the same string identically.
    assert_eq!(
        eboot_dir("D:/store/USRDIR/EBOOT.BIN"),
        Some(Path::new("D:/store/USRDIR")),
    );
}

#[cfg(windows)]
#[test]
fn a_backslash_path_yields_its_directory() {
    assert_eq!(
        eboot_dir(r"D:\store\USRDIR\EBOOT.BIN"),
        Some(Path::new(r"D:\store\USRDIR")),
    );
}

#[test]
fn a_bare_eboot_filename_resolves_to_the_cwd() {
    // The loader read `EBOOT.BIN` from the cwd, so that is the
    // directory it sits in; `Path::parent` spells it as the empty
    // path, which the providers cannot probe or canonicalize.
    let dir = eboot_dir("EBOOT.BIN").expect("a bare filename has a directory: the cwd");
    assert_eq!(dir, Path::new("."));
    assert!(
        !dir.as_os_str().is_empty(),
        "the empty path must never reach a provider as a base",
    );
}

#[test]
fn an_empty_eboot_path_has_no_directory() {
    assert_eq!(eboot_dir(""), None);
}

#[test]
fn a_root_only_eboot_path_has_no_directory() {
    assert_eq!(eboot_dir("/"), None);
}
