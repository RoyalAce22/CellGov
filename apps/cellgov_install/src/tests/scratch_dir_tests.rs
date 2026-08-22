//! Per-process uniqueness and drop-time cleanup of test scratch directories.

use super::scratch;

#[test]
fn two_scratch_dirs_from_one_process_do_not_share_a_path() {
    let a = scratch();
    let b = scratch();
    assert_ne!(a.to_path_buf(), b.to_path_buf());
    assert!(a.is_dir() && b.is_dir());
}

#[test]
fn a_scratch_path_carries_the_process_id() {
    let dir = scratch();
    let name = dir
        .file_name()
        .expect("scratch dir has a file name")
        .to_string_lossy()
        .into_owned();
    assert!(
        name.contains(&std::process::id().to_string()),
        "scratch name {name} omits the pid"
    );
}

#[test]
fn dropping_a_scratch_dir_removes_its_contents() {
    let path = {
        let dir = scratch();
        std::fs::write(dir.join("leftover.bin"), b"x").expect("write");
        dir.to_path_buf()
    };
    assert!(!path.exists(), "{} survived drop", path.display());
}

#[test]
fn a_scratch_dir_is_removed_when_a_test_body_panics() {
    let seen = std::cell::RefCell::new(std::path::PathBuf::new());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let dir = scratch();
        *seen.borrow_mut() = dir.to_path_buf();
        std::fs::write(dir.join("leftover.bin"), b"x").expect("write");
        panic!("deliberate unwind past a live ScratchDir");
    }));
    assert!(outcome.is_err(), "closure was expected to panic");
    let path = seen.into_inner();
    assert!(!path.exists(), "{} survived unwind", path.display());
}

#[test]
fn a_fresh_scratch_dir_is_empty() {
    let dir = scratch();
    assert_eq!(std::fs::read_dir(&*dir).expect("read_dir").count(), 0);
}
