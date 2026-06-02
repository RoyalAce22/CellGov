use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_fs::{CELL_FS_DIRENT_SIZE, CELL_FS_TYPE_DIRECTORY, CELL_FS_TYPE_REGULAR};

use crate::fs_store::FsMount;
use crate::host::Lv2Host;

use super::common::{
    assert_immediate, extract_fd, extract_readdir, fs_opendir, fs_readdir, parse_dirent, run,
    PathRuntime, TempMountDir,
};

fn open_mount(label: &str) -> (Lv2Host, TempMountDir) {
    let dir = TempMountDir::new(label);
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    (host, dir)
}

fn opendir_path(host: &mut Lv2Host, path: &[u8]) -> u32 {
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &bytes);
    extract_fd(run(host, &rt, fs_opendir(0x10000, 0x20000)), 0x20000)
}

#[test]
fn nread_out_ptr_unmapped_returns_efault() {
    let (mut host, _dir) = open_mount("readdir_efault_nread");
    let fd = opendir_path(&mut host, b"/app_home");
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0xFFFF_FF00)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn dirent_out_ptr_unmapped_returns_efault() {
    let (mut host, _dir) = open_mount("readdir_efault_dirent");
    let fd = opendir_path(&mut host, b"/app_home");
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_readdir(fd, 0xFFFF_FF00, 0x20000)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn unknown_dir_fd_returns_ebadf() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_readdir(0xDEAD_BEEF, 0x20000, 0x21000)),
        cell_errors::CELL_EBADF.code,
        0,
    );
}

#[test]
fn empty_directory_eofs_immediately() {
    let dir = TempMountDir::new("readdir_empty");
    let _ = std::fs::create_dir_all(dir.path.join("sub"));
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let fd = opendir_path(&mut host, b"/app_home/sub");

    let rt = PathRuntime::empty(0x40000);
    let (blob, nread) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(nread, 0, "EOF must report nread = 0");
    assert_eq!(blob.len(), CELL_FS_DIRENT_SIZE as usize);
    assert!(blob.iter().all(|&b| b == 0));
}

#[test]
fn readdir_walks_entries_lexicographically() {
    let dir = TempMountDir::new("readdir_lex");
    dir.write("zzz.xml", b"z");
    dir.write("a.xml", b"a");
    dir.write("middle.xml", b"m");
    dir.write("B.xml", b"B");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let fd = opendir_path(&mut host, b"/app_home");

    // Byte-order: 'B' (0x42) < 'a' (0x61) < 'm' (0x6d) < 'z' (0x7a).
    let expected_order = ["B.xml", "a.xml", "middle.xml", "zzz.xml"];
    for expected in expected_order {
        let rt = PathRuntime::empty(0x40000);
        let (blob, nread) = extract_readdir(
            run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
            0x20000,
            0x21000,
        );
        assert_eq!(nread, CELL_FS_DIRENT_SIZE);
        let (d_type, d_namlen, name) = parse_dirent(&blob);
        assert_eq!(d_type, CELL_FS_TYPE_REGULAR);
        assert_eq!(d_namlen as usize, expected.len());
        assert_eq!(name, expected);
    }

    let rt = PathRuntime::empty(0x40000);
    let (_blob, nread) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(nread, 0);
}

#[test]
fn readdir_classifies_subdirs_as_directory_type() {
    let dir = TempMountDir::new("readdir_dirs");
    dir.write("file.txt", b"hi");
    let _ = std::fs::create_dir_all(dir.path.join("subdir"));
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let fd = opendir_path(&mut host, b"/app_home");

    let rt = PathRuntime::empty(0x40000);
    let (blob, _) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    let (d_type, _, name) = parse_dirent(&blob);
    assert_eq!(name, "file.txt");
    assert_eq!(d_type, CELL_FS_TYPE_REGULAR);

    let rt = PathRuntime::empty(0x40000);
    let (blob, _) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    let (d_type, _, name) = parse_dirent(&blob);
    assert_eq!(name, "subdir");
    assert_eq!(d_type, CELL_FS_TYPE_DIRECTORY);
}

#[test]
fn readdir_eof_then_close_clean() {
    let dir = TempMountDir::new("readdir_close");
    dir.write("only.xml", b"x");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let fd = opendir_path(&mut host, b"/app_home");
    let rt = PathRuntime::empty(0x40000);
    let (_blob, nread) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(nread, CELL_FS_DIRENT_SIZE);
    let rt = PathRuntime::empty(0x40000);
    let (_blob, nread) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(nread, 0);
    // Invariant: snapshot stays drained across repeated post-EOF reads.
    let rt = PathRuntime::empty(0x40000);
    let (_blob, nread) = extract_readdir(
        run(&mut host, &rt, fs_readdir(fd, 0x20000, 0x21000)),
        0x20000,
        0x21000,
    );
    assert_eq!(nread, 0);
}
