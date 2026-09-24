use super::*;

/// The guest paths spell the same components the host layout joins.
#[test]
fn the_guest_paths_join_the_bare_components() {
    assert_eq!(GUEST_GAME_DIR, format!("/{HDD0_MOUNT}/{GAME_DIR}"));
    assert_eq!(
        GUEST_EXDATA_DIR,
        format!("/{HDD0_MOUNT}/{HOME_DIR}/{USER_DIR}/{EXDATA_DIR}")
    );
}
