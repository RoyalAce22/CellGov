//! `HostMountFiles` answers the three reads the LV2 mount table makes.

use std::io::ErrorKind;

use cellgov_lv2::{HostEntryKind, MountFiles};

use super::HostMountFiles;

#[test]
fn a_file_a_directory_and_an_absent_name_answer_their_own_kinds() {
    let dir = cellgov_testkit::scratch::scratch_labeled("mount_files_kind");
    std::fs::write(dir.join("level.xml"), b"<level/>").expect("write");
    std::fs::create_dir(dir.join("Data")).expect("mkdir");

    assert_eq!(
        HostMountFiles.kind(&dir.join("level.xml")),
        Ok(HostEntryKind::File)
    );
    assert_eq!(
        HostMountFiles.kind(&dir.join("Data")),
        Ok(HostEntryKind::Directory)
    );
    assert_eq!(
        HostMountFiles.kind(&dir.join("absent")),
        Err(ErrorKind::NotFound)
    );
}

#[test]
fn a_read_returns_every_byte_of_the_file() {
    let dir = cellgov_testkit::scratch::scratch_labeled("mount_files_read");
    std::fs::write(dir.join("level.xml"), b"<level/>").expect("write");

    assert_eq!(
        HostMountFiles.read(&dir.join("level.xml")),
        Ok(b"<level/>".to_vec())
    );
    assert_eq!(
        HostMountFiles.read(&dir.join("absent")),
        Err(ErrorKind::NotFound)
    );
}

#[test]
fn a_listing_names_each_entry_with_its_kind() {
    let dir = cellgov_testkit::scratch::scratch_labeled("mount_files_list");
    std::fs::write(dir.join("level.xml"), b"<level/>").expect("write");
    std::fs::create_dir(dir.join("Data")).expect("mkdir");

    let mut listed: Vec<(String, HostEntryKind)> = HostMountFiles
        .list(&dir)
        .expect("list")
        .into_iter()
        .map(|e| (e.name.into_string().expect("utf-8 name"), e.kind))
        .collect();
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        listed,
        vec![
            ("Data".to_string(), HostEntryKind::Directory),
            ("level.xml".to_string(), HostEntryKind::File),
        ]
    );
}

// The test runs on Unix only: Windows refuses to create a symlink
// unless developer mode or an administrator token grants the privilege.
#[cfg(unix)]
#[test]
fn a_probe_follows_a_symlink_and_a_listing_does_not() {
    use std::os::unix::fs::symlink;

    let dir = cellgov_testkit::scratch::scratch_labeled("mount_files_symlink");
    std::fs::write(dir.join("level.xml"), b"<level/>").expect("write");
    std::fs::create_dir(dir.join("Data")).expect("mkdir");
    let links = dir.join("links");
    std::fs::create_dir(&links).expect("mkdir");
    symlink(dir.join("level.xml"), links.join("to_file")).expect("symlink");
    symlink(dir.join("Data"), links.join("to_dir")).expect("symlink");
    symlink(dir.join("absent"), links.join("dangling")).expect("symlink");

    assert_eq!(
        HostMountFiles.kind(&links.join("to_file")),
        Ok(HostEntryKind::File)
    );
    assert_eq!(
        HostMountFiles.kind(&links.join("to_dir")),
        Ok(HostEntryKind::Directory)
    );
    assert_eq!(
        HostMountFiles.kind(&links.join("dangling")),
        Err(ErrorKind::NotFound)
    );

    let mut listed: Vec<(String, HostEntryKind)> = HostMountFiles
        .list(&links)
        .expect("list")
        .into_iter()
        .map(|e| (e.name.into_string().expect("utf-8 name"), e.kind))
        .collect();
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        listed,
        vec![
            ("dangling".to_string(), HostEntryKind::Other),
            ("to_dir".to_string(), HostEntryKind::Other),
            ("to_file".to_string(), HostEntryKind::Other),
        ]
    );
}
