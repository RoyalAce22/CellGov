//! HLE context trait -- the interface every HLE module operates through.
//!
//! Decouples HLE function implementations from the Runtime struct.
//! Each method corresponds to one capability an HLE handler needs:
//! reading/writing guest memory, returning values, modifying
//! registers, managing the bump allocator, and allocating kernel
//! object IDs.
//!
//! The dispatch layer in `hle.rs` constructs an internal adapter
//! that implements this trait by borrowing from the Runtime.
//! Per-module files (`hle::sys`, `hle::gcm`, and future modules)
//! accept `&mut dyn HleContext` and never see the Runtime.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// The interface HLE module implementations operate through.
///
/// Runtime implements this via an internal adapter struct. Tests can mock
/// it. Future module crates depend only on this trait, never on
/// Runtime directly.
pub trait HleContext {
    /// Read-only view of committed guest memory.
    fn guest_memory(&self) -> &[u8];

    /// Guest memory size in bytes.
    fn guest_memory_len(&self) -> usize {
        self.guest_memory().len()
    }

    /// Write bytes to guest memory at the given guest address.
    fn write_guest(&mut self, addr: u64, bytes: &[u8]);

    /// Set the syscall return value (lands in r3 on resume).
    fn set_return(&mut self, value: u64);

    /// Push a register write (e.g. r13 for TLS base).
    fn set_register(&mut self, reg: u8, value: u64);

    /// Mark the calling unit as Finished (for sys_process_exit).
    fn set_unit_finished(&mut self);

    /// Bump-allocate from the HLE heap. Returns the aligned guest
    /// address, or 0 if the arena is exhausted.
    fn heap_alloc(&mut self, size: u32, align: u32) -> u32;

    /// Allocate a monotonic kernel-object ID.
    fn alloc_id(&mut self) -> u32;
}

/// Adapter that implements [`HleContext`] by borrowing from a
/// [`Runtime`](crate::runtime::Runtime). Constructed per-dispatch in
/// `dispatch_hle`, dropped immediately after the handler returns.
pub(crate) struct RuntimeHleAdapter<'a> {
    pub(crate) memory: &'a mut cellgov_mem::GuestMemory,
    pub(crate) registry: &'a mut crate::registry::UnitRegistry,
    pub(crate) heap_ptr: &'a mut u32,
    pub(crate) next_id: &'a mut u32,
    pub(crate) source: UnitId,
}

impl HleContext for RuntimeHleAdapter<'_> {
    fn guest_memory(&self) -> &[u8] {
        self.memory.as_bytes()
    }

    fn write_guest(&mut self, addr: u64, bytes: &[u8]) {
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), bytes.len() as u64)
        {
            let _ = self.memory.apply_commit(range, bytes);
        }
    }

    fn set_return(&mut self, value: u64) {
        self.registry.set_syscall_return(self.source, value);
    }

    fn set_register(&mut self, reg: u8, value: u64) {
        self.registry.push_register_write(self.source, reg, value);
    }

    fn set_unit_finished(&mut self) {
        self.registry
            .set_status_override(self.source, UnitStatus::Finished);
    }

    fn heap_alloc(&mut self, size: u32, align: u32) -> u32 {
        let mask = align.max(1) - 1;
        let aligned = (*self.heap_ptr + mask) & !mask;
        let new_ptr = aligned + size;
        if (new_ptr as usize) <= self.memory.as_bytes().len() {
            *self.heap_ptr = new_ptr;
            aligned
        } else {
            0
        }
    }

    fn alloc_id(&mut self) -> u32 {
        let id = *self.next_id;
        *self.next_id = self.next_id.wrapping_add(1);
        id
    }
}
