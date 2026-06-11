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
mod tests {
    use super::*;

    #[test]
    fn reads_at_offset() {
        let data = [0xAA, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        assert_eq!(read_u16(&data, 1), 0x1234);
        assert_eq!(read_u32(&data, 1), 0x1234_5678);
        assert_eq!(read_u64(&data, 1), 0x1234_5678_9ABC_DEF0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "exceeds slice len")]
    fn short_input_panics_in_debug() {
        let _ = read_u32(&[0u8; 3], 0);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn short_input_reads_zero_in_release() {
        assert_eq!(read_u32(&[0u8; 3], 0), 0);
        assert_eq!(read_u16(&[0x12], 0), 0);
        // Offset arithmetic that would overflow also reads as zero.
        assert_eq!(read_u64(&[0u8; 8], usize::MAX), 0);
    }
}
