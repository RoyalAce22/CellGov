//! Guest-path scanning and disambiguation.

use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_fs::CELL_FS_MAX_PATH_LENGTH;

use crate::host::Lv2Runtime;

/// Read a NUL-terminated guest path string from `path_ptr`.
///
/// # Cost
///
/// Happy path: one `read_committed_until` call. Failure path: up to
/// two more `read_committed` probes to disambiguate EFAULT vs EINVAL.
pub(super) fn read_path_bytes(
    rt: &dyn Lv2Runtime,
    path_ptr: u32,
) -> Result<Vec<u8>, cellgov_ps3_abi::cell_errors::Lv2ErrCode> {
    if let Some(prefix) = rt.read_committed_until(path_ptr as u64, CELL_FS_MAX_PATH_LENGTH, 0) {
        return Ok(prefix.to_vec());
    }
    // Disambiguate the None: (a) path_ptr unmapped, (b) full window
    // mapped but no NUL, (c) first byte mapped, scan crossed unmapped.
    if rt.read_committed(path_ptr as u64, 1).is_none() {
        return Err(cell_errors::CELL_EFAULT);
    }
    if rt
        .read_committed(path_ptr as u64, CELL_FS_MAX_PATH_LENGTH)
        .is_some()
    {
        return Err(cell_errors::CELL_EINVAL);
    }
    Err(cell_errors::CELL_EFAULT)
}

#[cfg(test)]
#[path = "tests/path_tests.rs"]
mod tests;
