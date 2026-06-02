//! Guest-path scanning and disambiguation.

use cellgov_ps3_abi::cell_errors;
use cellgov_ps3_abi::sys_fs::CELL_FS_MAX_PATH_LENGTH;

use crate::host::Lv2Runtime;

/// Read a NUL-terminated guest path string from `path_ptr`. The
/// caller threads the resulting `Vec<u8>` into the path-keyed FS
/// surface. Return shape lifts the disambiguation between the three
/// failure modes (`read_committed_until` is too coarse to do this
/// inline -- it returns the same `None` for "scan crosses unmapped"
/// as it does for "no NUL within max_len").
///
/// # Cost
///
/// The happy path is one trait call. The failure path adds up to
/// two more `read_committed` probes -- one at byte 0 to separate
/// "fully unmapped" from the rest, and one at the full window to
/// separate "no NUL within mapped window" from "scan crossed an
/// unmapped boundary."
pub(super) fn read_path_bytes(
    rt: &dyn Lv2Runtime,
    path_ptr: u32,
) -> Result<Vec<u8>, cellgov_ps3_abi::cell_errors::Lv2ErrCode> {
    if let Some(prefix) = rt.read_committed_until(path_ptr as u64, CELL_FS_MAX_PATH_LENGTH, 0) {
        return Ok(prefix.to_vec());
    }
    // The scan returned None. Disambiguate:
    //   (a) `path_ptr` itself unmapped -> EFAULT.
    //   (b) The full CELL_FS_MAX_PATH_LENGTH window is mapped but no NUL -> EINVAL.
    //   (c) The first byte is mapped, NUL not seen before the mapping
    //       runs out -> EFAULT (real PS3 page-faults during the scan).
    if rt.read_committed(path_ptr as u64, 1).is_none() {
        return Err(cell_errors::CELL_EFAULT);
    }
    if rt
        .read_committed(path_ptr as u64, CELL_FS_MAX_PATH_LENGTH)
        .is_some()
    {
        // Whole window mapped; the scan failed because no NUL
        // appeared within CELL_FS_MAX_PATH_LENGTH bytes.
        return Err(cell_errors::CELL_EINVAL);
    }
    // First byte mapped but full window is not -- the scan crossed
    // an unmapped region.
    Err(cell_errors::CELL_EFAULT)
}
