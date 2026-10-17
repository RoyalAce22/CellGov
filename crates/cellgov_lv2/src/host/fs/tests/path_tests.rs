//! Guest path-string extraction tests: null-termination, length limits, region-boundary crossings, and non-ASCII bytes.

use cellgov_ps3_abi::lv2::errno;

use crate::host::Lv2Host;

use cellgov_ps3_abi::lv2::fs::CELL_FS_MAX_PATH_LENGTH;

use crate::host::fs::common::{assert_immediate, fs_open, run, PathRuntime};

#[test]
fn path_without_null_terminator_is_too_long() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, &vec![b'/'; 0x2000]);

    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENAMETOOLONG.code,
        0,
    );
}

/// `/dev_hdd0` and then `below` bytes of short names, NUL-terminated.
fn device_path(below: usize) -> Vec<u8> {
    let mut path = b"/dev_hdd0".to_vec();
    while path.len() < 9 + below {
        path.push(b'/');
        let run = (9 + below - path.len()).min(200);
        path.extend(std::iter::repeat_n(b'a', run));
    }
    path.push(0);
    path
}

#[test]
fn the_path_below_its_device_may_run_to_the_limit_and_no_further() {
    let mut host = Lv2Host::new();
    for (below, code) in [
        (CELL_FS_MAX_PATH_LENGTH, errno::CELL_ENOENT.code),
        (CELL_FS_MAX_PATH_LENGTH + 1, errno::CELL_ENAMETOOLONG.code),
    ] {
        let rt = PathRuntime::empty(0x40000).write(0x10000, &device_path(below));
        assert_immediate(
            run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
            code,
            0,
        );
    }
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
fn first_null_terminator_wins() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/foo\0/bar\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_ENOENT.code,
        0,
    );
}
