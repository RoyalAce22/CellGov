//! The content provider reads a `[content]` entry under the EBOOT
//! directories in probe order. An update supplies the files it
//! carries; the base supplies the rest.

use super::*;
use crate::manifest::ContentEntry;

fn scratch(name: &str) -> cellgov_testkit::scratch::ScratchDir {
    cellgov_testkit::scratch::scratch_labeled(name)
}

fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

fn manifest(entries: &[(&str, &str)]) -> ContentManifest {
    ContentManifest {
        override_base_env: None,
        files: entries
            .iter()
            .map(|(guest, host)| ContentEntry {
                guest_path: (*guest).to_string(),
                host_path: (*host).to_string(),
            })
            .collect(),
    }
}

#[test]
fn an_update_directory_shadows_the_base_for_a_file_both_hold() {
    let update = scratch("bases_shadow_update");
    let base = scratch("bases_shadow_base");
    write_file(&update.join("Data/a.xml"), b"UPDATE");
    write_file(&base.join("Data/a.xml"), b"BASE");
    let mut host = Lv2Host::new();

    let source = register_content_blobs(
        &manifest(&[("/app_home/Data/a.xml", "Data/a.xml")]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(
        source,
        ContentBaseSource::Usrdir {
            paths: vec![update.to_path_buf(), base.to_path_buf()]
        }
    );
    assert_eq!(
        host.fs_store().lookup_blob("/app_home/Data/a.xml"),
        Some(b"UPDATE".as_slice())
    );
}

#[test]
fn a_file_only_the_base_holds_is_read_from_the_base() {
    let update = scratch("bases_fallback_update");
    let base = scratch("bases_fallback_base");
    write_file(&update.join("Data/a.xml"), b"UPDATE");
    write_file(&base.join("Data/b.xml"), b"BASE");
    let mut host = Lv2Host::new();

    register_content_blobs(
        &manifest(&[
            ("/app_home/Data/a.xml", "Data/a.xml"),
            ("/app_home/Data/b.xml", "Data/b.xml"),
        ]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(
        host.fs_store().lookup_blob("/app_home/Data/b.xml"),
        Some(b"BASE".as_slice())
    );
}

#[test]
fn a_file_no_directory_holds_names_the_first_path_and_the_others_probed() {
    let update = scratch("bases_missing_update");
    let base = scratch("bases_missing_base");
    let mut host = Lv2Host::new();

    let err = register_content_blobs(
        &manifest(&[("/app_home/Data/a.xml", "Data/a.xml")]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .expect_err("neither directory holds the file");
    let msg = err.to_string();
    let ContentRegisterError::HostFileRead {
        host_path,
        also_probed,
        source,
        ..
    } = &err
    else {
        panic!("expected HostFileRead, got {err}");
    };
    assert_eq!(*host_path, update.join("Data/a.xml"));
    assert_eq!(*also_probed, vec![base.join("Data/a.xml")]);
    assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
    assert!(
        msg.contains(&format!(
            "also absent under: {}",
            base.join("Data/a.xml").display()
        )),
        "{msg}"
    );
}

#[test]
fn a_single_directory_names_no_others() {
    let base = scratch("bases_single");
    let mut host = Lv2Host::new();

    let err = register_content_blobs(
        &manifest(&[("/p", "absent.xml")]),
        Path::new("/unused"),
        None,
        &[base.to_path_buf()],
        &mut host,
    )
    .expect_err("the file is absent");
    assert!(
        matches!(&err, ContentRegisterError::HostFileRead { also_probed, .. } if also_probed.is_empty()),
        "{err:?}"
    );
    assert!(!err.to_string().contains("also absent"), "{err}");
}

#[test]
fn an_absolute_host_path_is_probed_once() {
    let update = scratch("bases_abs_update");
    let base = scratch("bases_abs_base");
    let elsewhere = scratch("bases_abs_elsewhere");
    let absent = elsewhere.join("abs.xml");
    let mut host = Lv2Host::new();

    let err = register_content_blobs(
        &manifest(&[("/p/abs.xml", &absent.to_string_lossy())]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .expect_err("the absolute path is absent");
    assert!(
        matches!(&err, ContentRegisterError::HostFileRead { host_path, also_probed, .. }
            if *host_path == absent && also_probed.is_empty()),
        "{err:?}"
    );
}

#[test]
fn a_directory_at_the_entry_path_under_the_first_base_stops_the_probe_there() {
    let update = scratch("bases_dir_first_update");
    let base = scratch("bases_dir_first_base");
    // A read of a directory fails on every host, and never as absence.
    std::fs::create_dir_all(update.join("Data/a.xml")).unwrap();
    write_file(&base.join("Data/a.xml"), b"BASE");
    let mut host = Lv2Host::new();

    let err = register_content_blobs(
        &manifest(&[("/app_home/Data/a.xml", "Data/a.xml")]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .expect_err("a read failure that is not absence stops at that base");
    let ContentRegisterError::HostFileRead {
        host_path,
        also_probed,
        source,
        ..
    } = &err
    else {
        panic!("expected HostFileRead, got {err}");
    };
    assert_eq!(*host_path, update.join("Data/a.xml"));
    assert!(also_probed.is_empty(), "{also_probed:?}");
    assert_ne!(source.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(
        host.fs_store().lookup_blob("/app_home/Data/a.xml"),
        None,
        "the base's copy must not be served past a failed read"
    );
}

#[test]
fn a_directory_at_the_entry_path_under_a_later_base_names_the_earlier_probe() {
    let update = scratch("bases_dir_later_update");
    let base = scratch("bases_dir_later_base");
    std::fs::create_dir_all(base.join("Data/a.xml")).unwrap();
    let mut host = Lv2Host::new();

    let err = register_content_blobs(
        &manifest(&[("/app_home/Data/a.xml", "Data/a.xml")]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .expect_err("the base's read fails");
    let ContentRegisterError::HostFileRead {
        host_path,
        also_probed,
        source,
        ..
    } = &err
    else {
        panic!("expected HostFileRead, got {err}");
    };
    assert_eq!(*host_path, base.join("Data/a.xml"));
    assert_eq!(*also_probed, vec![update.join("Data/a.xml")]);
    assert_ne!(source.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn a_path_that_runs_through_a_file_under_one_base_reads_as_absent_there() {
    let update = scratch("bases_notdir_update");
    let base = scratch("bases_notdir_base");
    // `Data` is a regular file under `update`, so nothing can live
    // below it there. Hosts report that as NotFound or NotADirectory
    // by family; both read as "not under this base", as the mount
    // layer's per-root probe already reads them.
    write_file(&update.join("Data"), b"update");
    write_file(&base.join("Data/a.xml"), b"BASE");
    let mut host = Lv2Host::new();

    register_content_blobs(
        &manifest(&[("/app_home/Data/a.xml", "Data/a.xml")]),
        Path::new("/unused"),
        None,
        &[update.to_path_buf(), base.to_path_buf()],
        &mut host,
    )
    .unwrap();
    assert_eq!(
        host.fs_store().lookup_blob("/app_home/Data/a.xml"),
        Some(b"BASE".as_slice())
    );
}
