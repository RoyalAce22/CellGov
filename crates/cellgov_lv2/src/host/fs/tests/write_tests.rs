//! `sys_fs_write` dispatch tests: read-only-model null-backend
//! responses pinned per RPCS3 `sys_fs.cpp` `sys_fs_write` precedence.

use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

use crate::host::fs::common::{open_registered, run, PathRuntime};

fn fs_write(fd: u32, buf_ptr: u32, size: u64, nwrite_ptr: u32) -> Lv2Request {
    Lv2Request::FsWrite {
        fd,
        buf_ptr,
        size,
        nwrite_ptr,
    }
}

fn extract_nwrite_zero(effects: &[Effect], expected_addr: u32) {
    assert_eq!(effects.len(), 1, "expected exactly one nwrite=0 effect");
    let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] else {
        panic!("expected SharedWriteIntent, got {:?}", effects[0]);
    };
    assert_eq!(range.start().raw(), u64::from(expected_addr));
    assert_eq!(range.length(), 8);
    assert_eq!(bytes.bytes(), &0u64.to_be_bytes());
}

#[test]
fn null_nwrite_ptr_returns_efault_and_emits_no_effects() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x10000);
    let d = run(&mut host, &rt, fs_write(1, 0x1000, 16, 0));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, u64::from(cell_errors::CELL_EFAULT));
    assert!(
        effects.is_empty(),
        "null nwrite_ptr must emit zero effects (RPCS3 sys_fs_write)"
    );
}

#[test]
fn null_buf_ptr_returns_efault_and_zeros_nwrite() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x10000);
    let d = run(&mut host, &rt, fs_write(1, 0, 16, 0x2000));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, u64::from(cell_errors::CELL_EFAULT));
    extract_nwrite_zero(&effects, 0x2000);
}

#[test]
fn zero_size_with_valid_fd_returns_ok_with_nwrite_zero_and_no_break() {
    // RPCS3 sys_fs_write: nbytes == 0 reaches the OK arm only
    // after the file-existence check passes. Our model never sets
    // file->lock, so the EBUSY sub-arm is dead; a valid fd with
    // size==0 returns CELL_OK + nwrite=0.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let breaks_before = host.observability().invariant_break_count;
    let d = run(&mut host, &rt, fs_write(fd, 0x1000, 0, 0x2000));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(code, 0, "size==0 + valid fd must yield CELL_OK");
    extract_nwrite_zero(&effects, 0x2000);
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        0,
        "zero-byte sys_fs_write to a valid fd is not a violation; no invariant break expected"
    );
}

#[test]
fn write_to_dir_fd_returns_ebadf_not_ok_for_both_zero_and_nonzero_size() {
    // RPCS3 sys_fs_write does a typed downcast:
    //   const auto file = idm::get_unlocked<lv2_fs_object, lv2_file>(fd);
    // A dir fd (lv2_dir, not lv2_file) yields a null and lands at the
    // !file arm -> CELL_EBADF. The CellGov mirror discriminates at the
    // data-structure level: FsStore keeps file fds in `open_fds` and
    // dir fds in a separate `open_dirs` map, and `fstat` only looks up
    // `open_fds`, so a dir fd reads as UnknownFd and arm 3's
    // CELL_EBADF fires.
    let mut host = Lv2Host::new();
    let dir_fd = host
        .fs_store_mut()
        .open_dir(Vec::new())
        .expect("open_dir on empty entries");
    let rt = PathRuntime::empty(0x10000);
    for size in [0u64, 16, 0x1_0000_0000] {
        let d = run(&mut host, &rt, fs_write(dir_fd, 0x1000, size, 0x2000));
        let Lv2Dispatch::Immediate { code, effects } = d else {
            panic!("expected Immediate at size={size:#x}, got {d:?}");
        };
        assert_eq!(
            code,
            u64::from(cell_errors::CELL_EBADF),
            "sys_fs_write(dir_fd, _, {size:#x}, _) must yield CELL_EBADF \
             (RPCS3 sys_fs_write downcast to lv2_file returns null for a dir fd, \
             firing the !file arm before any size check)"
        );
        extract_nwrite_zero(&effects, 0x2000);
    }
}

#[test]
fn zero_size_with_bad_fd_returns_ebadf_not_ok() {
    // RPCS3 sys_fs_write -- ordering matters: fd resolution (the
    // downcast) and the `!file` arm run BEFORE the `!nbytes`
    // short-circuit. A zero-byte write to a bogus fd is CELL_EBADF,
    // not CELL_OK.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x10000);
    // fd=99 is not registered; FsStore::fstat returns Err.
    let d = run(&mut host, &rt, fs_write(99, 0x1000, 0, 0x2000));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(
        code,
        u64::from(cell_errors::CELL_EBADF),
        "bad fd at size==0 must yield CELL_EBADF per RPCS3 sys_fs_write \
         (file-existence check precedes the size==0 short-circuit)"
    );
    extract_nwrite_zero(&effects, 0x2000);
}

#[test]
fn nonzero_size_with_valid_fd_returns_ebadf_zeros_nwrite_and_logs_break() {
    // The fd is registered so the rejection is specifically the
    // access-mode arm (RPCS3 sys_fs_write, second condition), not
    // the file-existence arm (covered separately).
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let breaks_before = host.observability().invariant_break_count;
    let d = run(&mut host, &rt, fs_write(fd, 0x1000, 16, 0x2000));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(
        code,
        u64::from(cell_errors::CELL_EBADF),
        "non-zero write to a read-only-model fd must return CELL_EBADF, not CELL_OK"
    );
    extract_nwrite_zero(&effects, 0x2000);
    assert_eq!(
        host.observability().invariant_break_count - breaks_before,
        1,
        "the null-backend write path must log_invariant_break exactly once"
    );
}

#[test]
fn u32_max_plus_one_size_does_not_truncate_and_returns_ebadf() {
    // Uses a valid fd so the rejection is the access-mode arm
    // specifically; nwrite is 0 regardless of the claimed size.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"hello".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    let d = run(&mut host, &rt, fs_write(fd, 0x1000, 0x1_0000_0000, 0x2000));
    let Lv2Dispatch::Immediate { code, effects } = d else {
        panic!("expected Immediate, got {d:?}");
    };
    assert_eq!(
        code,
        u64::from(cell_errors::CELL_EBADF),
        "oversize sys_fs_write still rejects honestly via CELL_EBADF"
    );
    extract_nwrite_zero(&effects, 0x2000);
}
