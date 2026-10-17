//! Guest-path scanning and disambiguation.

use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::fs::{
    CELL_FS_MAX_FS_FILE_NAME_LENGTH, CELL_FS_MAX_PATH_LENGTH, GAME_PROCESS_MOUNTED_ROOTS,
};

use crate::host::Lv2Runtime;

/// How many bytes the scan for a path's NUL reads before it calls the
/// path too long: the longest path a capture refuses, and its NUL.
const PATH_SCAN_LIMIT: usize = 4097;

/// Read a NUL-terminated guest path string from `path_ptr`.
///
/// The path below its first component, the device it names, may be
/// [`CELL_FS_MAX_PATH_LENGTH`] bytes long, and no name in it longer than
/// [`CELL_FS_MAX_FS_FILE_NAME_LENGTH`]; either excess is
/// `CELL_ENAMETOOLONG`. A path with no NUL within [`PATH_SCAN_LIMIT`]
/// bytes is too long as well.
///
/// The `lv2_fs_path_limit_mount` capture takes a path of 1033 bytes
/// under `/dev_hdd0` and 1034 under `/dev_flash`, and refuses one byte
/// more under each; the `lv2_fs_path_limit` capture refuses every
/// length it sends from 1040 to 4096; the `lv2_fs_constants` capture refuses a name of
/// 256 bytes and takes one of 255. Where a path that crosses into
/// unmapped memory past the limit faults or is too long is unprobed.
///
/// # Cost
///
/// Happy path: one `read_committed_until` call. Failure path: up to
/// two more `read_committed` probes to disambiguate EFAULT from
/// ENAMETOOLONG.
pub(super) fn read_path_bytes(
    rt: &dyn Lv2Runtime,
    path_ptr: u32,
) -> Result<Vec<u8>, cellgov_ps3_abi::lv2::errno::Lv2ErrCode> {
    let addr = u64::from(path_ptr);
    let Some(path) = rt.read_committed_until(addr, PATH_SCAN_LIMIT, 0) else {
        // Disambiguate the None: (a) path_ptr unmapped, (b) full window
        // mapped but no NUL, (c) first byte mapped, scan crossed unmapped.
        if rt.read_committed(addr, 1).is_none() {
            return Err(errno::CELL_EFAULT);
        }
        if rt.read_committed(addr, PATH_SCAN_LIMIT).is_some() {
            return Err(errno::CELL_ENAMETOOLONG);
        }
        return Err(errno::CELL_EFAULT);
    };
    let below_device = path.len() - device_len(path);
    let long_name = path
        .split(|&b| b == b'/')
        .any(|name| name.len() > usize::from(CELL_FS_MAX_FS_FILE_NAME_LENGTH));
    if below_device > CELL_FS_MAX_PATH_LENGTH || long_name {
        return Err(errno::CELL_ENAMETOOLONG);
    }
    Ok(path.to_vec())
}

/// What `sys_fs_open` and `sys_fs_opendir` answer for a path no mount
/// in CellGov's table holds: `CELL_ENOENT` when its root is one a game
/// process finds mounted, when it has no leading `/`, or for `/`
/// itself; `CELL_ENOTMOUNTED` for any other root. The console answers
/// `/` with `CELL_EISDIR` from `sys_fs_open` and lists it from
/// `sys_fs_opendir` (`lv2_fs_unmounted_paths`); CellGov mounts no `/`.
pub(super) fn unmounted_answer(path: &str) -> cellgov_ps3_abi::lv2::errno::Lv2ErrCode {
    let root = &path.as_bytes()[..device_len(path.as_bytes())];
    // `/` alone names the root itself, which is no device's mount.
    if root.len() <= 1
        || GAME_PROCESS_MOUNTED_ROOTS
            .iter()
            .any(|r| r.as_bytes() == root)
    {
        errno::CELL_ENOENT
    } else {
        errno::CELL_ENOTMOUNTED
    }
}

/// The bytes of `path`'s first component with its leading `/`
/// (`/dev_hdd0` of `/dev_hdd0/tmp`), or 0 for a path with no leading
/// `/`.
fn device_len(path: &[u8]) -> usize {
    match path.split_first() {
        Some((b'/', rest)) => 1 + rest.iter().position(|&b| b == b'/').unwrap_or(rest.len()),
        _ => 0,
    }
}

#[cfg(test)]
#[path = "tests/path_tests.rs"]
mod tests;
