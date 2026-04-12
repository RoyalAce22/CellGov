//! Dispatch result types returned by `Lv2Host::dispatch`.
//!
//! `Lv2Dispatch` tells the runtime what to do with the syscall: complete
//! it immediately, register a new SPU, or block the caller. Every
//! variant carries plain data -- no closures, no runtime references.

use cellgov_effects::Effect;

/// How the runtime should complete a dispatched syscall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lv2Dispatch {
    /// Immediate completion. The runtime writes `code` into the PPU's
    /// r3, advances PC past the `sc`, and commits any effects.
    Immediate {
        /// Return code for r3 (0 = CELL_OK).
        code: u64,
        /// Effects the host wants the runtime to commit.
        effects: Vec<Effect>,
    },
    /// The host asks the runtime to construct and register SPUs.
    /// One entry per slot in the thread group, in deterministic slot
    /// order. The runtime constructs and registers each.
    RegisterSpu {
        /// Initialization state for each SPU to create, in slot order.
        inits: Vec<SpuInitState>,
        /// Effects to commit alongside the registration.
        effects: Vec<Effect>,
        /// Return code for r3.
        code: u64,
    },
    /// The host wants the caller to block until a condition resolves.
    Block {
        /// Why the caller is blocking.
        reason: Lv2BlockReason,
        /// What the runtime should do when the block resolves.
        pending: PendingResponse,
        /// Effects to commit before blocking.
        effects: Vec<Effect>,
    },
}

/// A stable, deterministic, host-side token identifying a loaded SPU
/// image. Allocated by a monotonic counter, starting at 1 (0 is
/// reserved as "no image"). Not a pointer, not an index into a Vec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpuImageHandle(u32);

impl SpuImageHandle {
    /// Wrap a raw handle value.
    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw handle value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Initialization state for a new SPU execution unit.
///
/// Pure data -- the host constructs it from the image registry and
/// guest memory, the runtime applies it when creating the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuInitState {
    /// Bytes to copy into the SPU's local store.
    pub ls_bytes: Vec<u8>,
    /// Entry point PC within the local store.
    pub entry_pc: u32,
    /// Initial stack pointer (r1).
    pub stack_ptr: u32,
    /// SPU thread arguments (loaded into r3..=r6).
    pub args: [u64; 4],
    /// Unit id of the owning thread group (for join tracking).
    pub group_id: u32,
    /// Slot index within the group.
    pub slot: u32,
}

/// Why the LV2 host is blocking the caller.
///
/// Separate from `cellgov_core::BlockReason` because `cellgov_lv2`
/// does not depend on `cellgov_core`. The runtime maps this to its
/// own `BlockReason` when it consumes the dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2BlockReason {
    /// sys_spu_thread_group_join: waiting for all SPUs in the group
    /// to finish.
    ThreadGroupJoin {
        /// The group being joined.
        group_id: u32,
    },
}

/// What the runtime should do when a blocked PPU is woken.
///
/// Stored in the runtime-owned `SyscallResponseTable`, keyed by the
/// blocked unit's `UnitId`. When the wake condition fires, the
/// runtime reads this, fills r3, and (for join) writes the cause/status
/// out-pointers via `SharedWriteIntent` effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingResponse {
    /// On wake, set r3 = `code`. No out-pointer writes.
    ReturnCode {
        /// Value for r3.
        code: u64,
    },
    /// On wake, set r3 = `code` and write `cause`/`status` to the
    /// guest addresses the caller provided.
    ThreadGroupJoin {
        /// Which group the caller is joining (for wake matching).
        group_id: u32,
        /// Value for r3.
        code: u64,
        /// Guest address to write the exit cause.
        cause_ptr: u32,
        /// Guest address to write the exit status.
        status_ptr: u32,
        /// Exit cause value (filled in at wake time).
        cause: u32,
        /// Exit status value (filled in at wake time).
        status: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spu_image_handle_roundtrip() {
        let h = SpuImageHandle::new(42);
        assert_eq!(h.raw(), 42);
    }

    #[test]
    fn spu_image_handle_zero_reserved() {
        let h = SpuImageHandle::new(0);
        assert_eq!(h.raw(), 0);
    }

    #[test]
    fn spu_image_handle_ordering() {
        assert!(SpuImageHandle::new(1) < SpuImageHandle::new(2));
    }

    #[test]
    fn pending_response_return_code() {
        let p = PendingResponse::ReturnCode { code: 0 };
        assert_eq!(p, PendingResponse::ReturnCode { code: 0 });
    }

    #[test]
    fn pending_response_join_carries_pointers() {
        let p = PendingResponse::ThreadGroupJoin {
            group_id: 1,
            code: 0,
            cause_ptr: 0x1000,
            status_ptr: 0x1004,
            cause: 1,
            status: 0,
        };
        assert!(matches!(p, PendingResponse::ThreadGroupJoin { .. }));
    }

    #[test]
    fn lv2_dispatch_immediate() {
        let d = Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        };
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    }

    #[test]
    fn spu_init_state_fields() {
        let init = SpuInitState {
            ls_bytes: vec![0; 256],
            entry_pc: 0x100,
            stack_ptr: 0x3FFF0,
            args: [1, 2, 3, 4],
            group_id: 1,
            slot: 0,
        };
        assert_eq!(init.entry_pc, 0x100);
        assert_eq!(init.args[0], 1);
    }

    #[test]
    fn lv2_block_reason_join() {
        let r = Lv2BlockReason::ThreadGroupJoin { group_id: 5 };
        assert_eq!(r, Lv2BlockReason::ThreadGroupJoin { group_id: 5 });
    }
}
