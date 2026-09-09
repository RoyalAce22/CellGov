//! Mount-entry host-path validation, resolution, and registration.

use super::*;

struct TmpDir(cellgov_testkit::scratch::ScratchDir);
impl TmpDir {
    fn new(name: &str) -> Self {
        Self(cellgov_testkit::scratch::scratch_labeled(name))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

fn entry(prefix: &str, host: &str, override_env: Option<&str>) -> MountEntry {
    MountEntry {
        prefix: prefix.to_string(),
        host: Some(host.to_string()),
        override_env: override_env.map(|s| s.to_string()),
    }
}

fn hostless_entry(prefix: &str, override_env: Option<&str>) -> MountEntry {
    MountEntry {
        prefix: prefix.to_string(),
        host: None,
        override_env: override_env.map(|s| s.to_string()),
    }
}

/// Canonicalize on both sides so the `\\?\` prefix Windows adds
/// does not break portable assertions.
fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).expect("canonicalize")
}

fn mount_roots(host: &Lv2Host) -> Vec<PathBuf> {
    host.fs_mounts()
        .mounts()
        .next()
        .expect("one mount")
        .roots()
        .to_vec()
}

#[test]
fn validate_host_shape_rejects_empty() {
    let err = validate_host_shape("/p", "").unwrap_err();
    assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
}

#[test]
fn validate_host_shape_rejects_dotdot_segment() {
    let err = validate_host_shape("/p", "assets/../escape").unwrap_err();
    match err {
        MountRegisterError::InvalidHost { reason, .. } => {
            assert!(reason.contains(".."), "reason names the rule");
        }
        other => panic!("expected InvalidHost, got {other}"),
    }
    assert!(validate_host_shape("/p", "../foo").is_err());
    assert!(validate_host_shape("/p", "/abs/../escape").is_err());
}

#[test]
fn validate_host_shape_rejects_windows_drive_letter() {
    for shape in ["C:\\foo", "C:/foo", "C:", "Z:\\Users\\me"] {
        let err = validate_host_shape("/p", shape).expect_err(shape);
        assert!(
            matches!(err, MountRegisterError::InvalidHost { .. }),
            "{shape:?} -> {err:?}",
        );
    }
}

#[test]
fn validate_host_shape_rejects_unc_and_backslash_root() {
    for shape in ["\\\\server\\share", "\\foo"] {
        let err = validate_host_shape("/p", shape).expect_err(shape);
        assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
    }
}

#[test]
fn validate_host_shape_rejects_mid_string_backslash() {
    let inputs = ["foo\\..\\bar", "assets\\sub", "a/b\\c"];
    for shape in inputs {
        let err = validate_host_shape("/p", shape).expect_err(shape);
        match err {
            MountRegisterError::InvalidHost { reason, .. } => {
                assert!(
                    reason.contains("backslash"),
                    "{shape:?} reason should name backslash: {reason}",
                );
            }
            other => panic!("{shape:?} -> {other:?}"),
        }
    }
}

#[test]
fn validate_host_shape_accepts_posix_shapes() {
    validate_host_shape("/p", "tests/fixtures/foo").unwrap();
    validate_host_shape("/p", "/abs/path").unwrap();
    validate_host_shape("/p", "./relative").unwrap();
}

#[test]
fn resolve_against_treats_leading_slash_as_absolute_on_every_platform() {
    let r = resolve_against(Path::new("/workspace"), "/abs");
    assert_eq!(r, PathBuf::from("/abs"));
}

#[test]
fn resolve_against_joins_relative_paths_under_base() {
    let r = resolve_against(Path::new("/workspace"), "assets/sub");
    assert_eq!(r, PathBuf::from("/workspace").join("assets/sub"));
}

#[test]
fn register_zero_entries_succeeds_with_count_zero() {
    let mut host = Lv2Host::new();
    let baseline = host.fs_mounts().mounts().count();
    let n = register_mounts(&[], Path::new("/unused"), None, |_| None, &mut host).unwrap();
    assert_eq!(n, 0);
    assert_eq!(host.fs_mounts().mounts().count(), baseline);
}

#[test]
fn relative_host_resolves_under_workspace_and_canonicalizes() {
    let workspace = TmpDir::new("rel_host");
    std::fs::create_dir_all(workspace.path().join("assets")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "assets", None)];
    let n = register_mounts(&entries, workspace.path(), None, |_| None, &mut host).unwrap();
    assert_eq!(n, 1);
    let roots = mount_roots(&host);
    assert_eq!(roots, vec![canon(&workspace.path().join("assets"))]);
}

#[test]
fn missing_host_root_returns_typed_missing_error() {
    let workspace = TmpDir::new("missing_root");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "does/not/exist", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("missing host root must surface");
    match err {
        MountRegisterError::HostRootMissing {
            prefix,
            override_env,
            ..
        } => {
            assert_eq!(prefix, "/app_home");
            assert!(override_env.is_none());
        }
        other => panic!("expected HostRootMissing, got {other}"),
    }
}

#[test]
fn host_root_pointing_to_a_file_returns_not_directory_error() {
    let workspace = TmpDir::new("file_root");
    std::fs::write(workspace.path().join("not_a_dir"), b"x").unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "not_a_dir", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("file-as-mount must surface");
    match err {
        MountRegisterError::HostRootNotDirectory { prefix, .. } => {
            assert_eq!(prefix, "/app_home");
        }
        other => panic!("expected HostRootNotDirectory, got {other}"),
    }
}

#[test]
fn host_path_with_nul_byte_returns_host_root_io() {
    // NUL passes shape validation but metadata() rejects with
    // InvalidInput, exercising the HostRootIo arm portably.
    let workspace = TmpDir::new("nul_host");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "foo\0bar", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("NUL-byte host must surface");
    match err {
        MountRegisterError::HostRootIo { prefix, source, .. } => {
            assert_eq!(prefix, "/app_home");
            assert!(!source.to_string().is_empty());
        }
        other => panic!("expected HostRootIo, got {other}"),
    }
}

#[test]
fn override_env_replaces_manifest_host() {
    let workspace = TmpDir::new("override_workspace");
    std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
    std::fs::create_dir_all(workspace.path().join("real_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry(
        "/app_home",
        "manifest_dir",
        Some("CELLGOV_TEST_OVERRIDE"),
    )];
    let getter = |name: &str| {
        if name == "CELLGOV_TEST_OVERRIDE" {
            Some("real_dir".to_string())
        } else {
            None
        }
    };
    register_mounts(&entries, workspace.path(), None, getter, &mut host).unwrap();
    let roots = mount_roots(&host);
    assert_eq!(roots, vec![canon(&workspace.path().join("real_dir"))]);
}

#[test]
fn empty_override_env_value_falls_through_to_manifest_host() {
    let workspace = TmpDir::new("empty_override");
    std::fs::create_dir_all(workspace.path().join("fallback")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry(
        "/app_home",
        "fallback",
        Some("CELLGOV_EMPTY_OVERRIDE"),
    )];
    let getter = |name: &str| {
        if name == "CELLGOV_EMPTY_OVERRIDE" {
            Some(String::new())
        } else {
            None
        }
    };
    register_mounts(&entries, workspace.path(), None, getter, &mut host).unwrap();
    let roots = mount_roots(&host);
    assert_eq!(roots, vec![canon(&workspace.path().join("fallback"))]);
}

#[test]
fn whitespace_only_override_env_value_falls_through_to_manifest_host() {
    let workspace = TmpDir::new("ws_override");
    std::fs::create_dir_all(workspace.path().join("fallback")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "fallback", Some("CELLGOV_WS_OVERRIDE"))];
    let getter = |name: &str| {
        if name == "CELLGOV_WS_OVERRIDE" {
            Some(" \n\t ".to_string())
        } else {
            None
        }
    };
    register_mounts(&entries, workspace.path(), None, getter, &mut host).unwrap();
    let roots = mount_roots(&host);
    assert_eq!(roots, vec![canon(&workspace.path().join("fallback"))]);
}

#[test]
fn override_env_pointing_to_missing_dir_carries_env_name_in_error() {
    let workspace = TmpDir::new("env_missing_workspace");
    std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry(
        "/app_home",
        "manifest_dir",
        Some("CELLGOV_MISSING_OVERRIDE"),
    )];
    let getter = |name: &str| {
        if name == "CELLGOV_MISSING_OVERRIDE" {
            Some("nope/does/not/exist".to_string())
        } else {
            None
        }
    };
    let err = register_mounts(&entries, workspace.path(), None, getter, &mut host)
        .expect_err("env-pointed missing dir must surface");
    match err {
        MountRegisterError::HostRootMissing { override_env, .. } => {
            assert_eq!(override_env.as_deref(), Some("CELLGOV_MISSING_OVERRIDE"));
        }
        other => panic!("expected HostRootMissing with env name, got {other}"),
    }
}

#[test]
fn override_env_value_must_obey_host_shape_rules() {
    let workspace = TmpDir::new("env_shape");
    std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry(
        "/app_home",
        "manifest_dir",
        Some("CELLGOV_BAD_OVERRIDE"),
    )];
    let getter = |name: &str| {
        if name == "CELLGOV_BAD_OVERRIDE" {
            Some("../escape".to_string())
        } else {
            None
        }
    };
    let err = register_mounts(&entries, workspace.path(), None, getter, &mut host)
        .expect_err("env-supplied dotdot must surface");
    assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
}

#[test]
fn empty_override_env_name_is_rejected() {
    let workspace = TmpDir::new("empty_env_name");
    std::fs::create_dir_all(workspace.path().join("manifest_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "manifest_dir", Some(""))];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("empty env name must surface");
    match err {
        MountRegisterError::InvalidHost { reason, .. } => {
            assert!(reason.contains("override_env"), "reason names the field");
        }
        other => panic!("expected InvalidHost, got {other}"),
    }
}

#[test]
fn empty_prefix_is_rejected_before_io() {
    let workspace = TmpDir::new("empty_prefix");
    let mut host = Lv2Host::new();
    let entries = vec![entry("", "missing/dir", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("empty prefix must surface");
    assert!(matches!(err, MountRegisterError::InvalidPrefix { .. }));
}

#[test]
fn unrooted_prefix_is_rejected_before_io() {
    let workspace = TmpDir::new("unrooted_prefix");
    let mut host = Lv2Host::new();
    let entries = vec![entry("app_home", "missing/dir", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("non-rooted prefix must surface");
    match err {
        MountRegisterError::InvalidPrefix { prefix, reason } => {
            assert_eq!(prefix, "app_home");
            assert!(reason.contains("/"), "reason should mention rooting");
        }
        other => panic!("expected InvalidPrefix, got {other}"),
    }
}

#[test]
fn dotdot_in_prefix_is_rejected_before_io() {
    let workspace = TmpDir::new("dotdot_prefix");
    std::fs::create_dir_all(workspace.path().join("ok_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home/../etc", "ok_dir", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("invalid prefix must surface");
    assert!(matches!(err, MountRegisterError::InvalidPrefix { .. }));
}

#[test]
fn empty_host_string_is_rejected_before_io() {
    let workspace = TmpDir::new("empty_host_workspace");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("empty host must surface");
    match err {
        MountRegisterError::InvalidHost { prefix, host, .. } => {
            assert_eq!(prefix, "/app_home");
            assert_eq!(host, "");
        }
        other => panic!("expected InvalidHost, got {other}"),
    }
}

#[test]
fn dotdot_in_host_is_rejected() {
    let workspace = TmpDir::new("dotdot_host");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "../escape", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("dotdot host must surface");
    match err {
        MountRegisterError::InvalidHost { reason, .. } => {
            assert!(reason.contains(".."), "reason names the rule");
        }
        other => panic!("expected InvalidHost, got {other}"),
    }
}

#[test]
fn windows_shape_host_is_rejected_for_determinism() {
    let workspace = TmpDir::new("win_host");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "C:\\Users\\me\\flow", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("windows-shape host must surface");
    assert!(matches!(err, MountRegisterError::InvalidHost { .. }));
}

#[test]
fn invalid_prefix_takes_precedence_over_missing_host() {
    let workspace = TmpDir::new("precedence");
    let mut host = Lv2Host::new();
    let entries = vec![entry("not_rooted", "path/does/not/exist", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("must surface");
    assert!(
        matches!(err, MountRegisterError::InvalidPrefix { .. }),
        "expected InvalidPrefix, got {err:?}",
    );
}

#[test]
fn registration_order_matches_manifest_order() {
    let workspace = TmpDir::new("order");
    std::fs::create_dir_all(workspace.path().join("app_home")).unwrap();
    std::fs::create_dir_all(workspace.path().join("dev_hdd0")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![
        entry("/dev_hdd0", "dev_hdd0", None),
        entry("/app_home", "app_home", None),
    ];
    register_mounts(&entries, workspace.path(), None, |_| None, &mut host).unwrap();
    let prefixes: Vec<&str> = host
        .fs_mounts()
        .mounts()
        .map(|m| m.prefix.as_str())
        .collect();
    assert_eq!(prefixes, vec!["/dev_hdd0", "/app_home"]);
}

#[test]
fn first_failure_leaves_host_mount_table_untouched() {
    let workspace = TmpDir::new("atomicity");
    std::fs::create_dir_all(workspace.path().join("ok_dir")).unwrap();
    let mut host = Lv2Host::new();
    let baseline = host.fs_mounts().mounts().count();
    let entries = vec![
        entry("/ok", "ok_dir", None),
        entry("/missing", "does_not_exist", None),
    ];
    let _err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("entry 1 must fail");
    assert_eq!(
        host.fs_mounts().mounts().count(),
        baseline,
        "host mount table must be untouched after first-failure",
    );
}

#[test]
fn in_slice_duplicate_prefix_is_rejected() {
    let workspace = TmpDir::new("slice_dup");
    std::fs::create_dir_all(workspace.path().join("a")).unwrap();
    std::fs::create_dir_all(workspace.path().join("b")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "a", None), entry("/app_home", "b", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("in-slice duplicate must surface");
    assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}

#[test]
fn trailing_slash_prefix_dedup_matches_fsmount_normalization() {
    // FsMount::new strips trailing `/`, so `/app_home` and
    // `/app_home/` collide after normalization.
    let workspace = TmpDir::new("trailing_slash");
    std::fs::create_dir_all(workspace.path().join("a")).unwrap();
    std::fs::create_dir_all(workspace.path().join("b")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![
        entry("/app_home", "a", None),
        entry("/app_home/", "b", None),
    ];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("trailing-slash collision must surface");
    assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
}

#[test]
fn cross_call_duplicate_prefix_is_rejected() {
    let workspace = TmpDir::new("cross_call");
    std::fs::create_dir_all(workspace.path().join("a")).unwrap();
    std::fs::create_dir_all(workspace.path().join("b")).unwrap();
    let mut host = Lv2Host::new();
    register_mounts(
        &[entry("/app_home", "a", None)],
        workspace.path(),
        None,
        |_| None,
        &mut host,
    )
    .unwrap();
    let err = register_mounts(
        &[entry("/app_home", "b", None)],
        workspace.path(),
        None,
        |_| None,
        &mut host,
    )
    .expect_err("cross-call duplicate must surface");
    assert!(matches!(err, MountRegisterError::DuplicatePrefix { .. }));
    assert_eq!(host.fs_mounts().mounts().count(), 1);
}

#[test]
fn disjoint_register_mounts_calls_compose() {
    let workspace = TmpDir::new("disjoint");
    std::fs::create_dir_all(workspace.path().join("a")).unwrap();
    std::fs::create_dir_all(workspace.path().join("b")).unwrap();
    let mut host = Lv2Host::new();
    register_mounts(
        &[entry("/app_home", "a", None)],
        workspace.path(),
        None,
        |_| None,
        &mut host,
    )
    .unwrap();
    register_mounts(
        &[entry("/dev_hdd0", "b", None)],
        workspace.path(),
        None,
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(host.fs_mounts().mounts().count(), 2);
}

#[test]
fn absent_host_defaults_to_the_eboot_directory() {
    let workspace = TmpDir::new("hostless_workspace");
    let usrdir = TmpDir::new("hostless_usrdir");
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry("/app_home", Some("CELLGOV_HOSTLESS_UNSET"))];
    let n = register_mounts(
        &entries,
        workspace.path(),
        Some(usrdir.path()),
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(mount_roots(&host), vec![canon(usrdir.path())]);
}

#[test]
fn override_env_replaces_the_default_eboot_directory() {
    let workspace = TmpDir::new("hostless_override_workspace");
    let usrdir = TmpDir::new("hostless_override_usrdir");
    std::fs::create_dir_all(workspace.path().join("real_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry("/app_home", Some("CELLGOV_HOSTLESS_SET"))];
    let getter = |name: &str| {
        if name == "CELLGOV_HOSTLESS_SET" {
            Some("real_dir".to_string())
        } else {
            None
        }
    };
    register_mounts(
        &entries,
        workspace.path(),
        Some(usrdir.path()),
        getter,
        &mut host,
    )
    .unwrap();
    assert_eq!(
        mount_roots(&host),
        vec![canon(&workspace.path().join("real_dir"))]
    );
}

#[test]
fn absent_host_and_no_eboot_directory_is_a_no_host_error() {
    let workspace = TmpDir::new("hostless_nowhere");
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry(
        "/app_home",
        Some("CELLGOV_HOSTLESS_NOWHERE"),
    )];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("no host anywhere must surface");
    let msg = err.to_string();
    match err {
        MountRegisterError::NoHost {
            prefix,
            override_env,
        } => {
            assert_eq!(prefix, "/app_home");
            assert_eq!(override_env.as_deref(), Some("CELLGOV_HOSTLESS_NOWHERE"));
        }
        other => panic!("expected NoHost, got {other}"),
    }
    assert!(
        msg.contains("CELLGOV_HOSTLESS_NOWHERE is unset"),
        "Display names the env var the developer can set, got: {msg}",
    );
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}

#[test]
fn declared_host_wins_over_the_eboot_directory() {
    let workspace = TmpDir::new("declared_vs_usrdir");
    let usrdir = TmpDir::new("declared_vs_usrdir_usrdir");
    std::fs::create_dir_all(workspace.path().join("declared")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "declared", None)];
    register_mounts(
        &entries,
        workspace.path(),
        Some(usrdir.path()),
        |_| None,
        &mut host,
    )
    .unwrap();
    assert_eq!(
        mount_roots(&host),
        vec![canon(&workspace.path().join("declared"))]
    );
}

#[test]
fn override_env_alone_suffices_without_an_eboot_directory() {
    let workspace = TmpDir::new("hostless_env_only");
    std::fs::create_dir_all(workspace.path().join("real_dir")).unwrap();
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry(
        "/app_home",
        Some("CELLGOV_HOSTLESS_ENV_ONLY"),
    )];
    let getter = |name: &str| {
        if name == "CELLGOV_HOSTLESS_ENV_ONLY" {
            Some("real_dir".to_string())
        } else {
            None
        }
    };
    register_mounts(&entries, workspace.path(), None, getter, &mut host).unwrap();
    assert_eq!(
        mount_roots(&host),
        vec![canon(&workspace.path().join("real_dir"))]
    );
}

#[test]
fn whitespace_only_override_env_falls_through_to_the_eboot_directory() {
    let workspace = TmpDir::new("hostless_ws_workspace");
    let usrdir = TmpDir::new("hostless_ws_usrdir");
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry("/app_home", Some("CELLGOV_HOSTLESS_WS"))];
    let getter = |name: &str| {
        if name == "CELLGOV_HOSTLESS_WS" {
            Some("   ".to_string())
        } else {
            None
        }
    };
    register_mounts(
        &entries,
        workspace.path(),
        Some(usrdir.path()),
        getter,
        &mut host,
    )
    .unwrap();
    assert_eq!(mount_roots(&host), vec![canon(usrdir.path())]);
}

#[test]
fn an_empty_declared_host_does_not_fall_through_to_the_eboot_directory() {
    let workspace = TmpDir::new("empty_host_vs_usrdir");
    let usrdir = TmpDir::new("empty_host_vs_usrdir_usrdir");
    let mut host = Lv2Host::new();
    let entries = vec![entry("/app_home", "", None)];
    let err = register_mounts(
        &entries,
        workspace.path(),
        Some(usrdir.path()),
        |_| None,
        &mut host,
    )
    .expect_err("an empty host is a shape error, not an omitted one");
    assert!(
        matches!(err, MountRegisterError::InvalidHost { .. }),
        "expected InvalidHost, got {err:?}",
    );
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}

#[test]
fn absent_host_without_an_override_env_names_that_in_the_error() {
    let workspace = TmpDir::new("hostless_no_env_nowhere");
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry("/app_home", None)];
    let err = register_mounts(&entries, workspace.path(), None, |_| None, &mut host)
        .expect_err("no host anywhere must surface");
    let msg = err.to_string();
    match err {
        MountRegisterError::NoHost {
            prefix,
            override_env,
        } => {
            assert_eq!(prefix, "/app_home");
            assert!(override_env.is_none());
        }
        other => panic!("expected NoHost, got {other}"),
    }
    assert!(
        msg.contains("names no override_env"),
        "Display says there is no env var to set, got: {msg}",
    );
}

#[test]
fn an_empty_eboot_directory_counts_as_absent() {
    let workspace = TmpDir::new("hostless_empty_usrdir");
    let mut host = Lv2Host::new();
    let entries = vec![hostless_entry("/app_home", Some("CELLGOV_HOSTLESS_EMPTY"))];
    let err = register_mounts(
        &entries,
        workspace.path(),
        Some(Path::new("")),
        |_| None,
        &mut host,
    )
    .expect_err("an empty EBOOT directory is no directory");
    assert!(
        matches!(err, MountRegisterError::NoHost { .. }),
        "expected NoHost rather than a probe of \"\", got {err:?}",
    );
    assert_eq!(host.fs_mounts().mounts().count(), 0);
}
