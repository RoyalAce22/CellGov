//! `dispatch_fs_close` tests plus fd-exhaustion and the
//! `close_does_not_decrement_fs_fd_count` invariant.

use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;
use crate::request::Lv2Request;

use super::common::{
    assert_immediate, extract_fd, fs_close, fs_open, open_registered, run, PathRuntime,
};

#[test]
fn close_fsstore_fd_returns_ok_and_removes_from_table() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_eq!(host.fs_store().open_fd_count(), 1);
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_eq!(
        host.fs_store().open_fd_count(),
        0,
        "FsStore close must remove the fd from the open-fd table",
    );
}

#[test]
fn close_truly_unknown_fd_returns_ebadf() {
    // After whitelist retirement, a never-allocated fd is a
    // genuine guest bug. Real PS3 returns EBADF for these;
    // mirror that here so cross-runner compare catches the
    // bug rather than letting it fail-soft to CELL_OK.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_close(0xDEAD_BEEF)),
        errno::CELL_EBADF.code,
        0,
    );
}

#[test]
fn double_close_returns_ebadf_on_second_call() {
    // First close removes the fd from FsStore (spec-correct).
    // Second close hits UnknownFd which is now EBADF: there is
    // no whitelist allocator to coexist with, so a stale fd
    // can never alias a fresh one and the legacy CELL_OK shim
    // is gone.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_immediate(run(&mut host, &rt, fs_close(fd)), errno::CELL_EBADF.code, 0);
}

#[test]
fn close_param_sfo_fd_returns_ok() {
    // Pin: the synthetic PARAM.SFO blob can be opened and
    // closed with the same shape as any FsStore-registered
    // path. fs_fd_count was bumped by the open and stays at 1
    // (real PS3 leaves the kernel-side count untouched on
    // close; the sys_process ps3autotest pins this).
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert_eq!(host.fs_store().open_fd_count(), 1);
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_eq!(host.fs_store().open_fd_count(), 0);
}

#[test]
fn close_does_not_decrement_fs_fd_count() {
    // Real PS3 leaves fs_fd_count untouched across sys_fs_close;
    // the sys_process ps3autotest pins this. Open via the
    // synthetic PARAM.SFO blob (which bumps fs_fd_count), close
    // it, then confirm the count is still 1.
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    // Probe fs_fd_count via sys_process_get_number_of_object
    // (class 0x73 = SYS_FS_FD_OBJECT). 4 bytes at 0x21000.
    let probe = Lv2Request::ProcessGetNumberOfObject {
        class_id: 0x73,
        count_out_ptr: 0x21000,
    };
    let probe_count = |host: &mut Lv2Host| -> u32 {
        let effects = match run(host, &rt, probe) {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                effects
            }
            other => panic!("expected Immediate, got {other:?}"),
        };
        match &effects[0] {
            Effect::SharedWriteIntent { bytes, .. } => {
                u32::from_be_bytes(bytes.bytes().try_into().unwrap())
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        }
    };
    assert_eq!(probe_count(&mut host), 1, "fs_fd_count after open");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_eq!(
        probe_count(&mut host),
        1,
        "fs_fd_count must NOT decrement on close (PS3-pinned behaviour)",
    );
}

#[test]
fn fd_exhaustion_returns_emfile() {
    // Stage the FS-layer allocator at u32::MAX so the next
    // open_fd hits FdExhausted; the dispatch arm must surface
    // that as CELL_EMFILE rather than falling through to ENOENT.
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/hot".into(), b"x".to_vec())
        .unwrap();
    // Burn the allocator down to the cap. open_fd at
    // u32::MAX - 1 succeeds and leaves next_fd = u32::MAX; the
    // following call exhausts.
    host.fs_store_mut().force_next_fd_for_test(u32::MAX);
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/hot\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        errno::CELL_EMFILE.code,
        0,
    );
}
