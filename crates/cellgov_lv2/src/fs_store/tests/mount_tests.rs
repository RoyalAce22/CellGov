//! Mount-table resolution tests -- prefix matching, path normalization, root ordering, and traversal rejection.

use super::*;

fn standard_table() -> FsMountTable {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/app_home", PathBuf::from("/host/app")).unwrap())
        .unwrap();
    t.add(FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd0")).unwrap())
        .unwrap();
    t
}

fn candidates(t: &FsMountTable, guest_path: &str) -> Vec<PathBuf> {
    t.resolve_candidates(guest_path)
        .unwrap()
        .expect("path must match a mount")
}

#[test]
fn resolve_simple_app_home() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home/Data/first.xml"),
        vec![PathBuf::from("/host/app/Data/first.xml")]
    );
}

#[test]
fn resolve_strips_dot_segments() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home/./Data/./first.xml"),
        vec![PathBuf::from("/host/app/Data/first.xml")]
    );
}

#[test]
fn resolve_collapses_double_slashes() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home//Data//first.xml"),
        vec![PathBuf::from("/host/app/Data/first.xml")]
    );
}

#[test]
fn resolve_rejects_dotdot_traversal() {
    let t = standard_table();
    assert_eq!(
        t.resolve_candidates("/app_home/../etc/passwd"),
        Err(FsError::PathTraversal)
    );
    assert_eq!(
        t.resolve_candidates("/app_home/Data/../../etc/passwd"),
        Err(FsError::PathTraversal)
    );
}

#[test]
fn resolve_rejects_dotdot_before_joining_any_root() {
    let mut t = FsMountTable::new();
    t.add(
        FsMount::with_roots(
            "/app_home",
            vec![PathBuf::from("/update"), PathBuf::from("/base")],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        t.resolve_candidates("/app_home/../etc/passwd"),
        Err(FsError::PathTraversal)
    );
}

#[test]
fn resolve_returns_none_for_no_mount() {
    let t = standard_table();
    assert_eq!(t.resolve_candidates("/dev_flash/foo").unwrap(), None);
}

#[test]
fn resolve_handles_exact_prefix_match() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home"),
        vec![PathBuf::from("/host/app")]
    );
}

#[test]
fn resolve_handles_prefix_with_trailing_slash() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home/"),
        vec![PathBuf::from("/host/app")]
    );
}

#[test]
fn resolve_partial_prefix_does_not_match() {
    let t = standard_table();
    assert_eq!(t.resolve_candidates("/app_homeFoo").unwrap(), None);
    assert_eq!(t.resolve_candidates("/app_homeFoo/bar").unwrap(), None);
}

#[test]
fn resolve_picks_first_matching_mount() {
    let mut t = FsMountTable::new();
    t.add(FsMount::new("/app_home", PathBuf::from("/first")).unwrap())
        .unwrap();
    t.add(FsMount::new("/app_home_alt", PathBuf::from("/second")).unwrap())
        .unwrap();
    assert_eq!(
        candidates(&t, "/app_home/x"),
        vec![PathBuf::from("/first/x")]
    );
    assert_eq!(
        candidates(&t, "/app_home_alt/x"),
        vec![PathBuf::from("/second/x")]
    );
}

#[test]
fn resolve_lists_every_root_in_declaration_order() {
    let mut t = FsMountTable::new();
    t.add(
        FsMount::with_roots(
            "/app_home",
            vec![PathBuf::from("/update"), PathBuf::from("/base")],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        candidates(&t, "/app_home/USRDIR/EBOOT.BIN"),
        vec![
            PathBuf::from("/update/USRDIR/EBOOT.BIN"),
            PathBuf::from("/base/USRDIR/EBOOT.BIN"),
        ]
    );
}

#[test]
fn resolve_is_a_pure_function_of_path_and_roots() {
    let build = || {
        let mut t = FsMountTable::new();
        t.add(
            FsMount::with_roots(
                "/app_home",
                vec![PathBuf::from("/update"), PathBuf::from("/base")],
            )
            .unwrap(),
        )
        .unwrap();
        t
    };
    // Neither root exists on any host that runs this test. A
    // candidate list from a disk probe would be shorter.
    let expected = vec![
        PathBuf::from("/update/Data/x.xml"),
        PathBuf::from("/base/Data/x.xml"),
    ];
    let first = build();
    assert_eq!(candidates(&first, "/app_home/Data/x.xml"), expected);
    assert!(first
        .resolve_candidates("/app_home/other")
        .unwrap()
        .is_some());
    assert_eq!(candidates(&first, "/app_home/Data/x.xml"), expected);
    assert_eq!(candidates(&build(), "/app_home/Data/x.xml"), expected);
}

#[test]
fn resolve_rejects_a_segment_carrying_a_host_separator_or_drive_marker() {
    let t = standard_table();
    for guest in [
        "/app_home/..\\..\\etc/passwd",
        "/app_home/\\Windows/System32",
        "/app_home/C:/Windows",
        "/app_home/\\\\server\\share/x",
        "/app_home/Data/name:stream",
    ] {
        assert_eq!(
            t.resolve_candidates(guest),
            Err(FsError::PathTraversal),
            "{guest}"
        );
    }
}

#[test]
fn a_segment_that_could_leave_a_root_is_refused_before_any_root_is_joined() {
    let mut t = FsMountTable::new();
    t.add(
        FsMount::with_roots(
            "/app_home",
            vec![PathBuf::from("/update"), PathBuf::from("/base")],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        t.resolve_candidates("/app_home/C:/Windows"),
        Err(FsError::PathTraversal)
    );
}

#[test]
fn a_multibyte_segment_resolves_and_a_multibyte_near_prefix_does_not_match() {
    let t = standard_table();
    assert_eq!(
        candidates(&t, "/app_home/\u{00e9}t\u{00e9}/x.xml"),
        vec![PathBuf::from("/host/app/\u{00e9}t\u{00e9}/x.xml")]
    );
    assert_eq!(t.resolve_candidates("/app_home\u{00e9}").unwrap(), None);
    assert_eq!(t.resolve_candidates("/app_home\u{00e9}/x").unwrap(), None);
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
fn mount_with_roots_rejects_an_empty_root_list() {
    assert!(FsMount::with_roots("/app_home", Vec::new()).is_none());
}

#[test]
fn mount_new_is_a_one_root_mount() {
    let m = FsMount::new("/app_home", PathBuf::from("/x")).unwrap();
    assert_eq!(m.roots(), [PathBuf::from("/x")]);
}

#[test]
fn empty_table_resolves_nothing() {
    let t = FsMountTable::new();
    assert_eq!(t.resolve_candidates("/app_home/foo").unwrap(), None);
    assert_eq!(t.resolve_candidates("/").unwrap(), None);
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
    assert_eq!(candidates(&t, "/"), vec![PathBuf::from("/host")]);
    assert_eq!(
        candidates(&t, "/foo/bar"),
        vec![PathBuf::from("/host/foo/bar")]
    );
}
