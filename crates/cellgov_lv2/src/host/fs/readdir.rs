//! `sys_fs_readdir` host dispatch.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_fs::{
    CELL_FS_DIRENT_SIZE, CELL_FS_MAX_FS_FILE_NAME_LENGTH, CELL_FS_TYPE_DIRECTORY,
    CELL_FS_TYPE_REGULAR,
};

use crate::dispatch::Lv2Dispatch;
use crate::fs_store::{DirEntry, FsError};
use crate::host::{Lv2Host, Lv2Runtime};

use super::ptr::out_ptr_writable;

impl Lv2Host {
    /// `sys_fs_readdir` -- copy the next snapshotted entry into a
    /// 258-byte `CellFsDirent` and write the byte count to
    /// `nread_out_ptr`.
    ///
    /// At EOF the dirent is all zeros and nread = 0; on a real entry,
    /// nread = 258.
    ///
    /// # Errors
    ///
    /// In precedence order:
    /// 1. `nread_out_ptr` NULL / misaligned / unwritable -> CELL_EFAULT.
    /// 2. `dirent_out_ptr` NULL / unwritable for 258 bytes -> CELL_EFAULT.
    /// 3. Unknown directory `fd` -> CELL_EBADF.
    pub(in crate::host) fn dispatch_fs_readdir(
        &mut self,
        fd: u32,
        dirent_out_ptr: u32,
        nread_out_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !out_ptr_writable(rt, nread_out_ptr, 8, 8) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // CellFsDirent's leading field is u8-> 1-byte alignment.
        if !out_ptr_writable(rt, dirent_out_ptr, CELL_FS_DIRENT_SIZE as usize, 1) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }

        let entry = match self.fs_store_mut().read_dir_entry(fd) {
            Ok(e) => e,
            Err(FsError::UnknownDir) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EBADF.into());
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs_readdir.unexpected_fs_error",
                    format_args!(
                        "FsStore::read_dir_entry returned {other:?} for fd={fd:#x}; \
                         contract violated"
                    ),
                );
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };

        let (dirent_bytes, nread) = match entry {
            Some(e) => (build_dirent(&e), CELL_FS_DIRENT_SIZE),
            None => (vec![0u8; CELL_FS_DIRENT_SIZE as usize], 0),
        };

        let tick = rt.current_tick();
        let dirent_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(dirent_out_ptr, CELL_FS_DIRENT_SIZE as u32),
            bytes: WritePayload::from_slice(&dirent_bytes),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        let nread_write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(nread_out_ptr, 8),
            bytes: WritePayload::from_slice(&nread.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![dirent_write, nread_write],
        }
    }
}

/// Build the 258-byte `CellFsDirent` payload for `entry`.
///
/// Names longer than [`CELL_FS_MAX_FS_FILE_NAME_LENGTH`] are truncated
/// at BYTE boundaries (not codepoint boundaries); `d_namlen` reports
/// the truncated length and `d_name` is zero-padded out to 256 bytes.
fn build_dirent(entry: &DirEntry) -> Vec<u8> {
    let mut buf = vec![0u8; CELL_FS_DIRENT_SIZE as usize];
    let d_type = if entry.is_directory {
        CELL_FS_TYPE_DIRECTORY
    } else {
        CELL_FS_TYPE_REGULAR
    };
    buf[0] = d_type;
    let max_name = CELL_FS_MAX_FS_FILE_NAME_LENGTH as usize;
    let name_bytes = entry.name.as_bytes();
    let n = name_bytes.len().min(max_name);
    buf[1] = n as u8;
    buf[2..2 + n].copy_from_slice(&name_bytes[..n]);
    buf
}

#[cfg(test)]
#[path = "tests/readdir_tests.rs"]
mod tests;
