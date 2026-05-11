//! `CellFsStat` builder and the stat-pointer guard.
//!
//! Wire-format constants (`CELL_FS_STAT_SIZE`, `CELL_FS_BLOCK_SIZE`,
//! the `S_*` mode bits) live in [`cellgov_ps3_abi::sys_fs`]. This
//! module composes them into the regular-file mode CellGov emits
//! and wraps the write into a `SharedWriteIntent` effect.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::sys_fs::{
    CELL_FS_BLOCK_SIZE, CELL_FS_STAT_SIZE, CELL_FS_S_IFREG, CELL_FS_S_IRGRP, CELL_FS_S_IROTH,
    CELL_FS_S_IRUSR,
};

use crate::fs_store::FileStat;
use crate::host::Lv2Runtime;

use super::ptr::out_ptr_writable;

/// `CellFsStat::mode` for a regular read-only file: `S_IFREG`
/// plus `r--r--r--` (`IRUSR | IRGRP | IROTH`). Composed from the
/// individual mode bits in [`cellgov_ps3_abi::sys_fs`] so a future
/// caller that needs a different shape (e.g. directory stat) can
/// compose the same primitives.
pub(super) const CELL_FS_S_IFREG_R_ONLY_MODE: u32 =
    CELL_FS_S_IFREG | CELL_FS_S_IRUSR | CELL_FS_S_IRGRP | CELL_FS_S_IROTH;

/// Whether a 56-byte `CellFsStat` write at `stat_out_ptr` would
/// land in writable guest memory and the pointer satisfies 8-byte
/// alignment (needed for the embedded u64 fields).
pub(super) fn is_stat_ptr_writable(rt: &dyn Lv2Runtime, stat_out_ptr: u32) -> bool {
    out_ptr_writable(rt, stat_out_ptr, CELL_FS_STAT_SIZE as usize, 8)
}

/// Build the 56-byte big-endian `CellFsStat` payload and wrap it
/// in a `SharedWriteIntent` at `stat_out_ptr`. atime / mtime /
/// ctime are deterministic zeros (the oracle has no concept of
/// host time); blob content is immutable so a real timestamp
/// would be misleading.
pub(super) fn cell_fs_stat_write(
    stat: FileStat,
    stat_out_ptr: u32,
    source: UnitId,
    source_time: cellgov_time::GuestTicks,
) -> Effect {
    let mut blob = [0u8; CELL_FS_STAT_SIZE as usize];
    blob[0..4].copy_from_slice(&CELL_FS_S_IFREG_R_ONLY_MODE.to_be_bytes());
    // uid (offset 4), gid (offset 8), pad (offset 12) all stay zero.
    // atime / mtime / ctime at offsets 16 / 24 / 32 stay zero.
    blob[40..48].copy_from_slice(&stat.size.to_be_bytes());
    blob[48..56].copy_from_slice(&CELL_FS_BLOCK_SIZE.to_be_bytes());
    Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(stat_out_ptr as u64), CELL_FS_STAT_SIZE)
            .expect("stat_out_ptr range pre-validated by is_stat_ptr_writable"),
        bytes: WritePayload::from_slice(&blob),
        ordering: PriorityClass::Normal,
        source,
        source_time,
    }
}
