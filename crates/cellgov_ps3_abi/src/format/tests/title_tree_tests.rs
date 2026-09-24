use super::*;

/// The guest path spells the bare mount name.
#[test]
fn the_guest_disc_path_is_the_bare_mount() {
    assert_eq!(GUEST_BDVD, format!("/{BDVD_MOUNT}"));
}
