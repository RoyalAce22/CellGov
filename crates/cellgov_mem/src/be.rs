//! Big-endian integer reads from byte slices.
//!
//! Shared by the ELF / PRX / firmware parsers that walk `&[u8]` file
//! images. Callers bounds-check the containing structure once; an
//! out-of-range offset is a caller bug caught by a debug assertion.
//! Release builds read the missing bytes as zero instead of panicking.

/// Fetches `N` bytes at `offset`, zero-filling on out-of-range reads.
#[inline]
fn read_array<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
    let mut bytes = [0u8; N];
    match offset.checked_add(N).and_then(|end| data.get(offset..end)) {
        Some(src) => bytes.copy_from_slice(src),
        None => debug_assert!(
            false,
            "BE read of {N} bytes at offset {offset} exceeds slice len {}",
            data.len()
        ),
    }
    bytes
}

/// Reads a big-endian `u16` at `offset`.
///
/// # Panics
///
/// Panics in debug builds if `data` is shorter than `offset + 2`;
/// release builds read the missing bytes as zero.
#[inline]
#[must_use]
pub fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(read_array(data, offset))
}

/// Reads a big-endian `u32` at `offset`.
///
/// # Panics
///
/// Panics in debug builds if `data` is shorter than `offset + 4`;
/// release builds read the missing bytes as zero.
#[inline]
#[must_use]
pub fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(read_array(data, offset))
}

/// Reads a big-endian `u64` at `offset`.
///
/// # Panics
///
/// Panics in debug builds if `data` is shorter than `offset + 8`;
/// release builds read the missing bytes as zero.
#[inline]
#[must_use]
pub fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(read_array(data, offset))
}

#[cfg(test)]
#[path = "tests/be_tests.rs"]
mod tests;
