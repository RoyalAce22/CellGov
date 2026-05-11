//! `dispatch_fs_lseek` tests, including the cross-handler
//! `lseek_after_close` (close + lseek, primary: lseek return).

use cellgov_ps3_abi::cell_errors as errno;

use crate::host::Lv2Host;

use super::common::{
    assert_immediate, extract_pos, extract_read, fs_close, fs_lseek, fs_read, open_registered, run,
    PathRuntime,
};

#[test]
fn lseek_set_to_midfile_then_read_returns_expected_bytes() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdef".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let pos = extract_pos(
        run(&mut host, &rt, fs_lseek(fd, 4, 0 /* SET */, 0x30200)),
        0x30200,
    );
    assert_eq!(pos, 4);
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 10, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n, 2);
    assert_eq!(b.unwrap(), b"ef");
}

#[test]
fn lseek_end_returns_file_size() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello world".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let pos = extract_pos(
        run(&mut host, &rt, fs_lseek(fd, 0, 2 /* END */, 0x30200)),
        0x30200,
    );
    assert_eq!(pos, 11);
}

#[test]
fn lseek_cur_advances_relative_to_current_offset() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdefghij".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    // Read 3 bytes -> offset = 3.
    let _ = run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100));
    // SEEK_CUR + 4 -> offset = 7.
    let pos = extract_pos(
        run(&mut host, &rt, fs_lseek(fd, 4, 1 /* CUR */, 0x30200)),
        0x30200,
    );
    assert_eq!(pos, 7);
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 10, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n, 3);
    assert_eq!(b.unwrap(), b"hij");
}

#[test]
fn lseek_unknown_fd_returns_ebadf() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_lseek(0xCAFE_BABE, 0, 0, 0x30200)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn lseek_bad_whence_returns_einval() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    // whence=3 is not one of {0, 1, 2}.
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, 0, 3, 0x30200)),
        errno::CELL_EINVAL.code,
        0,
    );
}

#[test]
fn lseek_negative_past_zero_returns_einval() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    // SET to -1 lands outside [0, u64::MAX] -> EINVAL.
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, -1, 0 /* SET */, 0x30200)),
        errno::CELL_EINVAL.code,
        0,
    );
}

#[test]
fn lseek_failed_seek_does_not_advance_offset() {
    // Pin: a CELL_EINVAL seek must leave the fd's offset alone
    // so a subsequent read still returns from where it was.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    // Move to offset 1 first.
    let _ = run(&mut host, &rt, fs_lseek(fd, 1, 0, 0x30200));
    // Failed seek (negative past zero).
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, -10, 1 /* CUR */, 0x30200)),
        errno::CELL_EINVAL.code,
        0,
    );
    // Read from where we were (offset 1) -> 'bc'.
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 5, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n, 2);
    assert_eq!(b.unwrap(), b"bc");
}

#[test]
fn lseek_misaligned_pos_out_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    // 8-byte aligned required; 0x30201 is misaligned.
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, 0, 0, 0x30201)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn lseek_unmapped_pos_out_ptr_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, 0, 0, 0xFFFF_FF00)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn lseek_bad_pos_out_ptr_takes_precedence_over_bad_whence_and_fd() {
    // Pin precedence: EFAULT on pos_out_ptr is checked before
    // whence-decode and fd lookup. A probe that gets bad
    // pointer, bad whence, and bad fd all wrong sees only
    // EFAULT.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_lseek(0xDEAD_BEEF, 0, 99, 0x30201)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn lseek_bad_whence_takes_precedence_over_bad_fd() {
    // Pin precedence: EINVAL on whence is checked before fd
    // lookup. A probe with valid pos_out_ptr but bad whence
    // and bad fd sees EINVAL, not EBADF.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_lseek(0xDEAD_BEEF, 0, 99, 0x30200)),
        errno::CELL_EINVAL.code,
        0,
    );
}

#[test]
fn lseek_after_close_returns_ebadf() {
    // Cross-handler test (close + lseek): primary is the lseek
    // return.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdef".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_immediate(
        run(&mut host, &rt, fs_lseek(fd, 0, 0, 0x30000)),
        errno::CELL_EBADF.code,
        0,
    );
}
