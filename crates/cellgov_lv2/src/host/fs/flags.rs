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
/// of fd. PSL1GHT-test fixtures fopen these with `"w"` which decodes
/// to `O_WRONLY | O_CREAT | O_TRUNC`; surfacing EROFS would block
/// the test from emitting any output.
///
/// # Cross-module contract
///
/// Each path in this list MUST also be registered as a synthetic
/// zero-byte blob in [`crate::host::Lv2Host::new`] so existence
/// checks succeed. The two sites are kept in lockstep by the
/// `tty_sink_paths_are_pre_registered` regression test in
/// `super::tests`; if a third path is added, register it in both
/// places AND extend the test, or factor a single registration
/// helper.
//
// FUTURE: collapse FS_TTY_SINK_PATHS and the synthetic-blob
// registrations in `Lv2Host::new` into a single
// `Lv2Host::register_tty_sink(path)` method that updates both. The
// constant becomes either empty or a host-side BTreeSet looked up
// via a method, the validator switches from a static `contains` to
// `host.is_tty_sink(path)`, and the cross-site contract dissolves
// because there is only one site. Worth doing the next time a
// third TTY-sink path is needed; not before, since the pinning
// test catches the current two-site drift cheaply.
pub(super) const FS_TTY_SINK_PATHS: &[&str] = &["/app_home/output.txt"];

/// Returns `Some(errno)` if the open-flag combination is unsupportable
/// against a read-only blob store, `None` if it is OK to proceed with
/// open-for-read. `path` lets the validator skip flag checks for
/// known write-sink fixtures.
///
/// `O_EXCL` without `O_CREAT` is meaningless on real PS3 (it never
/// fires) and is not a write attempt; pass through silently.
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
