//! Integer reads of container header fields, and the conversion of a
//! header-supplied value to a host `usize`.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

// Every u32 header field widens to a host `usize` without loss.
const _: () = assert!(usize::BITS >= u32::BITS);

/// The `N` bytes at `offset`.
///
/// # Panics
///
/// Panics if the `N` bytes leave `data`. Each caller bounds the read
/// first, so a panic here means that a caller missed a bound.
fn bytes<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&data[offset..][..N]);
    out
}

/// Big-endian `u16` at `offset`; panics like [`bytes`].
pub(crate) fn read_be_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes(data, offset))
}

/// Big-endian `u32` at `offset`; panics like [`bytes`].
pub(crate) fn read_be_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes(data, offset))
}

/// Big-endian `u64` at `offset`; panics like [`bytes`].
pub(crate) fn read_be_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes(data, offset))
}

/// Little-endian `u16` at `offset`; panics like [`bytes`].
pub(crate) fn read_le_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes(data, offset))
}

/// Little-endian `u32` at `offset`; panics like [`bytes`].
pub(crate) fn read_le_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes(data, offset))
}

/// A header-supplied `u64` as a host `usize`, or `None` when it does not fit.
///
/// A value that does not fit a `usize` cannot be an offset or a length
/// inside a host buffer. Each caller refuses `None` with the error it
/// gives for an out-of-range field.
pub(crate) fn usize_from_header(value: u64) -> Option<usize> {
    usize::try_from(value).ok()
}

/// A header-supplied `u32` as a host `usize`, lossless by the module's width assert.
pub(crate) const fn usize_from_u32(value: u32) -> usize {
    value as usize
}
