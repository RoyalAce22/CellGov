//! A write to a fd opened with write flags on a TTY-sink path is
//! refused like any other, and the dropped bytes are named.

use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::fs::{CELL_FS_O_CREAT, CELL_FS_O_WRONLY};

use crate::dispatch::Lv2Dispatch;
use crate::host::fs::common::{extract_fd, fs_open, run, PathRuntime};
use crate::host::Lv2Host;
use crate::request::Lv2Request;

const SITE: &str = "dispatch.fs_write.tty_sink_write_dropped";

fn write_code(host: &mut Lv2Host, rt: &PathRuntime, fd: u32) -> u64 {
    let write = Lv2Request::FsWrite {
        fd,
        buf_ptr: 0x1000,
        size: 4,
        nwrite_ptr: 0x2000,
    };
    match run(host, rt, write) {
        Lv2Dispatch::Immediate { code, .. } => code,
        other => panic!("expected Immediate, got {other:?}"),
    }
}

#[test]
fn a_write_to_the_tty_sink_fd_is_ebadf_and_named() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/output.txt\0");
    let fd = extract_fd(
        run(
            &mut host,
            &rt,
            fs_open(0x10000, 0x20000, CELL_FS_O_WRONLY | CELL_FS_O_CREAT, 0o600),
        ),
        0x20000,
    );
    assert_eq!(write_code(&mut host, &rt, fd), u64::from(errno::CELL_EBADF));
    assert_eq!(host.invariant_break_site_count(SITE), 1);
}

#[test]
fn a_write_to_a_read_only_fd_names_nothing() {
    let mut host = Lv2Host::new();
    let rt = PathRuntime::empty(0x40000).write(0x10000, b"/app_home/PARAM.SFO\0");
    let fd = extract_fd(
        run(&mut host, &rt, fs_open(0x10000, 0x20000, 0, 0)),
        0x20000,
    );
    assert_eq!(write_code(&mut host, &rt, fd), u64::from(errno::CELL_EBADF));
    assert_eq!(host.invariant_break_site_count(SITE), 0);
}
