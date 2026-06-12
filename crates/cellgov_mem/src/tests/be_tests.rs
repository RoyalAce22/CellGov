//! Big-endian slice reads at offsets -- debug-panic vs release-zero on short input.

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
