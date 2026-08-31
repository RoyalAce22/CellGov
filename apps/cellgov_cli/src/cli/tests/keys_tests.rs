//! Where the CLI reads the imported vault, given the PS3 VFS root a
//! subcommand was handed.

use super::*;

#[test]
fn the_default_vfs_root_lands_on_the_install_root_cellgov_install_writes() {
    assert_eq!(
        install_root_of(Path::new("vfs/dev_hdd0")),
        PathBuf::from(cellgov_install::store::DEFAULT_VFS_ROOT),
    );
}

#[test]
fn a_relocated_vfs_root_reads_the_vault_beside_it_not_under_the_working_directory() {
    assert_eq!(
        cellgov_install::keys::installed_keys_dir(&install_root_of(Path::new(
            "/dumps/ps3/vfs/dev_hdd0"
        ))),
        PathBuf::from("/dumps/ps3/vfs/.cellgov/keys"),
    );
}

#[test]
fn a_bare_relative_root_stays_relative_instead_of_naming_the_filesystem_root() {
    assert_eq!(install_root_of(Path::new("dev_hdd0")), PathBuf::from("."));
}

#[test]
fn a_filesystem_root_stands_in_for_its_own_parent() {
    let root = Path::new("/");
    assert_eq!(install_root_of(root), root.to_path_buf());
}

#[test]
fn a_trailing_separator_does_not_move_the_root_up_a_level() {
    assert_eq!(
        install_root_of(Path::new("vfs/dev_hdd0/")),
        install_root_of(Path::new("vfs/dev_hdd0")),
    );
}

#[test]
fn a_working_directory_root_climbs_out_of_it_rather_than_staying_in_it() {
    let root = Path::new(".");
    assert_eq!(install_root_of(root), root.join(".."));
}

#[test]
fn a_dot_dot_root_climbs_above_itself_rather_than_back_down_to_the_working_directory() {
    let root = Path::new("..");
    assert_eq!(install_root_of(root), root.join(".."));
}

#[test]
fn a_root_ending_in_dot_dot_is_not_read_as_the_name_that_precedes_it() {
    let root = Path::new("vfs/..");
    assert_eq!(install_root_of(root), root.join(".."));
    assert_ne!(install_root_of(root), PathBuf::from("vfs"));
}

/// `C:` names the working directory on that drive, not the drive root.
#[cfg(windows)]
#[test]
fn a_drive_relative_root_is_not_mistaken_for_a_filesystem_root() {
    let root = Path::new("C:");
    assert_eq!(install_root_of(root), root.join(".."));
    assert_ne!(install_root_of(root), root.to_path_buf());
}

#[cfg(windows)]
#[test]
fn a_drive_root_stands_in_for_its_own_parent() {
    let root = Path::new("C:\\");
    assert_eq!(install_root_of(root), root.to_path_buf());
}

#[test]
fn two_mounts_under_one_install_root_agree_on_the_vault() {
    let cell = OnceLock::new();
    fix_vault_root_in(&cell, Path::new("vfs/dev_hdd0"));
    fix_vault_root_in(&cell, Path::new("vfs/dev_bdvd"));
    assert_eq!(cell.get().map(PathBuf::as_path), Some(Path::new("vfs")));
}

#[test]
#[should_panic(expected = "key vault root already fixed")]
fn a_second_root_under_a_different_install_root_is_refused_not_ignored() {
    let cell = OnceLock::new();
    fix_vault_root_in(&cell, Path::new("vfs/dev_hdd0"));
    fix_vault_root_in(&cell, Path::new("/dumps/ps3/vfs/dev_hdd0"));
}
