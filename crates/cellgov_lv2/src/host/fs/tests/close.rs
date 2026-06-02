use cellgov_effects::Effect;
use cellgov_ps3_abi::cell_errors;

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
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000);
    assert_immediate(
        run(&mut host, &rt, fs_close(0xDEAD_BEEF)),
        cell_errors::CELL_EBADF.code,
        0,
    );
}

#[test]
fn double_close_returns_ebadf_on_second_call() {
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/foo".into(), b"x".to_vec())
        .unwrap();
    let (fd, rt) = open_registered(&mut host, b"/foo");
    assert_immediate(run(&mut host, &rt, fs_close(fd)), 0, 0);
    assert_immediate(
        run(&mut host, &rt, fs_close(fd)),
        cell_errors::CELL_EBADF.code,
        0,
    );
}

#[test]
fn close_param_sfo_fd_returns_ok() {
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
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    // class 0x73 = SYS_FS_FD_OBJECT.
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
    let mut host = Lv2Host::new();
    host.fs_store_mut()
        .register_blob("/hot".into(), b"x".to_vec())
        .unwrap();
    host.fs_store_mut().force_next_fd_for_test(u32::MAX);
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/hot\0");
    assert_immediate(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        cell_errors::CELL_EMFILE.code,
        0,
    );
}
