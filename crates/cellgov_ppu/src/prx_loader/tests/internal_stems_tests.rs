use super::FIRMWARE_INTERNAL_PRX_STEMS;

#[test]
fn internal_stems_carry_no_directory_or_extension() {
    // The load site joins each stem with a directory and a .sprx/.prx
    // suffix. A stem that already carries either resolves to a path
    // that does not exist.
    for s in FIRMWARE_INTERNAL_PRX_STEMS {
        assert!(!s.is_empty(), "empty stem");
        assert!(
            !s.contains('/') && !s.contains('\\'),
            "{s:?} carries a directory separator"
        );
        assert!(
            !s.ends_with(".sprx") && !s.ends_with(".prx"),
            "{s:?} carries a file extension"
        );
    }
}
