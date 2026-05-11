//! `dispatch_fs_opendir` tests: out-pointer / path validation,
//! mount-resolve to a host directory, lexicographic enumeration,
//! and the typed errnos for missing / not-a-directory roots.

use cellgov_ps3_abi::cell_errors as errno;

use crate::fs_store::FsMount;
use crate::host::Lv2Host;

use super::common::{assert_immediate, extract_fd, fs_opendir, run, PathRuntime, TempMountDir};

#[test]
fn fd_out_ptr_unmapped_returns_efault_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0xFFFF_FF00)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_misaligned_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20001)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fd_out_ptr_null_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn unmounted_path_returns_enoent_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/dev_hdd0/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn missing_host_directory_returns_enoent() {
    let dir = TempMountDir::new("opendir_missing");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/missing_subdir\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn host_path_is_a_file_returns_enotdir() {
    let dir = TempMountDir::new("opendir_file");
    dir.write("level.xml", b"<level/>");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/level.xml\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_ENOTDIR.code,
        0,
    );
}

#[test]
fn dotdot_traversal_returns_eaccess() {
    let dir = TempMountDir::new("opendir_traversal");
    dir.write("Data/level.xml", b"<level/>");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/../../etc\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_EACCES.code,
        0,
    );
}

#[test]
fn opendir_at_mount_root_allocates_fd() {
    let dir = TempMountDir::new("opendir_root");
    dir.write("a.xml", b"a");
    dir.write("b.xml", b"b");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home\0");
    let fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    assert!(fd >= crate::fs_store::FD_BASE);
    assert_eq!(host.fs_store().open_dir_count(), 1);
    assert_eq!(host.fs_store().open_fd_count(), 0);
}

#[test]
fn opendir_at_subdir_allocates_fd() {
    let dir = TempMountDir::new("opendir_subdir");
    dir.write("Data/first.xml", b"first");
    dir.write("Data/second.xml", b"second");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data\0");
    let fd = extract_fd(run(&mut host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000);
    assert!(fd >= crate::fs_store::FD_BASE);
    assert_eq!(host.fs_store().open_dir_count(), 1);
}

#[test]
fn non_utf8_path_returns_enoent() {
    let mut host = Lv2Host::new();
    // Lone 0xFF byte makes this not valid UTF-8.
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"\xFF/Data\0");
    assert_immediate(
        run(&mut host, &rt, fs_opendir(0x10000, 0x20000)),
        errno::CELL_ENOENT.code,
        0,
    );
}
