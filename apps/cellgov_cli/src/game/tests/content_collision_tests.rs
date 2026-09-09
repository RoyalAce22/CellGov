//! A guest-path collision names where the earlier registration came from.

use super::*;
use crate::game::manifest::ContentEntry;

fn scratch(name: &str) -> cellgov_testkit::scratch::ScratchDir {
    cellgov_testkit::scratch::scratch_labeled(name)
}

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn entry(guest_path: &str, host_path: &str) -> ContentEntry {
    ContentEntry {
        guest_path: guest_path.to_string(),
        host_path: host_path.to_string(),
    }
}

fn register(dir: &Path, files: Vec<ContentEntry>, host: &mut Lv2Host) -> ContentRegisterError {
    let manifest = ContentManifest {
        override_base_env: None,
        files,
    };
    register_content_blobs(
        &manifest,
        Path::new("/unused"),
        None,
        &[dir.to_path_buf()],
        host,
    )
    .expect_err("the manifest collides with an earlier registration")
}

/// A guest path `Lv2Host::new()` registers before any manifest is read.
const BUILT_IN: &str = "/app_home/PARAM.SFO";

#[test]
fn a_collision_with_a_host_built_in_blob_names_the_built_in_origin() {
    let dir = scratch("collision_built_in");
    write_file(&dir.join("PARAM.SFO"), b"sfo");
    let mut host = Lv2Host::new();
    assert!(host.fs_store().has_path(BUILT_IN), "the fixture premise");

    let err = register(&dir, vec![entry(BUILT_IN, "PARAM.SFO")], &mut host);
    let ContentRegisterError::GuestPathBuiltIn {
        guest_path,
        host_path,
    } = &err
    else {
        panic!("expected a built-in collision, got {err}");
    };
    assert_eq!(guest_path, BUILT_IN);
    assert_eq!(*host_path, dir.join("PARAM.SFO"));
    let msg = err.to_string();
    assert!(msg.contains("registers itself"), "{msg}");
    assert_eq!(
        msg.matches(&dir.join("PARAM.SFO").display().to_string())
            .count(),
        1,
        "the manifest path appears once, not as both sides: {msg}"
    );
}

#[test]
fn a_collision_between_two_manifest_entries_names_each_host_source_once() {
    let dir = scratch("collision_manifest");
    write_file(&dir.join("a.xml"), b"a");
    write_file(&dir.join("b.xml"), b"b");
    let mut host = Lv2Host::new();

    let err = register(
        &dir,
        vec![entry("/dup", "a.xml"), entry("/dup", "b.xml")],
        &mut host,
    );
    let ContentRegisterError::DuplicateGuestPath {
        guest_path,
        first_host_path,
        second_host_path,
    } = &err
    else {
        panic!("expected a manifest duplicate, got {err}");
    };
    assert_eq!(guest_path, "/dup");
    assert_eq!(*first_host_path, dir.join("a.xml"));
    assert_eq!(*second_host_path, dir.join("b.xml"));
}

#[test]
fn a_built_in_collision_after_a_registered_entry_keeps_that_entry() {
    let dir = scratch("collision_after_entry");
    write_file(&dir.join("first.xml"), b"<r/>");
    write_file(&dir.join("PARAM.SFO"), b"sfo");
    let mut host = Lv2Host::new();
    let baseline = host.fs_store().blob_count();

    let err = register(
        &dir,
        vec![
            entry("/app_home/first.xml", "first.xml"),
            entry(BUILT_IN, "PARAM.SFO"),
        ],
        &mut host,
    );
    assert!(
        matches!(&err, ContentRegisterError::GuestPathBuiltIn { guest_path, .. } if guest_path == BUILT_IN),
        "{err}"
    );
    assert_eq!(host.fs_store().blob_count(), baseline + 1);
    assert_eq!(
        host.fs_store().lookup_blob("/app_home/first.xml"),
        Some(b"<r/>".as_slice())
    );
}

#[test]
fn a_manifest_duplicate_is_not_mistaken_for_a_built_in_when_both_precede_it() {
    // The first `/dup` registers; the second collides with the first,
    // not with any host blob, so the earlier manifest source is named.
    let dir = scratch("collision_ordering");
    write_file(&dir.join("PARAM.SFO"), b"sfo");
    write_file(&dir.join("a.xml"), b"a");
    write_file(&dir.join("b.xml"), b"b");
    let mut host = Lv2Host::new();

    let err = register(
        &dir,
        vec![
            entry("/dup", "a.xml"),
            entry("/dup", "b.xml"),
            entry(BUILT_IN, "PARAM.SFO"),
        ],
        &mut host,
    );
    assert!(
        matches!(&err, ContentRegisterError::DuplicateGuestPath { first_host_path, .. } if *first_host_path == dir.join("a.xml")),
        "{err}"
    );
}
