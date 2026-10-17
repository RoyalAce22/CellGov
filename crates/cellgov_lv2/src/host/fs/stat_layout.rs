//! `CellFsStat` builder and the stat-pointer guard.
//!
//! Wire-format constants (`CELL_FS_STAT_SIZE`, `CELL_FS_BLOCK_SIZE`,
//! the `S_*` mode bits) live in [`cellgov_ps3_abi::lv2::fs`]. This
//! module composes them into the regular-file mode CellGov emits
//! and wraps the write into a `SharedWriteIntent` effect.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::fs::{
    CELL_FS_BLOCK_SIZE, CELL_FS_STAT_BLKSIZE_OFFSET, CELL_FS_STAT_SIZE, CELL_FS_STAT_SIZE_OFFSET,
    CELL_FS_S_IFDIR, CELL_FS_S_IFREG, CELL_FS_S_IRGRP, CELL_FS_S_IROTH, CELL_FS_S_IRUSR,
    CELL_FS_S_IXGRP, CELL_FS_S_IXOTH, CELL_FS_S_IXUSR,
};

use crate::host::Lv2Runtime;

use super::ptr::out_ptr_writable;

/// `CellFsStat::mode` for a regular read-only file: `S_IFREG`
/// plus `r--r--r--` (`IRUSR | IRGRP | IROTH`).
///
/// The console reports the permission bits a file was created with
/// (`0o600` in the `lv2_fs_answers` capture). CellGov's files are
/// never created by the guest, so the read-only bits stand in.
pub(super) const CELL_FS_S_IFREG_R_ONLY_MODE: u32 =
    CELL_FS_S_IFREG | CELL_FS_S_IRUSR | CELL_FS_S_IRGRP | CELL_FS_S_IROTH;

/// `CellFsStat::mode` for a directory: `S_IFDIR` plus `r-xr-xr-x`,
/// the read-only bits a mounted directory stands in with, as
/// [`CELL_FS_S_IFREG_R_ONLY_MODE`] does for a file.
pub(super) const CELL_FS_S_IFDIR_R_ONLY_MODE: u32 = CELL_FS_S_IFDIR
    | CELL_FS_S_IRUSR
    | CELL_FS_S_IXUSR
    | CELL_FS_S_IRGRP
    | CELL_FS_S_IXGRP
    | CELL_FS_S_IROTH
    | CELL_FS_S_IXOTH;

/// `CellFsStat::size` for a directory: one block, as the
/// `lv2_fs_answers` capture reads for a directory of six entries. A
/// directory too big for one block is unprobed.
pub(super) const CELL_FS_DIRECTORY_SIZE: u64 = CELL_FS_BLOCK_SIZE;

/// Whether a `CellFsStat` write at `stat_out_ptr` would land in
/// writable guest memory, with the pointer 4-byte aligned: the packed
/// record puts its 64-bit fields at 4-byte offsets.
pub(super) fn is_stat_ptr_writable(rt: &dyn Lv2Runtime, stat_out_ptr: u32) -> bool {
    out_ptr_writable(rt, stat_out_ptr, CELL_FS_STAT_SIZE as usize, 4)
}

/// Build the 52-byte big-endian `CellFsStat` payload for an entry of
/// `mode` and `size`, and wrap it
/// in a `SharedWriteIntent` at `stat_out_ptr`. atime / mtime /
/// ctime are deterministic zeros (the oracle has no concept of
/// host time); blob content is immutable so a real timestamp
/// would be misleading.
pub(super) fn cell_fs_stat_write(
    mode: u32,
    size: u64,
    stat_out_ptr: u32,
    source: UnitId,
    source_time: cellgov_time::GuestTicks,
) -> Effect {
    let mut blob = [0u8; CELL_FS_STAT_SIZE as usize];
    blob[0..4].copy_from_slice(&mode.to_be_bytes());
    // uid (offset 4) and gid (offset 8) stay zero, as do atime /
    // mtime / ctime at offsets 12 / 20 / 28.
    blob[CELL_FS_STAT_SIZE_OFFSET..CELL_FS_STAT_SIZE_OFFSET + 8]
        .copy_from_slice(&size.to_be_bytes());
    blob[CELL_FS_STAT_BLKSIZE_OFFSET..CELL_FS_STAT_BLKSIZE_OFFSET + 8]
        .copy_from_slice(&CELL_FS_BLOCK_SIZE.to_be_bytes());
    Effect::shared_write(
        ByteRange::contiguous_u32(stat_out_ptr, CELL_FS_STAT_SIZE as u32),
        WritePayload::from_slice(&blob),
        source,
        source_time,
    )
}
