//! Mount-table resolution tests -- prefix matching, path normalization, and traversal rejection.

use super::*;

fn standard_table() -> FsMountTable {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/app_home", PathBuf::from("/host/app")).unwrap())
        .unwrap();
    t.add(FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd0")).unwrap())
        .unwrap();
    t
}

#[test]
fn resolve_simple_app_home() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home/Data/first.xml").unwrap(),
        Some(PathBuf::from("/host/app/Data/first.xml"))
    );
}

#[test]
fn resolve_strips_dot_segments() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home/./Data/./first.xml").unwrap(),
        Some(PathBuf::from("/host/app/Data/first.xml"))
    );
}

#[test]
fn resolve_collapses_double_slashes() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home//Data//first.xml").unwrap(),
        Some(PathBuf::from("/host/app/Data/first.xml"))
    );
}

#[test]
fn resolve_rejects_dotdot_traversal() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home/../etc/passwd"),
        Err(FsError::PathTraversal)
    );
    assert_eq!(
        t.resolve("/app_home/Data/../../etc/passwd"),
        Err(FsError::PathTraversal)
    );
}

#[test]
fn resolve_returns_none_for_no_mount() {
    let t = standard_table();
    assert_eq!(t.resolve("/dev_flash/foo").unwrap(), None);
}

#[test]
fn resolve_handles_exact_prefix_match() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home").unwrap(),
        Some(PathBuf::from("/host/app"))
    );
}

#[test]
fn resolve_handles_prefix_with_trailing_slash() {
    let t = standard_table();
    assert_eq!(
        t.resolve("/app_home/").unwrap(),
        Some(PathBuf::from("/host/app"))
    );
}

#[test]
fn resolve_partial_prefix_does_not_match() {
    let t = standard_table();
    assert_eq!(t.resolve("/app_homeFoo").unwrap(), None);
    assert_eq!(t.resolve("/app_homeFoo/bar").unwrap(), None);
}

#[test]
fn resolve_picks_first_matching_mount() {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/app_home", PathBuf::from("/first")).unwrap())
        .unwrap();
    t.add(FsMount::new("/app_home_alt", PathBuf::from("/second")).unwrap())
        .unwrap();
    assert_eq!(
        t.resolve("/app_home/x").unwrap(),
        Some(PathBuf::from("/first/x"))
    );
    assert_eq!(
        t.resolve("/app_home_alt/x").unwrap(),
        Some(PathBuf::from("/second/x"))
    );
}

#[test]
fn add_rejects_duplicate_prefix() {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/app_home", PathBuf::from("/a")).unwrap())
        .unwrap();
    let err = t
        .add(FsMount::new("/app_home", PathBuf::from("/b")).unwrap())
        .unwrap_err();
    assert_eq!(err, FsError::MountAlreadyRegistered);
}

#[test]
fn mount_new_normalizes_trailing_slash() {
    let m = FsMount::new("/app_home/", PathBuf::from("/x")).unwrap();
    assert_eq!(m.prefix, "/app_home");
}

#[test]
fn mount_new_rejects_relative_prefix() {
    assert!(FsMount::new("app_home", PathBuf::from("/x")).is_none());
    assert!(FsMount::new("", PathBuf::from("/x")).is_none());
}

#[test]
fn mount_new_rejects_dotdot_in_prefix() {
    assert!(FsMount::new("/app_home/..", PathBuf::from("/x")).is_none());
    assert!(FsMount::new("/../etc", PathBuf::from("/x")).is_none());
}

#[test]
fn empty_table_resolves_nothing() {
    let t = FsMountTable::new();
    assert_eq!(t.resolve("/app_home/foo").unwrap(), None);
    assert_eq!(t.resolve("/").unwrap(), None);
}

#[test]
fn mounts_iterates_in_registration_order() {
    let t = standard_table();
    let prefixes: Vec<&str> = t.mounts().map(|m| m.prefix.as_str()).collect();
    assert_eq!(prefixes, vec!["/app_home", "/dev_hdd0"]);
}

#[test]
fn resolve_root_mount_with_subpath() {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/", PathBuf::from("/host")).unwrap())
        .unwrap();
    assert_eq!(t.resolve("/").unwrap(), Some(PathBuf::from("/host")));
    assert_eq!(
        t.resolve("/foo/bar").unwrap(),
        Some(PathBuf::from("/host/foo/bar"))
    );
}
