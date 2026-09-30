//! The main-memory-to-local-store transfer path.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

/// The source of a main-memory-to-local-store copy resolves to no
/// region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Unresolved;

/// Copy `size` bytes of committed memory at `ea` into local store at `lsa`.
///
/// The limit register wraps each local-store address.
///
/// # Errors
///
/// Returns [`Unresolved`] when no region backs the source. A refusal
/// leaves local store untouched.
pub(super) fn copy_into_local_store(
    state: &mut crate::state::SpuState,
    memory: &cellgov_mem::GuestMemory,
    ea: u64,
    lsa: u32,
    size: u32,
) -> Result<(), Unresolved> {
    let bytes = ByteRange::new(GuestAddr::new(ea), u64::from(size))
        .and_then(|src| memory.read(src))
        .ok_or(Unresolved)?;
    state.write_ls_wrapped(lsa, bytes);
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
