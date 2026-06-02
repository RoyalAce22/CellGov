use cellgov_ps3_abi::cell_errors;

use crate::host::Lv2Host;

use cellgov_ps3_abi::sys_fs::{CELL_FS_BLOCK_SIZE, CELL_FS_MAX_PATH_LENGTH};

use super::common::{
    assert_immediate, extract_stat, fs_close, fs_fstat, fs_stat, open_registered, parse_stat, run,
    PathRuntime, TempMountDir,
};
use crate::host::fs::stat_layout::CELL_FS_S_IFREG_R_ONLY_MODE;

#[test]
fn fstat_returns_size_mode_and_blksize() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello world".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let blob = extract_stat(run(&mut host, &rt, fs_fstat(fd, 0x40000)), 0x40000);
    let (mode, size, blksize) = parse_stat(&blob);
    assert_eq!(mode, CELL_FS_S_IFREG_R_ONLY_MODE);
    assert_eq!(size, 11);
    assert_eq!(blksize, CELL_FS_BLOCK_SIZE);
}

#[test]
fn fstat_pads_and_zeros_deterministic_fields() {
    // Determinism invariant: every non-(size/mode/blksize) byte is
    // zero so two stats of the same blob hash bit-identical.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let blob = extract_stat(run(&mut host, &rt, fs_fstat(fd, 0x40000)), 0x40000);
    for byte in &blob[4..40] {
        assert_eq!(*byte, 0, "deterministic-field byte must be zero");
    }
}

#[test]
fn fstat_unknown_fd_returns_ebadf_with_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_fstat(0xCAFE_BABE, 0x40000)),
        cell_errors::CELL_EBADF.code,
        0,
    );
}

#[test]
fn fstat_after_close_returns_ebadf() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_immediate(
        run(&mut host, &rt, fs_fstat(fd, 0x40000)),
        cell_errors::CELL_EBADF.code,
        0,
    );
}

#[test]
fn fstat_misaligned_stat_out_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(
        run(&mut host, &rt, fs_fstat(fd, 0x40001)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn fstat_unmapped_stat_out_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(
        run(&mut host, &rt, fs_fstat(fd, 0xFFFF_FF00)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn stat_known_path_returns_size() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/data.xml".into(), b"abcdef".to_vec())
        .unwrap();
    let rt = PathRuntime::empty(0x100000).write(0x10000, b"/data.xml\0");
    let blob = extract_stat(run(&mut host, &rt, fs_stat(0x10000, 0x40000)), 0x40000);
    let (mode, size, blksize) = parse_stat(&blob);
    assert_eq!(mode, CELL_FS_S_IFREG_R_ONLY_MODE);
    assert_eq!(size, 6);
    assert_eq!(blksize, CELL_FS_BLOCK_SIZE);
}

#[test]
fn stat_unknown_path_returns_enoent_with_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000).write(0x10000, b"/missing\0");
    assert_immediate(
        run(&mut host, &rt, fs_stat(0x10000, 0x40000)),
        cell_errors::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn stat_bad_path_pointer_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_stat(0xFFFF_FF00, 0x40000)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn stat_path_without_null_terminator_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000).write(0x10000, &vec![b'A'; CELL_FS_MAX_PATH_LENGTH]);
    assert_immediate(
        run(&mut host, &rt, fs_stat(0x10000, 0x40000)),
        cell_errors::CELL_EINVAL.code,
        0,
    );
}

#[test]
fn stat_misaligned_stat_out_ptr_returns_efault_before_path_check() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_stat(0xFFFF_FF00, 0x40001)),
        cell_errors::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn stat_path_at_region_end_succeeds() {
    let mut host = Lv2Host::new();
    let path_ptr: u32 = 0x40000 - 5;
    let rt = PathRuntime::empty(0x100000).write(path_ptr, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_stat(path_ptr, 0x60000)),
        cell_errors::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn fs_stat_resolves_via_mount_and_reports_size() {
    let dir = TempMountDir::new("stat_resolves");
    dir.write("Data/level.xml", b"abcdefghij");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/Data/level.xml\0");

    let dispatch = run(&mut host, &rt, fs_stat(0x10000, 0x20000));
    let blob = extract_stat(dispatch, 0x20000);
    // CellFsStat size offset: 40 (mode + uid + gid + pad + 3
    // x 8-byte timestamps).
    let size = u64::from_be_bytes(blob[40..48].try_into().unwrap());
    assert_eq!(size, 10);
}

#[test]
fn fs_stat_mounted_missing_returns_enoent() {
    let dir = TempMountDir::new("stat_missing");
    let mut host = Lv2Host::new();
    host.fs_mounts_mut()
        .add(crate::fs_store::FsMount::new("/app_home", dir.path.clone()).expect("valid mount"))
        .expect("registration");
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/nope.bin\0");
    assert_immediate(
        run(&mut host, &rt, fs_stat(0x10000, 0x20000)),
        cell_errors::CELL_ENOENT.code,
        0,
    );
}
