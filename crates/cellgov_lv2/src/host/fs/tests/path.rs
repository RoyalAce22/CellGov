use cellgov_ps3_abi::cell_errors as errno;

use crate::host::Lv2Host;

use cellgov_ps3_abi::sys_fs::CELL_FS_MAX_PATH_LENGTH;

use super::common::{assert_immediate, fs_open, run, PathRuntime};

#[test]
fn path_without_null_terminator_returns_einval() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, &vec![b'A'; CELL_FS_MAX_PATH_LENGTH]);

    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_EINVAL.code,
        0,
    );
}

#[test]
fn out_of_range_path_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_open(0xFFFF_FF00, 0x20000, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn path_at_region_end_succeeds() {
    // Regression: pre-region-aware reads EFAULTed here because
    // a fixed 1024-byte window spilled past the buffer.
    let mut host = Lv2Host::new();
    let path_ptr: u32 = 0x40000 - 5;
    let rt = PathRuntime::empty(0x40000).write(path_ptr, b"/foo\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(path_ptr, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn path_crossing_unmapped_returns_efault_not_einval() {
    // Regression: real PS3 page-faults during the kernel scan
    // (-> EFAULT). The historical 1-byte probe at `path_ptr`
    // returned EINVAL ("first byte mapped, no NUL within
    // max_len") -- that is the bug this regression pins.
    let mut host = Lv2Host::new();
    let path_ptr: u32 = 0x30000 - 32;
    let rt = PathRuntime::empty(0x40000)
        .write(path_ptr, &[b'A'; 32])
        .reserve(0x30000, 0x31000);
    assert_immediate(
        run(&mut host, &rt, fs_open(path_ptr, 0x20000, 0, 0)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn high_bit_bytes_in_path_succeed() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\xe6\x97\xa5\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn empty_path_returns_enoent() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn max_length_path_succeeds() {
    // NUL at index CELL_FS_MAX_PATH_LENGTH - 1 is the inclusive
    // boundary the scan window must reach.
    let mut host = Lv2Host::new();
    let mut payload = vec![b'A'; CELL_FS_MAX_PATH_LENGTH - 1];
    payload.push(0);
    let rt = PathRuntime::empty(0x40000).write(0x10000, &payload);
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}

#[test]
fn first_null_terminator_wins() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0/bar\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}
