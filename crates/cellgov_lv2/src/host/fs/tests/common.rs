//! Shared test fixtures: synthetic [`Lv2Runtime`] backed by a byte
//! sandbox, request builders, dispatch-shape assertions, and
//! per-test temp-dir helpers for mount tests.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::request::Lv2Request;

use cellgov_ps3_abi::sys_fs::CELL_FS_STAT_SIZE;

/// Test runtime: a flat host-side buffer that mimics guest memory,
/// with optional reserved subranges that mirror unwritable regions.
pub(super) struct PathRuntime {
    pub(super) bytes: Vec<u8>,
    pub(super) reserved: Vec<(u32, u32)>,
    pub(super) tick: GuestTicks,
}

impl PathRuntime {
    pub(super) fn empty(size: usize) -> Self {
        Self {
            bytes: vec![0u8; size],
            reserved: Vec::new(),
            tick: GuestTicks::ZERO,
        }
    }

    pub(super) fn write(mut self, addr: u32, payload: &[u8]) -> Self {
        let start = addr as usize;
        self.bytes[start..start + payload.len()].copy_from_slice(payload);
        self
    }

    pub(super) fn reserve(mut self, start: u32, end: u32) -> Self {
        self.reserved.push((start, end));
        self
    }

    pub(super) fn with_tick(mut self, tick: GuestTicks) -> Self {
        self.tick = tick;
        self
    }
}

impl Lv2Runtime for PathRuntime {
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
        let start = addr as usize;
        let end = start.checked_add(len)?;
        if end > self.bytes.len() {
            return None;
        }
        for &(rs, re) in &self.reserved {
            let rs = rs as usize;
            let re = re as usize;
            if start < re && rs < end {
                return None;
            }
        }
        Some(&self.bytes[start..end])
    }

    fn current_tick(&self) -> GuestTicks {
        self.tick
    }

    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
        let start = addr as usize;
        self.read_committed(addr, 1)?;
        let mut len = 0usize;
        while len < max_len {
            let probe = self.read_committed(addr + len as u64, 1)?;
            if probe[0] == terminator {
                return Some(&self.bytes[start..start + len]);
            }
            len += 1;
        }
        None
    }

    fn writable(&self, addr: u64, len: usize) -> bool {
        let Some(end) = (addr).checked_add(len as u64) else {
            return false;
        };
        if end > self.bytes.len() as u64 {
            return false;
        }
        let start = addr as usize;
        let end = end as usize;
        for &(rs, re) in &self.reserved {
            let rs = rs as usize;
            let re = re as usize;
            if start < re && rs < end {
                return false;
            }
        }
        true
    }
}

pub(super) fn run(host: &mut Lv2Host, rt: &dyn Lv2Runtime, request: Lv2Request) -> Lv2Dispatch {
    host.dispatch(request, UnitId::new(0), rt)
}

pub(super) fn fs_open(path_ptr: u32, fd_out_ptr: u32, flags: u32, mode: u32) -> Lv2Request {
    Lv2Request::FsOpen {
        path_ptr,
        flags,
        fd_out_ptr,
        mode,
    }
}

pub(super) fn fs_read(fd: u32, buf_ptr: u32, nbytes: u64, nread_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsRead {
        fd,
        buf_ptr,
        nbytes,
        nread_out_ptr,
    }
}

pub(super) fn fs_close(fd: u32) -> Lv2Request {
    Lv2Request::FsClose { fd }
}

pub(super) fn fs_fstat(fd: u32, stat_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsFstat { fd, stat_out_ptr }
}

pub(super) fn fs_stat(path_ptr: u32, stat_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsStat {
        path_ptr,
        stat_out_ptr,
    }
}

pub(super) fn fs_lseek(fd: u32, offset: i64, whence: u32, pos_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsLseek {
        fd,
        offset,
        whence,
        pos_out_ptr,
    }
}

pub(super) fn fs_opendir(path_ptr: u32, fd_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsOpendir {
        path_ptr,
        fd_out_ptr,
    }
}

pub(super) fn fs_readdir(fd: u32, dirent_out_ptr: u32, nread_out_ptr: u32) -> Lv2Request {
    Lv2Request::FsReaddir {
        fd,
        dirent_out_ptr,
        nread_out_ptr,
    }
}

pub(super) fn fs_closedir(fd: u32) -> Lv2Request {
    Lv2Request::FsClosedir { fd }
}

pub(super) fn assert_immediate(
    d: Lv2Dispatch,
    expected_code: u32,
    expected_effects: usize,
) -> Vec<Effect> {
    match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code as u32, expected_code, "errno mismatch");
            assert_eq!(effects.len(), expected_effects, "effect count mismatch");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    }
}

/// Pull the u32 fd payload out of a single SharedWriteIntent at
/// `expected_addr`. Panics if the dispatch shape diverges.
pub(super) fn extract_fd(d: Lv2Dispatch, expected_addr: u64) -> u32 {
    let effects = match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "expected CELL_OK");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(effects.len(), 1, "expected exactly one effect");
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), expected_addr);
            assert_eq!(range.length(), 4);
            u32::from_be_bytes(bytes.bytes().try_into().unwrap())
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

/// Open `path` against `host` (which must already have the blob
/// registered) and return the allocated fd. Panics on any
/// dispatch shape divergence.
pub(super) fn open_registered(host: &mut Lv2Host, path: &[u8]) -> (u32, PathRuntime) {
    // Lay out the path at 0x10000 and reserve the rest of the
    // 1 MiB sandbox for buffer / out-pointer use.
    let mut bytes = path.to_vec();
    bytes.push(0);
    let rt = PathRuntime::empty(0x100000).write(0x10000, &bytes);
    let fd = extract_fd(run(host, &rt, fs_open(0x10000, 0x20000, 0, 0)), 0x20000);
    (fd, rt)
}

/// Pull (nread, optional buffer-write bytes) from a successful
/// FsRead dispatch. `expected_nread_addr` and
/// `expected_buf_addr` pin the effect addresses.
pub(super) fn extract_read(
    d: Lv2Dispatch,
    expected_buf_addr: u64,
    expected_nread_addr: u64,
) -> (u64, Option<Vec<u8>>) {
    let effects = match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "expected CELL_OK");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    // 1 effect = nread-only (zero-byte read); 2 effects = buffer + nread.
    match effects.as_slice() {
        [nread_effect] => match nread_effect {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                assert_eq!(range.start().raw(), expected_nread_addr);
                assert_eq!(range.length(), 8);
                let nread = u64::from_be_bytes(bytes.bytes().try_into().unwrap());
                (nread, None)
            }
            other => panic!("expected SharedWriteIntent, got {other:?}"),
        },
        [buf_effect, nread_effect] => {
            let buf_bytes = match buf_effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), expected_buf_addr);
                    bytes.bytes().to_vec()
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            };
            let nread = match nread_effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    assert_eq!(range.start().raw(), expected_nread_addr);
                    assert_eq!(range.length(), 8);
                    u64::from_be_bytes(bytes.bytes().try_into().unwrap())
                }
                other => panic!("expected SharedWriteIntent, got {other:?}"),
            };
            (nread, Some(buf_bytes))
        }
        other => panic!("expected 1 or 2 effects, got {other:?}"),
    }
}

/// Pull the 56-byte CellFsStat blob written by an FsFstat /
/// FsStat success dispatch.
pub(super) fn extract_stat(d: Lv2Dispatch, expected_addr: u64) -> Vec<u8> {
    let effects = match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "expected CELL_OK");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(effects.len(), 1, "expected exactly one effect");
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), expected_addr);
            assert_eq!(range.length(), CELL_FS_STAT_SIZE);
            bytes.bytes().to_vec()
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

/// Read mode / size / blksize out of a CellFsStat blob.
pub(super) fn parse_stat(blob: &[u8]) -> (u32, u64, u64) {
    let mode = u32::from_be_bytes(blob[0..4].try_into().unwrap());
    let size = u64::from_be_bytes(blob[40..48].try_into().unwrap());
    let blksize = u64::from_be_bytes(blob[48..56].try_into().unwrap());
    (mode, size, blksize)
}

/// Pull (dirent_bytes, nread) from a successful FsReaddir
/// dispatch. Both writes always fire (even at EOF, where the
/// dirent buffer is zero-filled and nread = 0).
pub(super) fn extract_readdir(
    d: Lv2Dispatch,
    expected_dirent_addr: u64,
    expected_nread_addr: u64,
) -> (Vec<u8>, u64) {
    let effects = match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "expected CELL_OK");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(effects.len(), 2, "expected dirent + nread effect pair");
    let dirent_bytes = match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), expected_dirent_addr);
            assert_eq!(range.length(), cellgov_ps3_abi::sys_fs::CELL_FS_DIRENT_SIZE,);
            bytes.bytes().to_vec()
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    };
    let nread = match &effects[1] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), expected_nread_addr);
            assert_eq!(range.length(), 8);
            u64::from_be_bytes(bytes.bytes().try_into().unwrap())
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    };
    (dirent_bytes, nread)
}

/// Decode the (d_type, d_namlen, name) triple from a 258-byte
/// CellFsDirent buffer. The name is read out of `d_name` up to
/// `d_namlen` bytes.
pub(super) fn parse_dirent(blob: &[u8]) -> (u8, u8, String) {
    let d_type = blob[0];
    let d_namlen = blob[1];
    let n = d_namlen as usize;
    let name = String::from_utf8(blob[2..2 + n].to_vec()).expect("dirent name must be UTF-8");
    (d_type, d_namlen, name)
}

/// Pull the new u64 position from a successful FsLseek dispatch.
pub(super) fn extract_pos(d: Lv2Dispatch, expected_addr: u64) -> u64 {
    let effects = match d {
        Lv2Dispatch::Immediate { code, effects } => {
            assert_eq!(code, 0, "expected CELL_OK");
            effects
        }
        other => panic!("expected Immediate, got {other:?}"),
    };
    assert_eq!(effects.len(), 1, "expected exactly one effect");
    match &effects[0] {
        Effect::SharedWriteIntent { range, bytes, .. } => {
            assert_eq!(range.start().raw(), expected_addr);
            assert_eq!(range.length(), 8);
            u64::from_be_bytes(bytes.bytes().try_into().unwrap())
        }
        other => panic!("expected SharedWriteIntent, got {other:?}"),
    }
}

/// Per-test scratch dir under the host temp directory. Drops
/// remove the directory tree so parallel tests do not pollute
/// each other and the leftover bytes are not committed.
pub(super) struct TempMountDir {
    pub(super) path: std::path::PathBuf,
}

impl TempMountDir {
    pub(super) fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("cellgov_lv2_mount_{label}_{pid}_{n}"));
        std::fs::create_dir_all(&path).expect("temp mount dir");
        Self { path }
    }

    pub(super) fn write(&self, rel: &str, bytes: &[u8]) {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("temp parent dir");
        }
        std::fs::write(full, bytes).expect("temp file write");
    }
}

impl Drop for TempMountDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
