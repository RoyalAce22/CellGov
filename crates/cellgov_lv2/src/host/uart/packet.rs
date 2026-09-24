//! Big-endian field reads and writes over PS3AV packets.

use crate::host::guest_struct::GuestStruct;

pub(super) fn rd16(p: &[u8], off: usize) -> u16 {
    GuestStruct::new(p).u16_at(off)
}

pub(super) fn rd32(p: &[u8], off: usize) -> u32 {
    GuestStruct::new(p).u32_at(off)
}

/// # Panics
///
/// Panics if `src` does not fit at `off`. Every caller writes a named
/// field of a fixed-size record, so a panic means the offset table and
/// the record length disagree.
pub(super) fn put_bytes(dst: &mut [u8], off: usize, src: &[u8]) {
    dst[off..off + src.len()].copy_from_slice(src);
}

/// A packet's bytes, zero-padded to the length its header declares
/// so field reads past a short send read zeros rather than fault.
pub(super) fn padded(tx: &[u8], off: usize, len: usize) -> Vec<u8> {
    let mut pkt = vec![0u8; len];
    let end = off.saturating_add(len).min(tx.len());
    if off < end {
        pkt[..end - off].copy_from_slice(&tx[off..end]);
    }
    pkt
}
