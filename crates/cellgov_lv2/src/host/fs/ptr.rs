//! Out-pointer validation shared across FS handlers.

use crate::host::Lv2Runtime;

/// True iff `ptr` is non-zero, satisfies `align`-byte alignment, and
/// `len` bytes from `ptr` land in writable guest memory. NULL is
/// rejected even when `len == 0` -- guests that pass a NULL out-pointer
/// have a clear ABI bug, and the runtime trait's `writable(0, ..)` is
/// not guaranteed to reject it.
pub(super) fn out_ptr_writable(rt: &dyn Lv2Runtime, ptr: u32, len: usize, align: u32) -> bool {
    debug_assert!(align.is_power_of_two());
    ptr != 0 && ptr & (align - 1) == 0 && rt.writable(ptr as u64, len)
}
