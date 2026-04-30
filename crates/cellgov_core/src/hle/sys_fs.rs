//! `sys_fs` HLE implementations.
//!
//! PSL1GHT-built titles call `cellFsOpen` / `cellFsRead` / etc.
//! instead of issuing the raw `sys_fs_*` syscalls. Each handler
//! here translates the HLE C-level signature into the matching
//! [`Lv2Request::Fs*`] and routes through
//! [`Runtime::dispatch_lv2_request`], so the title's HLE path
//! sees the same FsStore-backed behaviour as the LV2 syscall path
//! (manifest-registered blobs, EBOOT-adjacent USRDIR
//! auto-discovery, fd allocator, error precedence).
//!
//! The wrappers do not call back into the [`HleContext`] adapter:
//! `dispatch_lv2_request` already sets the syscall return value
//! and applies effects via the runtime commit pipeline, which is
//! the same path the raw-syscall arm uses.

use cellgov_event::UnitId;
use cellgov_lv2::Lv2Request;
use cellgov_ps3_abi::nid::sys_fs as sys_fs_nid;

use crate::runtime::Runtime;

/// Every NID this module claims; sourced from
/// [`cellgov_ps3_abi::nid::sys_fs::OWNED`].
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = sys_fs_nid::OWNED;

/// Dispatch entry point; returns `None` if the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        sys_fs_nid::OPEN => fs_open(runtime, source, args),
        sys_fs_nid::READ => fs_read(runtime, source, args),
        sys_fs_nid::CLOSE => fs_close(runtime, source, args),
        sys_fs_nid::LSEEK => fs_lseek(runtime, source, args),
        sys_fs_nid::FSTAT => fs_fstat(runtime, source, args),
        sys_fs_nid::STAT => fs_stat(runtime, source, args),
        _ => return None,
    }
    Some(())
}

/// `cellFsOpen(const char *path, s32 oflag, s32 *fd, const void *arg, u64 size)`.
///
/// `arg` and `size` carry per-flag extension data (e.g. mode bits
/// when `O_CREAT` is set). The current LV2 dispatch arm rejects
/// `O_CREAT` with ENOENT regardless, so passing `mode = 0` is
/// faithful for the only path that succeeds today (read-only opens
/// of manifest-registered blobs).
fn fs_open(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let path_ptr = args[1] as u32;
    let flags = args[2] as u32;
    let fd_out_ptr = args[3] as u32;
    runtime.dispatch_lv2_request(
        Lv2Request::FsOpen {
            path_ptr,
            flags,
            fd_out_ptr,
            mode: 0,
        },
        source,
    );
}

/// `cellFsRead(s32 fd, void *buf, u64 nbytes, u64 *nread)`.
fn fs_read(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let fd = args[1] as u32;
    let buf_ptr = args[2] as u32;
    let nbytes = args[3];
    let nread_out_ptr = args[4] as u32;
    runtime.dispatch_lv2_request(
        Lv2Request::FsRead {
            fd,
            buf_ptr,
            nbytes,
            nread_out_ptr,
        },
        source,
    );
}

/// `cellFsClose(s32 fd)`.
fn fs_close(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let fd = args[1] as u32;
    runtime.dispatch_lv2_request(Lv2Request::FsClose { fd }, source);
}

/// `cellFsLseek(s32 fd, s64 offset, s32 whence, u64 *pos)`.
fn fs_lseek(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let fd = args[1] as u32;
    let offset = args[2] as i64;
    let whence = args[3] as u32;
    let pos_out_ptr = args[4] as u32;
    runtime.dispatch_lv2_request(
        Lv2Request::FsLseek {
            fd,
            offset,
            whence,
            pos_out_ptr,
        },
        source,
    );
}

/// `cellFsFstat(s32 fd, sysFSStat *st)`.
fn fs_fstat(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let fd = args[1] as u32;
    let stat_out_ptr = args[2] as u32;
    runtime.dispatch_lv2_request(Lv2Request::FsFstat { fd, stat_out_ptr }, source);
}

/// `cellFsStat(const char *path, sysFSStat *st)`.
fn fs_stat(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let path_ptr = args[1] as u32;
    let stat_out_ptr = args[2] as u32;
    runtime.dispatch_lv2_request(
        Lv2Request::FsStat {
            path_ptr,
            stat_out_ptr,
        },
        source,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_ps3_abi::cell_errors as errno;
    use cellgov_time::Budget;

    fn fixture() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x40_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    fn read_syscall_return(rt: &mut Runtime, unit_id: UnitId) -> u64 {
        rt.registry_mut()
            .drain_syscall_return(unit_id)
            .expect("handler must set syscall return")
    }

    fn write_path(rt: &mut Runtime, addr: u32, path: &[u8]) {
        let mut bytes = path.to_vec();
        bytes.push(0);
        let range = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(addr as u64),
            bytes.len() as u64,
        )
        .unwrap();
        rt.memory_mut().apply_commit(range, &bytes).unwrap();
    }

    fn read_u32_be(rt: &Runtime, addr: u32) -> u32 {
        let m = rt.memory().as_bytes();
        let a = addr as usize;
        u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]])
    }

    fn read_u64_be(rt: &Runtime, addr: u32) -> u64 {
        let m = rt.memory().as_bytes();
        let a = addr as usize;
        u64::from_be_bytes([
            m[a],
            m[a + 1],
            m[a + 2],
            m[a + 3],
            m[a + 4],
            m[a + 5],
            m[a + 6],
            m[a + 7],
        ])
    }

    #[test]
    fn cell_fs_open_routes_through_fs_store_for_registered_blob() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"hello".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        // args[0] = HLE syscall slot (unused by the handler).
        // args[1] = path_ptr, args[2] = oflag, args[3] = fd_out_ptr.
        let args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0, "CELL_OK");
        let fd = read_u32_be(&rt, 0x20000);
        assert_ne!(fd, 0, "fs-layer fd must be non-zero");
        assert_eq!(rt.lv2_host().fs_store().open_fd_count(), 1);
    }

    #[test]
    fn cell_fs_open_unknown_path_returns_enoent() {
        let (mut rt, unit_id) = fixture();
        write_path(&mut rt, 0x10000, b"/missing");
        let args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &args);
        assert_eq!(
            read_syscall_return(&mut rt, unit_id) as u32,
            errno::CELL_ENOENT.code,
        );
        // The LV2 dispatcher writes 0 to fd_out_ptr on ENOENT.
        assert_eq!(read_u32_be(&rt, 0x20000), 0);
    }

    #[test]
    fn cell_fs_read_returns_blob_bytes() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"abcdef".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        // Open via the HLE path so the fd is FsStore-allocated.
        let open_args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &open_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        let fd = read_u32_be(&rt, 0x20000);
        // Read 4 bytes into 0x30000; nread out at 0x30100.
        let read_args: [u64; 9] = [0x10000, fd as u64, 0x30000, 4, 0x30100, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::READ, &read_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        assert_eq!(read_u64_be(&rt, 0x30100), 4);
        assert_eq!(&rt.memory().as_bytes()[0x30000..0x30004], b"abcd");
    }

    #[test]
    fn cell_fs_close_releases_fsstore_fd() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"x".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        let open_args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &open_args);
        let _ = read_syscall_return(&mut rt, unit_id);
        let fd = read_u32_be(&rt, 0x20000);
        assert_eq!(rt.lv2_host().fs_store().open_fd_count(), 1);
        let close_args: [u64; 9] = [0x10000, fd as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::CLOSE, &close_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        assert_eq!(
            rt.lv2_host().fs_store().open_fd_count(),
            0,
            "close must remove the fd from FsStore",
        );
    }

    #[test]
    fn cell_fs_lseek_set_writes_new_position() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"abcdef".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        let open_args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &open_args);
        let _ = read_syscall_return(&mut rt, unit_id);
        let fd = read_u32_be(&rt, 0x20000);
        // SEEK_SET to offset 4 -> pos = 4.
        let lseek_args: [u64; 9] = [0x10000, fd as u64, 4, 0 /* SET */, 0x30000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::LSEEK, &lseek_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        assert_eq!(read_u64_be(&rt, 0x30000), 4);
    }

    #[test]
    fn cell_fs_fstat_writes_size_for_open_fd() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"hello world".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        let open_args: [u64; 9] = [0x10000, 0x10000, 0, 0x20000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::OPEN, &open_args);
        let _ = read_syscall_return(&mut rt, unit_id);
        let fd = read_u32_be(&rt, 0x20000);
        // fstat into a 56-byte struct at 0x40000 (8-byte aligned).
        let fstat_args: [u64; 9] = [0x10000, fd as u64, 0x40000, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::FSTAT, &fstat_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        // size lives at offset 40 of the CellFsStat struct.
        assert_eq!(read_u64_be(&rt, 0x40000 + 40), 11);
    }

    #[test]
    fn cell_fs_stat_returns_size_for_registered_path() {
        let (mut rt, unit_id) = fixture();
        rt.lv2_host_mut()
            .fs_store_mut()
            .register_blob("/foo".into(), b"abc".to_vec())
            .unwrap();
        write_path(&mut rt, 0x10000, b"/foo");
        let stat_args: [u64; 9] = [0x10000, 0x10000, 0x40000, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::STAT, &stat_args);
        assert_eq!(read_syscall_return(&mut rt, unit_id), 0);
        assert_eq!(read_u64_be(&rt, 0x40000 + 40), 3);
    }

    #[test]
    fn cell_fs_stat_unknown_path_returns_enoent() {
        let (mut rt, unit_id) = fixture();
        write_path(&mut rt, 0x10000, b"/missing");
        let stat_args: [u64; 9] = [0x10000, 0x10000, 0x40000, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, sys_fs_nid::STAT, &stat_args);
        assert_eq!(
            read_syscall_return(&mut rt, unit_id) as u32,
            errno::CELL_ENOENT.code,
        );
    }
}

#[cfg(test)]
mod canary_tests {
    use super::{dispatch, OWNED_NIDS};
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn canary_runtime() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x40_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    #[test]
    fn owned_nids_all_claimed_by_dispatch() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            // Provide aligned, in-bounds arg values so the LV2
            // dispatch reaches a known errno (ENOENT / EBADF /
            // EFAULT) without panicking on bad pointers.
            let args: [u64; 9] = [0x10000, 0x10100, 0x10200, 0x10300, 0x10400, 0, 0, 0, 0];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result,
                Some(()),
                "sys_fs::dispatch returned None for NID {nid:#010x} listed in OWNED_NIDS"
            );
        }
    }

    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            cellgov_ps3_abi::nid::sys_prx_for_user::MALLOC,
            cellgov_ps3_abi::nid::cell_gcm_sys::INIT_BODY,
            0xDEAD_BEEF,
        ];
        for &nid in probes {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "sys_fs::dispatch claimed NID {nid:#010x} not in OWNED_NIDS"
            );
        }
    }
}
