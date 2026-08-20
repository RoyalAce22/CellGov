//! LV2 dispatch for heavy mutexes.
//!
//! `acquire_or_enqueue` is atomic: owner re-lock (a recursion-count
//! bump, or EDEADLK without `recursive`) and contention (park on
//! FIFO waiter list) are distinguished in one call.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
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
        // mutates (RPCS3 sys_mutex.cpp sys_mutex_create rejects a
        // null mutex_id ahead of attribute validation and creation).
        if id_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // `sys_mutex_attribute_t`: protocol@0 u32, recursive@4 u32,
        // pshared@8 u32 (BE); ipc_key@16 u64 and flags@24 u32 are
        // read only for process-shared creates. Known divergence:
        // a null attr_ptr serves default attributes where the kernel
        // is EFAULT (RPCS3 sys_mutex.cpp sys_mutex_create); the
        // testkit relies on the null-attr default create.
        let attrs = if attr_ptr == 0 {
            MutexAttrs::default()
        } else if let Some(bytes) = rt.read_committed(attr_ptr as u64, 12) {
            let protocol = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let recursive_raw = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            let pshared = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
            // Validation ladder and errnos follow RPCS3 sys_mutex.cpp
            // sys_mutex_create: protocol then recursive in the syscall
            // body, pshared / ipc_key / flags inside lv2_obj::create
            // (sys_sync.h); every unknown enumerant is EINVAL.
            match protocol {
                cellgov_ps3_abi::sys_sync::SYS_SYNC_FIFO
                | cellgov_ps3_abi::sys_sync::SYS_SYNC_PRIORITY => {}
                // SYS_SYNC_PRIORITY_INHERIT (RPCS3 sys_sync.h); the
                // enumerant is not yet in cellgov_ps3_abi::sys_sync.
                0x3 => {}
                _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            }
            // Only the RECURSIVE enumerant enables re-locking;
            // SYS_SYNC_NOT_RECURSIVE (0x20) is nonzero but means
            // NOT recursive (RPCS3 sys_mutex.cpp sys_mutex_create
            // accepts exactly these two values).
            match recursive_raw {
                cellgov_ps3_abi::sys_sync::SYS_SYNC_RECURSIVE
                | cellgov_ps3_abi::sys_sync::SYS_SYNC_NOT_RECURSIVE => {}
                _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            }
            match pshared {
                cellgov_ps3_abi::sys_sync::SYS_SYNC_PROCESS_SHARED => {
                    // Process-shared creates carry an ipc_key and an
                    // attach policy; a zero key or an attach flag
                    // outside NEWLY_CREATED / NOT_CREATE / NOT_CARE
                    // (1 / 2 / 3, RPCS3 sys_sync.h) is EINVAL
                    // (RPCS3 sys_sync.h lv2_obj::create).
                    let Some(tail) = rt.read_committed(attr_ptr as u64 + 16, 12) else {
                        return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
                    };
                    let ipc_key = u64::from_be_bytes([
                        tail[0], tail[1], tail[2], tail[3], tail[4], tail[5], tail[6], tail[7],
                    ]);
                    let flags = u32::from_be_bytes([tail[8], tail[9], tail[10], tail[11]]);
                    if ipc_key == 0 || !(1..=3).contains(&flags) {
                        return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
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
                // SYS_SYNC_NOT_PROCESS_SHARED (0x200, RPCS3
                // sys_sync.h); the enumerant is not yet in
                // cellgov_ps3_abi::sys_sync.
                0x200 => {}
                _ => return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into()),
            }
            MutexAttrs {
                priority_policy: protocol,
                recursive: recursive_raw == cellgov_ps3_abi::sys_sync::SYS_SYNC_RECURSIVE,
                protocol,
            }
        } else {
            // The kernel reads the attribute struct unconditionally;
            // an unreadable attr pointer is a guest fault, not the
            // default-attribute arm (RPCS3 sys_mutex.cpp
            // sys_mutex_create dereferences attr before validating).
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let id = self.alloc_id();
        if self.state.mutexes.create_with_id(id, attrs).is_err() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    pub(super) fn dispatch_mutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.state.mutexes.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if entry.owner().is_some() || !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.state.mutexes.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_mutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.state.mutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::MutexAcquireOrEnqueue::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::Acquired
            | crate::sync_primitives::MutexAcquireOrEnqueue::Recursed => Lv2Dispatch::immediate(0),
            // A recursive re-lock whose count would overflow is
            // EKRESOURCE (RPCS3 sys_mutex.h lv2_mutex::try_lock).
            crate::sync_primitives::MutexAcquireOrEnqueue::CountSaturated => {
                Lv2Dispatch::immediate(cell_errors::CELL_EKRESOURCE.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into())
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
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.state.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_mutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        // An owner unlock with recursive holds outstanding consumes
        // one hold and keeps ownership; no waiter is granted until
        // the count reaches zero (RPCS3 sys_mutex.cpp
        // sys_mutex_unlock). Unknown-id and non-owner callers fall
        // through to the release path for ESRCH / EPERM.
        if self.state.mutexes.unlock_decrement(id, caller) {
            return Lv2Dispatch::immediate(0);
        }
        match self.state.mutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::MutexRelease::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexRelease::NotOwner => {
                self.obs.mutex_unlock_not_owner_count += 1;
                Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into())
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
