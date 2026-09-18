//! LV2 dispatch for heavy mutexes.
//!
//! `acquire_or_enqueue` is atomic: owner re-lock (a recursion-count
//! bump, or EDEADLK without `recursive`) and contention (park on
//! FIFO waiter list) are distinguished in one call.
//!
//! The kernel entry records the owner and the create-time
//! attributes; the lightweight mutex entry records neither.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::guest_struct::GuestStruct;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::sync_primitives::MutexAttrs;
use cellgov_time::GuestTicks;

impl Lv2Host {
    pub(super) fn dispatch_mutex_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // A null out-pointer is EFAULT before any kernel-object state
        // mutates, so the refused create mints no id.
        if id_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        // Known divergence: the kernel faults on a null attr, but here
        // a null attr_ptr serves default attributes because the testkit
        // relies on the null-attr default create.
        use cellgov_ps3_abi::lv2::sync::mutex_attribute as attr_layout;
        let attrs = if attr_ptr == 0 {
            MutexAttrs::default()
        } else if let Some(attr) =
            GuestStruct::read(rt, attr_ptr as u64, attr_layout::SIZE as usize)
        {
            let protocol = attr.u32_at(attr_layout::PROTOCOL_OFFSET);
            let recursive_raw = attr.u32_at(attr_layout::RECURSIVE_OFFSET);
            let pshared = attr.u32_at(attr_layout::PSHARED_OFFSET);
            let adaptive = attr.u32_at(attr_layout::ADAPTIVE_OFFSET);
            // Each attribute word is an enumeration, so every unknown
            // enumerant is EINVAL -- except adaptive, whose refusal
            // code is unestablished. Validation ladder: protocol,
            // recursive, pshared / ipc_key / flags, then adaptive.
            // The ladder's order is a CellGov choice -- which EINVAL
            // fires first when two words are both bad is
            // unestablished.
            match protocol {
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_FIFO
                | cellgov_ps3_abi::lv2::sync::SYS_SYNC_PRIORITY => {}
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_PRIORITY_INHERIT => {}
                _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            }
            // Only the RECURSIVE enumerant enables re-locking;
            // SYS_SYNC_NOT_RECURSIVE (0x20) is nonzero but means
            // not recursive, and no third value is defined.
            match recursive_raw {
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_RECURSIVE
                | cellgov_ps3_abi::lv2::sync::SYS_SYNC_NOT_RECURSIVE => {}
                _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            }
            match pshared {
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_PROCESS_SHARED => {
                    // Process-shared creates carry an ipc_key and an
                    // attach policy. A valid key starts at 1, so a
                    // zero key is out of range; the attach flag must
                    // be NEWLY_CREATED / NOT_CREATE / NOT_CARE
                    // (1 / 2 / 3). EINVAL for the zero key is a
                    // CellGov choice -- the key range is established,
                    // the code for breaking it is not.
                    let ipc_key = attr.u64_at(attr_layout::IPC_KEY_OFFSET);
                    let flags = attr.u32_at(attr_layout::FLAGS_OFFSET);
                    if ipc_key == 0 || !(1..=3).contains(&flags) {
                        return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
                    }
                    // No mutex ipc-key registry exists yet, so the key
                    // is not published and NOT_CREATE cannot attach; the
                    // entry is created process-local instead of silently
                    // pretending the namespace exists.
                    self.log_invariant_break(
                        "mutex.pshared_attach_unmodeled",
                        format_args!(
                            "sys_mutex_create with pshared ipc_key {ipc_key:#x} \
                             flags {flags}: cross-process attach not modeled; \
                             created process-local"
                        ),
                    );
                }
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_NOT_PROCESS_SHARED => {}
                _ => return Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
            }
            // The adaptive word is the last member of the ladder, and
            // the only one CellGov does not refuse. Neither member
            // reaches the entry, for the reason recorded on
            // SYS_SYNC_ADAPTIVE.
            match adaptive {
                cellgov_ps3_abi::lv2::sync::SYS_SYNC_ADAPTIVE
                | cellgov_ps3_abi::lv2::sync::SYS_SYNC_NOT_ADAPTIVE => {}
                _ => self.log_invariant_break(
                    "mutex.adaptive_out_of_range",
                    format_args!(
                        "sys_mutex_create with adaptive {adaptive:#x}: neither \
                         SYS_SYNC_ADAPTIVE nor SYS_SYNC_NOT_ADAPTIVE; the refusal \
                         code is unestablished, so the mutex is created"
                    ),
                ),
            }
            MutexAttrs {
                priority_policy: protocol,
                recursive: recursive_raw == cellgov_ps3_abi::lv2::sync::SYS_SYNC_RECURSIVE,
                protocol,
            }
        } else {
            // The kernel reads the attribute struct unconditionally;
            // an unreadable attr pointer is a guest fault, not the
            // default-attribute arm.
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let id = self.alloc_id();
        if self.state.mutexes.create_with_id(id, attrs).is_err() {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    /// `sys_mutex_destroy` (101).
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown id.
    /// - `CELL_EBUSY` while the mutex has an owner or a waiter.
    pub(super) fn dispatch_mutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.state.mutexes.lookup(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if entry.owner().is_some() || !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
        }
        self.state.mutexes.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_mutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            self.log_invariant_break(
                "dispatch.mutex_caller_without_thread_record",
                format_args!("mutex 0x{id:08x}: unit {requester:?} has no PPU thread record"),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        match self.state.mutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::MutexAcquireOrEnqueue::Unknown => {
                Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::Acquired
            | crate::sync_primitives::MutexAcquireOrEnqueue::Recursed => Lv2Dispatch::immediate(0),
            // Recursive locking is counted, and a re-lock past the
            // 2^32 - 1 limit is EKRESOURCE.
            crate::sync_primitives::MutexAcquireOrEnqueue::CountSaturated => {
                Lv2Dispatch::immediate(errno::CELL_EKRESOURCE.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::immediate(errno::CELL_EDEADLK.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::Enqueued => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::Mutex { id },
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_mutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            self.log_invariant_break(
                "dispatch.mutex_caller_without_thread_record",
                format_args!("mutex 0x{id:08x}: unit {requester:?} has no PPU thread record"),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        match self.state.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                Lv2Dispatch::immediate(errno::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_mutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            self.log_invariant_break(
                "dispatch.mutex_caller_without_thread_record",
                format_args!("mutex 0x{id:08x}: unit {requester:?} has no PPU thread record"),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        // A recursive hold is counted, so it needs a matching unlock:
        // an owner unlock with holds outstanding consumes one and
        // keeps ownership, and no waiter is granted until the count
        // reaches zero. Unknown-id and non-owner callers fall
        // through to the release path for ESRCH / EPERM.
        if self.state.mutexes.unlock_decrement(id, caller) {
            return Lv2Dispatch::immediate(0);
        }
        match self.state.mutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::MutexRelease::Unknown => {
                Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexRelease::NotOwner => {
                self.obs.mutex_unlock_not_owner_count += 1;
                Lv2Dispatch::immediate(errno::CELL_EPERM.into())
            }
            crate::sync_primitives::MutexRelease::Freed => Lv2Dispatch::immediate(0),
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                match self.resolve_wake_thread(new_owner, "mutex_unlock.Transferred") {
                    Some(unit) => Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![unit],
                        response_updates: vec![],
                        effects: vec![],
                    },
                    None => Lv2Dispatch::immediate(0),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/mutex_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/mutex_adaptive_tests.rs"]
mod adaptive_tests;
