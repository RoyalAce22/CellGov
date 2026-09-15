//! The mount table reads its roots through the file source installed
//! on it, and nothing else.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::fs::{CELL_FS_TYPE_DIRECTORY, CELL_FS_TYPE_REGULAR};

use crate::fs_store::{FsMount, HostDirEntry, HostEntryKind, MountFiles};
use crate::host::fs::common::{
    assert_immediate, extract_fd, extract_read, extract_readdir, fs_open, fs_opendir, fs_read,
    fs_readdir, parse_dirent, run, PathRuntime,
};
use crate::host::Lv2Host;

const UNREADABLE: &str = "dispatch.fs.mount_candidate_unreadable";

fn mounted(files: Option<Rc<dyn MountFiles>>) -> Lv2Host {
    let mut host = Lv2Host::new();
    let mounts = host.fs_mounts_mut();
    if let Some(files) = files {
        mounts.set_files(files);
    }
    mounts
        .add(FsMount::new("/app_home", PathBuf::from("root")).expect("valid mount"))
        .expect("registration");
    host
}

/// One file, `root/level.xml`; `reads` counts the reads of it.
#[derive(Debug, Default)]
struct OneFile {
    reads: Cell<u32>,
}

impl MountFiles for OneFile {
    fn kind(&self, path: &Path) -> Result<HostEntryKind, ErrorKind> {
        if path == Path::new("root").join("level.xml") {
            Ok(HostEntryKind::File)
        } else {
            Err(ErrorKind::NotFound)
        }
    }

    fn read(&self, _path: &Path) -> Result<Vec<u8>, ErrorKind> {
        self.reads.set(self.reads.get() + 1);
        Ok(b"<level/>".to_vec())
    }

    fn list(&self, _path: &Path) -> Result<Vec<HostDirEntry>, ErrorKind> {
        Err(ErrorKind::NotFound)
    }
}

#[test]
fn a_mount_with_no_file_source_answers_eio_and_names_the_break() {
    let mut host = mounted(None);
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/level.xml\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_EIO.code,
        0,
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
    assert!(!host.fs_store().has_path("/app_home/level.xml"));
}

#[test]
fn a_directory_under_a_mount_with_no_file_source_answers_eio() {
    let mut host = mounted(None);
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_EIO.code,
        0,
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
}

#[test]
fn an_opened_file_is_read_through_the_installed_source_once() {
    let files = Rc::new(OneFile::default());
    let mut host = mounted(Some(files.clone()));
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/level.xml\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert_eq!(files.reads.get(), 1);

    let rt = PathRuntime::empty(0x40000);
    let (_, bytes) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x20000, 64, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(bytes.unwrap_or_default(), b"<level/>");
}

/// A host that answers each path from a table; a path with no entry
/// answers `NotFound`.
#[derive(Debug, Default)]
struct Scripted {
    kinds: BTreeMap<PathBuf, Result<HostEntryKind, ErrorKind>>,
    reads: BTreeMap<PathBuf, Result<Vec<u8>, ErrorKind>>,
    lists: BTreeMap<PathBuf, Result<Vec<HostDirEntry>, ErrorKind>>,
}

fn at(parts: &[&str]) -> PathBuf {
    parts.iter().copied().collect()
}

impl Scripted {
    fn with_kind(mut self, parts: &[&str], answer: Result<HostEntryKind, ErrorKind>) -> Self {
        self.kinds.insert(at(parts), answer);
        self
    }

    fn with_read(mut self, parts: &[&str], answer: Result<&[u8], ErrorKind>) -> Self {
        self.reads.insert(at(parts), answer.map(<[u8]>::to_vec));
        self
    }

    fn with_list(mut self, parts: &[&str], answer: Result<Vec<HostDirEntry>, ErrorKind>) -> Self {
        self.lists.insert(at(parts), answer);
        self
    }
}

impl MountFiles for Scripted {
    fn kind(&self, path: &Path) -> Result<HostEntryKind, ErrorKind> {
        self.kinds
            .get(path)
            .cloned()
            .unwrap_or(Err(ErrorKind::NotFound))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, ErrorKind> {
        self.reads
            .get(path)
            .cloned()
            .unwrap_or(Err(ErrorKind::NotFound))
    }

    fn list(&self, path: &Path) -> Result<Vec<HostDirEntry>, ErrorKind> {
        self.lists
            .get(path)
            .cloned()
            .unwrap_or(Err(ErrorKind::NotFound))
    }
}

/// `/app_home` served from the roots `a`, `b`, `c` in that order.
fn layered(files: Scripted) -> Lv2Host {
    let mut host = Lv2Host::new();
    let mounts = host.fs_mounts_mut();
    mounts.set_files(Rc::new(files));
    mounts
        .add(
            FsMount::with_roots("/app_home", vec![at(&["a"]), at(&["b"]), at(&["c"])])
                .expect("valid mount"),
        )
        .expect("registration");
    host
}

fn open_code(host: &mut Lv2Host, path: &[u8]) -> u32 {
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &bytes);
    match run(host, &rt, fs_open(0x10000, 0x20000, 0, 0)) {
        crate::dispatch::Lv2Dispatch::Immediate { code, .. } => code as u32,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn opendir_code(host: &mut Lv2Host, path: &[u8]) -> u32 {
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &bytes);
    match run(host, &rt, fs_opendir(0x10000, 0x20000)) {
        crate::dispatch::Lv2Dispatch::Immediate { code, .. } => code as u32,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

fn entry(name: impl Into<OsString>, kind: HostEntryKind) -> HostDirEntry {
    HostDirEntry {
        name: name.into(),
        kind,
    }
}

#[test]
fn a_root_the_host_will_not_describe_answers_eacces_and_hides_later_roots() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "level.xml"], Err(ErrorKind::PermissionDenied))
            .with_kind(&["b", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["b", "level.xml"], Ok(b"base")),
    );
    assert_eq!(
        open_code(&mut host, b"/app_home/level.xml"),
        errno::CELL_EACCES.code
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
    assert!(!host.fs_store().has_path("/app_home/level.xml"));
}

#[test]
fn a_root_that_fails_for_any_other_reason_answers_eio() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "level.xml"], Err(ErrorKind::Other))
            .with_kind(&["b", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["b", "level.xml"], Ok(b"base")),
    );
    assert_eq!(
        open_code(&mut host, b"/app_home/level.xml"),
        errno::CELL_EIO.code
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
}

#[test]
fn not_a_directory_and_invalid_filename_read_as_nothing_under_that_root() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "level.xml"], Err(ErrorKind::NotADirectory))
            .with_kind(&["b", "level.xml"], Err(ErrorKind::InvalidFilename))
            .with_kind(&["c", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["c", "level.xml"], Ok(b"third")),
    );
    assert_eq!(open_code(&mut host, b"/app_home/level.xml"), 0);
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 0);
    assert!(host.fs_store().has_path("/app_home/level.xml"));
}

#[test]
fn a_read_the_host_refuses_after_the_probe_is_a_named_refusal() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["a", "level.xml"], Err(ErrorKind::PermissionDenied))
            .with_kind(&["b", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["b", "level.xml"], Ok(b"base")),
    );
    assert_eq!(
        open_code(&mut host, b"/app_home/level.xml"),
        errno::CELL_EACCES.code
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
    assert!(!host.fs_store().has_path("/app_home/level.xml"));
}

#[test]
fn a_special_file_at_the_earliest_root_shadows_later_roots() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "Data"], Ok(HostEntryKind::Other))
            .with_kind(&["b", "Data"], Ok(HostEntryKind::Directory))
            .with_list(&["b", "Data"], Ok(vec![]))
            .with_kind(&["a", "level.xml"], Ok(HostEntryKind::Other))
            .with_kind(&["b", "level.xml"], Ok(HostEntryKind::File))
            .with_read(&["b", "level.xml"], Ok(b"base")),
    );
    assert_eq!(
        opendir_code(&mut host, b"/app_home/Data"),
        errno::CELL_ENOTDIR.code
    );
    assert_eq!(
        open_code(&mut host, b"/app_home/level.xml"),
        errno::CELL_ENOENT.code
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 0);
}

#[test]
fn a_listing_the_host_refuses_under_a_later_root_fails_the_whole_listing() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "Data"], Ok(HostEntryKind::Directory))
            .with_list(
                &["a", "Data"],
                Ok(vec![entry("level.xml", HostEntryKind::File)]),
            )
            .with_kind(&["b", "Data"], Ok(HostEntryKind::Directory))
            .with_list(&["b", "Data"], Err(ErrorKind::PermissionDenied)),
    );
    assert_eq!(
        opendir_code(&mut host, b"/app_home/Data"),
        errno::CELL_EACCES.code
    );
    assert_eq!(host.invariant_break_site_count(UNREADABLE), 1);
}

/// A name with no UTF-8 spelling on this host family.
#[cfg(unix)]
fn non_utf8_name() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![b'x', 0xff])
}

/// A name with no UTF-8 spelling on this host family: an unpaired
/// surrogate.
#[cfg(windows)]
fn non_utf8_name() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[u16::from(b'x'), 0xd800])
}

#[cfg(any(unix, windows))]
#[test]
fn a_listing_drops_special_entries_and_names_with_no_utf8_spelling() {
    let mut host = layered(
        Scripted::default()
            .with_kind(&["a", "Data"], Ok(HostEntryKind::Directory))
            .with_list(
                &["a", "Data"],
                Ok(vec![
                    entry("sub", HostEntryKind::Directory),
                    entry("link", HostEntryKind::Other),
                    entry(non_utf8_name(), HostEntryKind::File),
                    entry("keep.xml", HostEntryKind::File),
                ]),
            ),
    );
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    let fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    let mut listed = Vec::new();
    loop {
        let rt = PathRuntime::empty(0x40000);
        let (blob, nread) = extract_readdir(
            run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
            0x20000,
            0x21000,
        );
        if nread == 0 {
            break;
        }
        let (d_type, _, name) = parse_dirent(&blob);
        listed.push((name, d_type));
    }
    assert_eq!(
        listed,
        vec![
            ("keep.xml".to_string(), CELL_FS_TYPE_REGULAR),
            ("sub".to_string(), CELL_FS_TYPE_DIRECTORY),
        ]
    );
}
