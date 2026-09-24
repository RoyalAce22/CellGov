//! The main-memory-to-local-store transfer path.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

/// Which end of a main-memory-to-local-store copy refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyRefusal {
    /// No region backs the source range.
    Unresolved,
    /// The destination range escapes local store.
    LocalStoreEscapes,
}

/// Copy `size` bytes of committed memory at `ea` into local store at
/// `lsa`, or name the end that refused.
///
/// The source is tested first, so an escaping destination refuses only
/// where the source resolves. Either refusal leaves local store
/// untouched.
pub(super) fn copy_into_local_store(
    ls: &mut [u8],
    memory: &cellgov_mem::GuestMemory,
    ea: u64,
    lsa: u32,
    size: u32,
) -> Result<(), CopyRefusal> {
    let bytes = ByteRange::new(GuestAddr::new(ea), u64::from(size))
        .and_then(|src| memory.read(src))
        .ok_or(CopyRefusal::Unresolved)?;
    let dst_start = lsa as usize;
    let slot = dst_start
        .checked_add(size as usize)
        .and_then(|end| ls.get_mut(dst_start..end))
        .ok_or(CopyRefusal::LocalStoreEscapes)?;
    slot.copy_from_slice(bytes);
    Ok(())
}

/// Records the bytes a transfer copied from main memory into local store.
///
/// Dependency analysis pairs the range against another unit's write to
/// the same bytes, so a zero-byte transfer, which pairs with nothing,
/// records none. A range past the end of the address space records
/// none either.
pub(super) fn shared_read(ea: u64, size: u32, source: UnitId) -> Option<Effect> {
    // [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] Zero is a valid MFC transfer size.
    if size == 0 {
        return None;
    }
    cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ea), u64::from(size))
        .map(|range| Effect::SharedReadIntent { range, source })
}
