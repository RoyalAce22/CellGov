use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

use super::common::{
    assert_immediate, extract_read, fs_close, fs_lseek, fs_read, open_registered, run, PathRuntime,
};

#[test]
fn read_full_file_returns_all_bytes_and_full_count() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let (nread, buf) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 5, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(nread, 5);
    assert_eq!(buf.unwrap(), b"hello");
}

#[test]
fn partial_read_advances_offset_and_second_read_returns_remainder() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdef".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let (n1, b1) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n1, 3);
    assert_eq!(b1.unwrap(), b"abc");
    let (n2, b2) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n2, 3);
    assert_eq!(b2.unwrap(), b"def");
}

#[test]
fn read_past_eof_returns_zero_bytes_and_no_buffer_write() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let (n, _) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 100, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n, 3);
    let (n_eof, b_eof) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 100, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n_eof, 0);
    assert!(
        b_eof.is_none(),
        "EOF read must not emit a buffer write effect",
    );
}

#[test]
fn read_with_zero_nbytes_returns_ok_with_only_nread_write() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 0, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n, 0);
    assert!(b.is_none());
    // Invariant: zero-byte read must not advance the offset.
    let (n2, b2) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
        0x30000,
        0x30100,
    );
    assert_eq!(n2, 3);
    assert_eq!(b2.unwrap(), b"abc");
}

#[test]
fn read_unknown_fd_returns_ebadf_with_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000);
    assert_immediate(
        run(&mut host, &rt, fs_read(0xCAFE_BABE, 0x30000, 8, 0x30100)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn read_bad_buffer_pointer_returns_efault_and_does_not_advance_offset() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdef".to_vec())
        .unwrap();
    let (fd, rt_with_path) = open_registered(&mut host, b"/foo");
    let rt = PathRuntime::empty(0x100000)
        .write(0x10000, b"/foo\0")
        .reserve(0x30000, 0x31000);
    let _ = rt_with_path;
    assert_immediate(
        run(&mut host, &rt, fs_read(fd, 0x30100, 3, 0x40000)),
        errno::CELL_EFAULT.code,
        0,
    );
    // Invariant: an EFAULT read must not advance the offset.
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x40010, 6, 0x40000)),
        0x40010,
        0x40000,
    );
    assert_eq!(n, 6);
    assert_eq!(b.unwrap(), b"abcdef");
}

#[test]
fn read_misaligned_nread_pointer_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(
        run(&mut host, &rt, fs_read(fd, 0x30000, 1, 0x30001)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn read_unmapped_nread_pointer_returns_efault() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(
        run(&mut host, &rt, fs_read(fd, 0x30000, 1, 0xFFFF_FF00)),
        errno::CELL_EFAULT.code,
        0,
    );
}

#[test]
fn read_unknown_fd_takes_precedence_over_bad_buffer() {
    // Precedence invariant: an unknown fd surfaces as EBADF
    // before buffer validation, so no buffer-write attempt is
    // emitted on an invalid fd.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x100000).reserve(0x30000, 0x31000);
    assert_immediate(
        run(&mut host, &rt, fs_read(0xDEAD_BEEF, 0x30100, 4, 0x40000)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn read_after_close_returns_ebadf() {
    // Invariant: closed fds are not reusable for reads.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_immediate(
        run(&mut host, &rt, fs_read(fd, 0x30000, 3, 0x30100)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn read_zero_nbytes_with_bad_buf_ptr_returns_ok_with_only_nread_write() {
    // Invariant: nbytes == 0 skips buf_ptr writability check
    // (POSIX-permitted). A future refactor that flips && to ||
    // would silently break this.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abc".to_vec())
        .unwrap();
    let (fd, _) = open_registered(&mut host, b"/foo");
    let rt = PathRuntime::empty(0x100000)
        .write(0x10000, b"/foo\0")
        .reserve(0x30000, 0x31000);
    let (n, _b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30100, 0, 0x40000)),
        0x30100,
        0x40000,
    );
    assert_eq!(n, 0);
}

#[test]
fn lseek_past_eof_then_read_returns_zero_bytes() {
    // Invariant: a cursor past EOF reads 0 bytes (EOF
    // semantics), not EINVAL.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"abcdef".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let dispatch = run(&mut host, &rt, fs_lseek(fd, 100, 2, 0x30000));
    match dispatch {
        Lv2Dispatch::Immediate { code, .. } => assert_eq!(code, 0),
        other => panic!("expected Immediate, got {other:?}"),
    }
    let (n, b) = extract_read(
        run(&mut host, &rt, fs_read(fd, 0x30100, 8, 0x40000)),
        0x30100,
        0x40000,
    );
    assert_eq!(n, 0);
    assert!(b.is_none(), "EOF read must not emit a buffer write");
}
