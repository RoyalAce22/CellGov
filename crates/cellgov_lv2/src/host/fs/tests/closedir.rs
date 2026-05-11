//! `dispatch_fs_closedir` tests: success path, double-close,
//! type-mixing across the file/dir fd surfaces.

use cellgov_ps3_abi::cell_errors as errno;

use crate::fs_store::FsMount;
use crate::host::Lv2Host;

use super::common::{
    assert_immediate, extract_fd, fs_close, fs_closedir, fs_open, fs_opendir, run, PathRuntime,
    TempMountDir,
};

fn open_mount(label: &str) -> (Lv2Host, TempMountDir) {
    let dir = TempMountDir::new(label);
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    (host, dir)
}

#[test]
fn closedir_success_drains_dir_count() {
    let (mut host, dir) = open_mount("closedir_ok");
    dir.write("a.xml", b"a");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home\0");
    let fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    assert_eq!(host.fs_store().open_dir_count(), 1);
    assert_immediate(run(&mut host, &rt, fs_closedir(fd)), 0, 0);
    assert_eq!(host.fs_store().open_dir_count(), 0);
}

#[test]
fn closedir_unknown_fd_returns_ebadf() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_closedir(0xDEAD_BEEF)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn closedir_twice_returns_ebadf_on_second_call() {
    let (mut host, dir) = open_mount("closedir_twice");
    dir.write("a.xml", b"a");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home\0");
    let fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    assert_immediate(run(&mut host, &rt, fs_closedir(fd)), 0, 0);
    assert_immediate(
        run(&mut host, &rt, fs_closedir(fd)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn closedir_on_file_fd_returns_ebadf_and_leaves_file_open() {
    // Mount has a file we can open through the regular fs_open
    // path; then attempt to close that file fd via fs_closedir.
    // The two stores are deliberately distinct so the dispatch
    // surfaces EBADF and the file fd stays open for fs_close.
    let (mut host, dir) = open_mount("closedir_type_mixing");
    dir.write("data.xml", b"<x/>");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/data.xml\0");
    let file_fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert_eq!(host.fs_store().open_fd_count(), 1);
    assert_immediate(
        run(&mut host, &rt, fs_closedir(file_fd)),
        errno::CELL_EBADF.code,
        0,
    );
    // File fd still alive: fs_close on it succeeds.
    assert_eq!(host.fs_store().open_fd_count(), 1);
    assert_immediate(run(&mut host, &rt, fs_close(file_fd)), 0, 0);
    assert_eq!(host.fs_store().open_fd_count(), 0);
}

#[test]
fn close_on_dir_fd_returns_ebadf_and_leaves_dir_open() {
    // Symmetric to the above: a dir fd cannot be closed via
    // fs_close, only via fs_closedir.
    let (mut host, dir) = open_mount("close_type_mixing");
    dir.write("a.xml", b"a");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home\0");
    let dir_fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    assert_eq!(host.fs_store().open_dir_count(), 1);
    assert_immediate(
        run(&mut host, &rt, fs_close(dir_fd)),
        errno::CELL_EBADF.code,
        0,
    );
    assert_eq!(host.fs_store().open_dir_count(), 1);
    assert_immediate(run(&mut host, &rt, fs_closedir(dir_fd)), 0, 0);
    assert_eq!(host.fs_store().open_dir_count(), 0);
}
