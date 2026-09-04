use super::*;

#[test]
fn a_scratch_directory_exists_while_held_and_is_gone_after_drop() {
    let path = {
        let s = scratch();
        std::fs::write(s.join("f"), b"x").unwrap();
        assert!(s.is_dir());
        s.to_path_buf()
    };
    assert!(!path.exists(), "{} outlived its guard", path.display());
}

#[test]
fn a_fresh_scratch_directory_is_empty() {
    let s = scratch();
    assert_eq!(std::fs::read_dir(&*s).unwrap().count(), 0);
}

#[test]
fn two_scratch_directories_never_share_a_path() {
    let a = scratch();
    let b = scratch();
    assert_ne!(&*a, &*b);
}

#[test]
fn a_label_reaches_the_directory_name() {
    let s = scratch_labeled("mount_probe");
    let name = s.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.contains("mount_probe"), "{name}");
    assert!(name.starts_with(PREFIX), "{name}");
}

#[test]
fn a_label_that_is_not_a_plain_name_cannot_escape_the_directory() {
    let s = scratch_labeled("../../etc/passwd");
    let name = s.file_name().unwrap().to_string_lossy().into_owned();
    assert!(!name.contains('.'), "{name}");
    assert!(!name.contains('/') && !name.contains('\\'), "{name}");
    assert_eq!(s.parent(), Some(std::env::temp_dir().as_path()));
}

/// The fold runs per character, so a label outside ASCII still leaves
/// a name a Windows console can render.
#[test]
fn a_label_outside_ascii_folds_to_an_ascii_name() {
    let s = scratch_labeled("caf\u{e9}_\u{4e2d}");
    let name = s.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.is_ascii(), "{name}");
    assert!(name.starts_with(PREFIX), "{name}");
    assert!(s.is_dir());
}

#[test]
fn an_empty_label_still_names_a_directory() {
    let s = scratch_labeled("");
    let name = s.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(PREFIX), "{name}");
    assert!(s.is_dir());
}

#[test]
fn a_panic_through_the_guard_still_removes_the_tree() {
    let seen = std::sync::Mutex::new(None);
    let caught = std::panic::catch_unwind(|| {
        let s = scratch();
        std::fs::write(s.join("half-written"), b"x").unwrap();
        *seen.lock().unwrap() = Some(s.to_path_buf());
        panic!("the assertion this test stands in for");
    });
    assert!(caught.is_err(), "the panic has to reach the caller");
    let path = seen.lock().unwrap().clone().expect("the path was recorded");
    assert!(!path.exists(), "{} survived an unwind", path.display());
}

#[test]
fn a_populated_scratch_tree_is_removed_whole() {
    let path = {
        let s = scratch();
        std::fs::create_dir_all(s.join("a").join("b")).unwrap();
        std::fs::write(s.join("a").join("b").join("c"), b"x").unwrap();
        s.to_path_buf()
    };
    assert!(!path.exists());
}
