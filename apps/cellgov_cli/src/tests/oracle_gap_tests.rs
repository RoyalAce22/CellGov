//! Where the overlay lives.

use super::*;

#[test]
fn the_overlay_sits_in_the_install_roots_metadata_directory() {
    // The PS3 VFS root is the `dev_hdd0` mount; the overlay sits one
    // level up, beside the key vault.
    assert_eq!(
        overlay_path(Path::new("store/dev_hdd0")),
        Path::new("store/.cellgov/oracle-gap.tsv")
    );
}
