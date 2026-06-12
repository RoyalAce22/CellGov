//! Firmware system-IPC key namespace (`0x8006_0100_0000_xxxx`).
//!
//! cellSysutil's module_start binds its process-shared LV2 objects
//! under ipc keys in this namespace. Decoded from libsysutil.prx's
//! syscall arguments; PS3-firmware ABI facts, data only.

/// ipc_key of the 64 KiB cellSysutil slot-state shared memory,
/// created via a keyed `sys_mmapper_allocate_shared_memory` during
/// libsysutil module_start.
pub const CELLSYSUTIL_SHM_IPC_KEY: u64 = 0x8006_0100_0000_0010;

/// Mask isolating the system-IPC namespace from a full ipc_key.
pub const SYSTEM_IPC_KEY_NAMESPACE_MASK: u64 = 0xffff_ffff_ffff_0000;

/// The system-IPC namespace prefix under
/// [`SYSTEM_IPC_KEY_NAMESPACE_MASK`].
pub const SYSTEM_IPC_KEY_NAMESPACE: u64 = 0x8006_0100_0000_0000;

/// Per-slot stride inside the cellSysutil slot-state shm (two slots).
pub const CELLSYSUTIL_SLOT_STRIDE: u32 = 0x8000;

/// Slot count in the cellSysutil slot-state shm.
pub const CELLSYSUTIL_SLOT_COUNT: u32 = 2;

/// Byte offset of the ring limit field inside a cellSysutil slot.
pub const CELLSYSUTIL_SLOT_LIMIT_OFFSET: u32 = 4;

/// Byte offset of the ring cursor field inside a cellSysutil slot.
pub const CELLSYSUTIL_SLOT_CURSOR_OFFSET: u32 = 16;

/// Byte offset of the record ring inside a cellSysutil slot.
pub const CELLSYSUTIL_SLOT_DATA_OFFSET: u32 = 0x40;

/// ipc_key of slot 0's cond\[0\] (consumer signal / record-finish
/// wait). Per-facility cond keys follow
/// `0x8006_0100_0000_00(3 + facility)(slot)`: facility selects the
/// high nibble (0x30 / 0x40 / 0x50 / 0x60), slot the low nibble.
pub const CELLSYSUTIL_COND0_IPC_KEY_BASE: u64 = 0x8006_0100_0000_0030;

/// ipc_key of slot 0's cond\[1\] (producer-handshake mid-record
/// refill wait). Slot N's key is this base plus N. See
/// [`CELLSYSUTIL_COND0_IPC_KEY_BASE`] for the full key pattern.
pub const CELLSYSUTIL_COND1_IPC_KEY_BASE: u64 = 0x8006_0100_0000_0040;
