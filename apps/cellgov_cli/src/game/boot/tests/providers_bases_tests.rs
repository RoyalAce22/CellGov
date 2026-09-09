//! The base list the providers share: the composition's EBOOT
//! directories when the loaded executable is one of them, else the
//! executable's own directory alone.

use std::path::{Path, PathBuf};

use super::usrdir_bases;

fn dirs(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

#[test]
fn the_composition_order_stands_when_the_loaded_eboot_sits_in_it() {
    let composition = dirs(&["store/updates/02.10/game/USRDIR", "store/game/USRDIR"]);
    assert_eq!(
        usrdir_bases("store/game/USRDIR/EBOOT.BIN", &composition),
        composition,
        "the base's executable was loaded, and the update still shadows it"
    );
    assert_eq!(
        usrdir_bases("store/updates/02.10/game/USRDIR/EBOOT.BIN", &composition),
        composition
    );
}

#[test]
fn an_executable_outside_the_composition_keeps_its_own_directory_alone() {
    let composition = dirs(&["store/updates/02.10/game/USRDIR", "store/game/USRDIR"]);
    assert_eq!(
        usrdir_bases("build/out/EBOOT.BIN", &composition),
        dirs(&["build/out"])
    );
}

#[test]
fn no_composition_yields_the_eboot_directory_alone() {
    assert_eq!(
        usrdir_bases("build/out/EBOOT.BIN", &[]),
        dirs(&["build/out"])
    );
    assert_eq!(usrdir_bases("EBOOT.BIN", &[]), dirs(&["."]));
}

#[test]
fn no_composition_and_no_eboot_directory_yields_nothing() {
    assert!(usrdir_bases("", &[]).is_empty());
}

#[test]
fn another_spelling_of_a_composed_directory_keeps_the_composition_order() {
    // The explicit executable names the base's directory through `..`.
    // Component-wise the two spellings differ; on disk they are one
    // directory, so the update still leads and the list holds each
    // directory once.
    let update = cellgov_testkit::scratch::scratch_labeled("bases_update");
    let base = cellgov_testkit::scratch::scratch_labeled("bases_base");
    std::fs::create_dir_all(base.join("sub")).unwrap();
    let composition = vec![update.to_path_buf(), base.to_path_buf()];
    let elf_path = base.join("sub").join("..").join("EBOOT.BIN");
    let elf_path = elf_path.to_str().expect("a UTF-8 scratch path");
    assert_ne!(
        Path::new(elf_path).parent(),
        Some(&*base),
        "the spelling differs component-wise, or this test proves nothing"
    );
    assert_eq!(usrdir_bases(elf_path, &composition), composition);
}

#[test]
fn a_directory_that_does_not_resolve_is_compared_component_wise() {
    let base = cellgov_testkit::scratch::scratch_labeled("bases_unresolved");
    let absent = base.join("no-such-usrdir");
    let composition = vec![absent.clone()];
    let elf_path = absent.join("EBOOT.BIN");
    let elf_path = elf_path.to_str().expect("a UTF-8 scratch path");
    assert_eq!(usrdir_bases(elf_path, &composition), composition);
}

#[cfg(windows)]
#[test]
fn a_forward_slash_spelling_matches_a_backslash_composition_entry() {
    // `forwardable_eboot_path` spells the loaded path with `/`; the
    // composition built it with the host separator. Both name one
    // directory.
    let composition = vec![PathBuf::from(r"D:\store\game\USRDIR")];
    assert_eq!(
        usrdir_bases("D:/store/game/USRDIR/EBOOT.BIN", &composition),
        composition
    );
}
