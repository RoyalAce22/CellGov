use std::path::Path;

use super::*;

#[test]
fn a_byte_count_renders_in_the_unit_an_operator_reads() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(1023), "1023 B");
    assert_eq!(human_bytes(1024), "1.0 KB");
    assert_eq!(human_bytes(1024 * 1024), "1.0 MB");
    assert_eq!(human_bytes(13_314_398_617), "12.4 GB");
}

#[test]
fn a_count_past_the_largest_unit_stays_in_that_unit() {
    let rendered = human_bytes(u64::MAX);
    assert!(rendered.ends_with(" TB"), "{rendered}");
}

#[test]
fn a_count_that_rounds_up_into_the_next_unit_is_reported_in_it() {
    assert_eq!(human_bytes(1024 * 1024 - 1), "1.0 MB");
    assert_eq!(human_bytes(1024 * 1024 * 1024 - 1), "1.0 GB");
}

#[test]
fn a_tree_sums_every_file_under_it() {
    let dir = std::env::temp_dir().join(format!("cellgov_read_tests_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("a").join("b")).expect("create the tree");
    std::fs::write(dir.join("top"), [0u8; 10]).expect("write");
    std::fs::write(dir.join("a").join("mid"), [0u8; 20]).expect("write");
    std::fs::write(dir.join("a").join("b").join("leaf"), [0u8; 30]).expect("write");

    let size = tree_bytes(&dir);
    assert_eq!(size.bytes, 60);
    assert_eq!(size.unreadable, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_absent_tree_holds_nothing() {
    let size = tree_bytes(Path::new("no-such-store-root"));
    assert_eq!(size.bytes, 0);
    assert_eq!(size.unreadable, 0);
}

/// A plain file refuses `read_dir`, so it stands in for any path the
/// walk cannot enumerate.
#[test]
fn a_path_the_walk_cannot_read_is_counted_rather_than_dropped() {
    let dir = std::env::temp_dir().join(format!("cellgov_read_refused_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the tree");
    let not_a_dir = dir.join("plain-file");
    std::fs::write(&not_a_dir, [0u8; 7]).expect("write");

    let size = tree_bytes(&not_a_dir);
    assert_eq!(size.bytes, 0);
    assert_eq!(size.unreadable, 1, "a refused directory is named, not zero");
    std::fs::remove_dir_all(&dir).ok();
}

/// Only Unix creates a symlink without an elevated process.
#[cfg(unix)]
#[test]
fn an_entry_the_walk_does_not_follow_is_counted_rather_than_dropped() {
    let dir = std::env::temp_dir().join(format!("cellgov_read_link_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the tree");
    let target = dir.join("target");
    std::fs::write(&target, [0u8; 40]).expect("write");
    std::os::unix::fs::symlink(&target, dir.join("link")).expect("link");

    let size = tree_bytes(&dir);
    assert_eq!(size.bytes, 40, "the target is counted once, through itself");
    assert_eq!(size.unreadable, 1, "the link is named, not silently zero");
    std::fs::remove_dir_all(&dir).ok();
}
