//! The form of the ids the kernel mints for process objects.
//!
//! An id is `class << 24 | generation << 16 | slot << 8`:
//!
//! - `class` names the object's kind. For the sync objects it is the
//!   class `sys_process_get_number_of_object` counts them under
//!   ([`super::process`]).
//! - `slot` is an entry of one table every class shares. A create takes
//!   the lowest free entry.
//! - `generation` counts the entry's earlier holders. Freeing an object
//!   moves its entry to the generation after the freed id's, so the
//!   entry's next create does not repeat the freed id. The byte wraps
//!   after 256 generations.
//!
//! The captures show each part:
//!
//! - `lv2_process_queries` reads a mutex, a semaphore, an event flag
//!   and an event queue in four consecutive entries.
//! - `lv2_mutex_cond_rwlock_rows` reads three lwmutexes in consecutive
//!   entries. After all three are destroyed, an rwlock takes the first
//!   entry at generation 1.
//! - `lv2_mmapper_allocate` frees each shared-memory segment before the
//!   next create. One entry serves all five, at generations 1 to 5,
//!   across two classes.
//! - `lv2_usbd_config_hid` reads an event queue and a config handle in
//!   the first two entries freed usbd handles left, each at generation 1.
//!
//! Memory containers and SPU thread groups take no entry. Each has
//! its own form.

/// Bits below the class byte.
pub const CLASS_SHIFT: u32 = 24;

/// Bits below the generation byte.
pub const GENERATION_SHIFT: u32 = 16;

/// Bits below the slot byte. The byte under it is zero in every
/// captured id.
pub const SLOT_SHIFT: u32 = 8;

/// Entries the slot byte names.
pub const SLOT_COUNT: u32 = 1 << 8;

/// Generations the generation byte names.
pub const GENERATION_COUNT: u32 = 1 << 8;

/// The most ids of the slot-table classes one process may hold at
/// once; a create past it is refused with `CELL_EAGAIN`.
pub const MAX_LIVE_IDS: u32 = 8192;

/// The id of `class` in `slot` at `generation`.
#[inline]
pub const fn compose(class: u32, generation: u32, slot: u32) -> u32 {
    (class << CLASS_SHIFT)
        | ((generation % GENERATION_COUNT) << GENERATION_SHIFT)
        | ((slot % SLOT_COUNT) << SLOT_SHIFT)
}

/// The class byte of `id`.
#[inline]
pub const fn class_of(id: u32) -> u32 {
    id >> CLASS_SHIFT
}

/// The generation byte of `id`.
#[inline]
pub const fn generation_of(id: u32) -> u32 {
    (id >> GENERATION_SHIFT) & (GENERATION_COUNT - 1)
}

/// The slot byte of `id`.
#[inline]
pub const fn slot_of(id: u32) -> u32 {
    (id >> SLOT_SHIFT) & (SLOT_COUNT - 1)
}

/// Shared memory `sys_mmapper_allocate_shared_memory` (332) mints, as
/// the `lv2_mmapper_map` and `lv2_mmapper_allocate` captures read.
pub const SHARED_MEMORY_CLASS: u32 = 0x08;

/// Shared memory `sys_mmapper_allocate_shared_memory_ext` (339) mints,
/// as the `lv2_mmapper_allocate` capture reads.
pub const SHARED_MEMORY_EXT_CLASS: u32 = 0x20;

/// USB driver handles `sys_usbd_initialize` (530) mints, as the
/// `lv2_usbd_config_hid` capture reads.
pub const USBD_HANDLE_CLASS: u32 = 0x30;

/// Handles `sys_config_open` (516) mints, as the
/// `lv2_config_listener_shapes` capture reads.
pub const CONFIG_HANDLE_CLASS: u32 = 0x41;

/// Listeners `sys_config_add_service_listener` (519) mints, as the
/// `lv2_config_listener_shapes` capture reads.
pub const CONFIG_LISTENER_CLASS: u32 = 0x42;

/// Services `sys_config_register_service` (521) mints. No capture
/// reads one; this class is CellGov's choice.
pub const CONFIG_SERVICE_CLASS: u32 = 0x43;

/// Memory containers `sys_memory_container_create` mints through
/// syscall 341: this base plus a serial. The `lv2_memory_free_container`
/// capture reads 0x3F10000E and, after it is destroyed, 0x3F10000F.
pub const MEMORY_CONTAINER_BASE: u32 = 0x3F10_0000;

/// Memory containers syscall 324 mints: this base plus a serial from
/// the same counter as [`MEMORY_CONTAINER_BASE`]'s. The same capture
/// reads 0x3F00000A.
pub const MEMORY_CONTAINER_324_BASE: u32 = 0x3F00_0000;

/// Bits the container serial occupies under either base.
pub const MEMORY_CONTAINER_SERIAL_MASK: u32 = 0x000F_FFFF;

#[cfg(test)]
#[path = "tests/kernel_id_tests.rs"]
mod tests;
