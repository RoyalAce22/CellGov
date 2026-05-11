//! Open-flag validation and the TTY-sink path allowlist.
//!
//! The `CELL_FS_O_*` constants live in [`cellgov_ps3_abi::sys_fs`]
//! since they are PS3 ABI values, not CellGov policy. This module
//! holds the validator and the (CellGov-internal) allowlist of
//! paths whose write-flag opens succeed because the dispatcher
//! routes their writes to the host TTY log.

use cellgov_ps3_abi::cell_errors as errno;
use cellgov_ps3_abi::sys_fs::{
    CELL_FS_O_ACCMODE, CELL_FS_O_APPEND, CELL_FS_O_CREAT, CELL_FS_O_RDWR, CELL_FS_O_TRUNC,
    CELL_FS_O_WRONLY,
};

/// Paths whose `sys_fs_open` accepts write flags because the
/// dispatcher routes `sys_fs_write` to the host TTY log regardless
/// of fd.
///
/// # Cross-module contract
///
/// Each path in this list MUST also be registered as a synthetic
/// zero-byte blob in [`crate::host::Lv2Host::new`] so existence
/// checks succeed. The `tty_sink_paths_are_pre_registered`
/// regression test in `super::tests` pins this.
pub(super) const FS_TTY_SINK_PATHS: &[&str] = &["/app_home/output.txt"];

/// Returns `Some(errno)` if the open-flag combination is unsupportable
/// against a read-only blob store, `None` if it is OK to proceed with
/// open-for-read. `path` lets the validator skip flag checks for
/// known write-sink fixtures.
pub(super) fn validate_open_flags(
    flags: u32,
    path: &str,
) -> Option<cellgov_ps3_abi::cell_errors::Lv2Error> {
    if FS_TTY_SINK_PATHS.contains(&path) {
        return None;
    }
    let access = flags & CELL_FS_O_ACCMODE;
    if access == CELL_FS_O_WRONLY || access == CELL_FS_O_RDWR {
        return Some(errno::CELL_EROFS);
    }
    if flags & (CELL_FS_O_CREAT | CELL_FS_O_TRUNC | CELL_FS_O_APPEND) != 0 {
        return Some(errno::CELL_EROFS);
    }
    None
}
