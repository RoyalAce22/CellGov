//! The uniqueness and cleanup guarantees the store filesystem tests
//! rely on.

use super::scratch;

#[test]
fn two_scratch_dirs_in_one_process_never_share_a_path() {
    let a = scratch();
    let b = scratch();
    assert_ne!(*a, *b);
    assert!(a.is_dir() && b.is_dir());
}

#[test]
fn a_fresh_scratch_dir_is_empty() {
    let dir = scratch();
    assert_eq!(
        std::fs::read_dir(&*dir)
            .expect("read the fresh dir")
            .count(),
        0
    );
}

#[test]
fn dropping_a_scratch_dir_removes_what_it_holds() {
    let path = {
        let dir = scratch();
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/file.bin"), b"content").unwrap();
        dir.to_path_buf()
    };
    assert!(!path.exists(), "{} outlived its ScratchDir", path.display());
}
