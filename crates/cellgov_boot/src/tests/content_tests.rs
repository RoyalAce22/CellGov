//! Content-manifest blob registration into the LV2 FS store.

use super::*;
use crate::manifest::ContentEntry;

struct TmpDir(cellgov_testkit::scratch::ScratchDir);
impl TmpDir {
    fn new(name: &str) -> Self {
        Self(cellgov_testkit::scratch::scratch_labeled(name))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Baseline blob count from `Lv2Host::new()`; tests assert deltas
/// against this rather than absolute counts.
fn pristine_blob_count() -> usize {
    Lv2Host::new().fs_store().blob_count()
}

#[test]
fn registers_all_entries_in_fs_store() {
    let tmp = TmpDir::new("happy_path");
    write_file(&tmp.path().join("first.xml"), b"<root/>");
    write_file(&tmp.path().join("Localization.xml"), b"<i18n/>");
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![
            ContentEntry {
                guest_path: "/app_home/Data/Resources/first.xml".to_string(),
                host_path: "first.xml".to_string(),
            },
            ContentEntry {
                guest_path: "/app_home/Data/Local/Localization.xml".to_string(),
                host_path: "Localization.xml".to_string(),
            },
        ],
    };
    let mut host = Lv2Host::new();
    let baseline = host.fs_store().blob_count();
    register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[tmp.path().to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(host.fs_store().blob_count(), baseline + 2);
    assert_eq!(
        host.fs_store()
            .lookup_blob("/app_home/Data/Resources/first.xml"),
        Some(b"<root/>".as_slice()),
    );
    assert_eq!(
        host.fs_store()
            .lookup_blob("/app_home/Data/Local/Localization.xml"),
        Some(b"<i18n/>".as_slice()),
    );
}

#[test]
fn missing_host_file_is_a_startup_error() {
    let tmp = TmpDir::new("missing");
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![ContentEntry {
            guest_path: "/p".to_string(),
            host_path: "absent.xml".to_string(),
        }],
    };
    let mut host = Lv2Host::new();
    let err = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[tmp.path().to_path_buf()],
        &mut host,
    )
    .expect_err("missing host file must surface");
    match err {
        ContentRegisterError::HostFileRead {
            guest_path,
            override_env,
            ..
        } => {
            assert_eq!(guest_path, "/p");
            assert!(
                override_env.is_none(),
                "no override env in play for the USRDIR case",
            );
        }
        other => panic!("expected HostFileRead, got {other}"),
    }
    assert_eq!(host.fs_store().blob_count(), pristine_blob_count());
}

#[test]
fn relative_override_base_is_resolved_against_workspace_root() {
    let tmp = TmpDir::new("rel_override");
    let workspace = tmp.path();
    write_file(&workspace.join("fx").join("first.xml"), b"<r/>");
    let manifest = ContentManifest {
        override_base_env: Some("CELLGOV_TEST_REL_OVERRIDE".to_string()),
        files: vec![ContentEntry {
            guest_path: "/p/first.xml".to_string(),
            host_path: "first.xml".to_string(),
        }],
    };
    let mut host = Lv2Host::new();
    let source =
        register_content_blobs(&manifest, workspace, Some(Path::new("fx")), &[], &mut host)
            .unwrap();
    assert!(matches!(source, ContentBaseSource::Override { .. }));
    assert_eq!(
        host.fs_store().lookup_blob("/p/first.xml"),
        Some(b"<r/>".as_slice())
    );
}

#[test]
fn absolute_host_path_overrides_base() {
    let tmp = TmpDir::new("abs_host");
    let unrelated = TmpDir::new("abs_host_unrelated_base");
    write_file(&tmp.path().join("abs.xml"), b"<a/>");
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![ContentEntry {
            guest_path: "/p/abs.xml".to_string(),
            host_path: tmp.path().join("abs.xml").to_string_lossy().into_owned(),
        }],
    };
    let mut host = Lv2Host::new();
    register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[unrelated.path().to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(
        host.fs_store().lookup_blob("/p/abs.xml"),
        Some(b"<a/>".as_slice())
    );
}

#[test]
fn duplicate_guest_path_is_a_startup_error() {
    let tmp = TmpDir::new("dup");
    write_file(&tmp.path().join("a.xml"), b"a");
    write_file(&tmp.path().join("b.xml"), b"b");
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![
            ContentEntry {
                guest_path: "/dup".to_string(),
                host_path: "a.xml".to_string(),
            },
            ContentEntry {
                guest_path: "/dup".to_string(),
                host_path: "b.xml".to_string(),
            },
        ],
    };
    let mut host = Lv2Host::new();
    let err = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[tmp.path().to_path_buf()],
        &mut host,
    )
    .expect_err("duplicate guest_path must surface");
    match err {
        ContentRegisterError::DuplicateGuestPath { guest_path, .. } => {
            assert_eq!(guest_path, "/dup");
        }
        other => panic!("expected DuplicateGuestPath, got {other}"),
    }
    assert_eq!(host.fs_store().blob_count(), pristine_blob_count() + 1);
    assert_eq!(host.fs_store().lookup_blob("/dup"), Some(b"a".as_slice()));
}

#[test]
fn override_base_alone_is_sufficient() {
    let real = TmpDir::new("real_override");
    write_file(&real.path().join("first.xml"), b"REAL");
    let manifest = ContentManifest {
        override_base_env: Some("DOES_NOT_MATTER_FOR_THIS_TEST".to_string()),
        files: vec![ContentEntry {
            guest_path: "/first.xml".to_string(),
            host_path: "first.xml".to_string(),
        }],
    };
    let mut host = Lv2Host::new();
    let source = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        Some(real.path()),
        &[],
        &mut host,
    )
    .unwrap();
    assert!(matches!(source, ContentBaseSource::Override { .. }));
    assert_eq!(
        host.fs_store().lookup_blob("/first.xml"),
        Some(b"REAL".as_slice()),
        "override path's bytes must be the ones registered",
    );
}

#[test]
fn override_base_missing_file_error_carries_env_name() {
    let real = TmpDir::new("real_missing");
    let manifest = ContentManifest {
        override_base_env: Some("CELLGOV_TEST_OVERRIDE_DIR".to_string()),
        files: vec![ContentEntry {
            guest_path: "/p".to_string(),
            host_path: "absent.xml".to_string(),
        }],
    };
    let mut host = Lv2Host::new();
    let err = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        Some(real.path()),
        &[],
        &mut host,
    )
    .expect_err("missing override file must surface");
    let msg = format!("{}", err);
    match err {
        ContentRegisterError::HostFileRead {
            override_env: Some(env),
            ..
        } => {
            assert_eq!(env, "CELLGOV_TEST_OVERRIDE_DIR");
            assert!(
                msg.contains("CELLGOV_TEST_OVERRIDE_DIR"),
                "Display must name the override env var, got: {msg}",
            );
        }
        other => panic!("expected HostFileRead with override_env Some, got {other:?}"),
    }
}

#[test]
fn override_base_lookup_returns_none_when_env_unset() {
    let manifest = ContentManifest {
        override_base_env: Some("UNSET_ENV_VAR_FOR_TEST".to_string()),
        files: vec![],
    };
    let result = override_base_from_env(&manifest, |_| None);
    assert!(result.is_none());
}

#[test]
fn override_base_lookup_returns_none_for_empty_string() {
    let manifest = ContentManifest {
        override_base_env: Some("MAYBE_EMPTY".to_string()),
        files: vec![],
    };
    let result = override_base_from_env(&manifest, |name| {
        assert_eq!(name, "MAYBE_EMPTY");
        Some(String::new())
    });
    assert!(result.is_none());
}

#[test]
fn override_base_lookup_returns_none_for_whitespace_only() {
    // The mount provider reads the same env var and treats
    // whitespace as unset; the content provider must agree, or one
    // exported value redirects one provider and not the other.
    let manifest = ContentManifest {
        override_base_env: Some("MAYBE_BLANK".to_string()),
        files: vec![],
    };
    let result = override_base_from_env(&manifest, |_| Some(" \n\t ".to_string()));
    assert!(result.is_none());
}

#[test]
fn override_base_lookup_returns_path_when_env_set() {
    let manifest = ContentManifest {
        override_base_env: Some("SET_TO_PATH".to_string()),
        files: vec![],
    };
    let result = override_base_from_env(&manifest, |name| {
        assert_eq!(name, "SET_TO_PATH");
        Some("/tmp/local-flow".to_string())
    });
    assert_eq!(result, Some(PathBuf::from("/tmp/local-flow")));
}

#[test]
fn override_base_lookup_returns_none_when_no_env_var_declared() {
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![],
    };
    let result = override_base_from_env(&manifest, |_| {
        panic!("getter must not be called when override_base_env is None")
    });
    assert!(result.is_none());
}

/// A PSN title's two-XML data layout for USRDIR-resolution tests.
fn flow_shaped_manifest() -> ContentManifest {
    ContentManifest {
        override_base_env: Some("CELLGOV_TEST_TITLE_CONTENT_DIR".to_string()),
        files: vec![
            ContentEntry {
                guest_path: "/app_home/Data/Resources/first.xml".to_string(),
                host_path: "Data/Resources/first.xml".to_string(),
            },
            ContentEntry {
                guest_path: "/app_home/Data/Local/Localization.xml".to_string(),
                host_path: "Data/Local/Localization.xml".to_string(),
            },
        ],
    }
}

#[test]
fn usrdir_is_selected_when_no_override_is_set() {
    let usrdir = TmpDir::new("usrdir_real");
    write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
    write_file(&usrdir.path().join("Data/Local/Localization.xml"), b"USR");
    let manifest = flow_shaped_manifest();
    let mut host = Lv2Host::new();
    let source = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[usrdir.path().to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(
        source,
        ContentBaseSource::Usrdir {
            paths: vec![usrdir.path().to_path_buf()]
        }
    );
    assert_eq!(
        host.fs_store()
            .lookup_blob("/app_home/Data/Resources/first.xml"),
        Some(b"USR".as_slice()),
    );
}

#[test]
fn partial_usrdir_is_a_missing_file_error() {
    let usrdir = TmpDir::new("partial_usrdir");
    write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
    let manifest = flow_shaped_manifest();
    let mut host = Lv2Host::new();
    let err = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[usrdir.path().to_path_buf()],
        &mut host,
    )
    .expect_err("a USRDIR missing one entry must surface that entry");
    match err {
        ContentRegisterError::HostFileRead {
            guest_path,
            host_path,
            override_env,
            ..
        } => {
            assert_eq!(guest_path, "/app_home/Data/Local/Localization.xml");
            assert_eq!(
                host_path,
                usrdir.path().join("Data/Local/Localization.xml"),
                "the error names the exact path probed under the USRDIR",
            );
            assert!(override_env.is_none());
        }
        other => panic!("expected HostFileRead, got {other}"),
    }
}

#[test]
fn override_takes_priority_over_usrdir() {
    let usrdir = TmpDir::new("prio_usrdir");
    let override_dir = TmpDir::new("prio_override");
    write_file(&usrdir.path().join("Data/Resources/first.xml"), b"USR");
    write_file(&usrdir.path().join("Data/Local/Localization.xml"), b"USR");
    write_file(
        &override_dir.path().join("Data/Resources/first.xml"),
        b"OVR",
    );
    write_file(
        &override_dir.path().join("Data/Local/Localization.xml"),
        b"OVR",
    );
    let manifest = flow_shaped_manifest();
    let mut host = Lv2Host::new();
    let source = register_content_blobs(
        &manifest,
        Path::new("/unused"),
        Some(override_dir.path()),
        &[usrdir.path().to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert!(matches!(source, ContentBaseSource::Override { .. }));
    assert_eq!(
        host.fs_store()
            .lookup_blob("/app_home/Data/Resources/first.xml"),
        Some(b"OVR".as_slice()),
    );
}

#[test]
fn no_override_and_no_usrdir_is_a_startup_error() {
    let manifest = flow_shaped_manifest();
    let mut host = Lv2Host::new();
    let err = register_content_blobs(&manifest, Path::new("/unused"), None, &[], &mut host)
        .expect_err("no base at all must surface");
    let msg = err.to_string();
    match err {
        ContentRegisterError::NoBase { n, override_env } => {
            assert_eq!(n, 2);
            assert_eq!(
                override_env.as_deref(),
                Some("CELLGOV_TEST_TITLE_CONTENT_DIR")
            );
        }
        other => panic!("expected NoBase, got {other}"),
    }
    assert!(
        msg.contains("CELLGOV_TEST_TITLE_CONTENT_DIR is unset or empty"),
        "Display names the env var the developer can set, and that an \
         empty value counts as unset, got: {msg}",
    );
    assert!(msg.contains("2 manifest entries"), "got: {msg}");
    assert_eq!(host.fs_store().blob_count(), pristine_blob_count());
}

#[test]
fn no_base_error_without_a_declared_override_env_says_so() {
    let manifest = ContentManifest {
        override_base_env: None,
        files: vec![ContentEntry {
            guest_path: "/p".to_string(),
            host_path: "p.bin".to_string(),
        }],
    };
    let mut host = Lv2Host::new();
    let err = register_content_blobs(&manifest, Path::new("/unused"), None, &[], &mut host)
        .expect_err("no base at all must surface");
    let msg = err.to_string();
    assert!(matches!(err, ContentRegisterError::NoBase { n: 1, .. }));
    assert!(msg.contains("1 manifest entry:"), "got: {msg}");
    assert!(
        msg.contains("declares no override_base_env"),
        "Display says there is no env var to set, got: {msg}",
    );
}
