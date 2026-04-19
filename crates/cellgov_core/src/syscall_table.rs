//! Runtime-owned table of pending syscall responses.
//!
//! When a PPU blocks on a syscall (e.g., `sys_spu_thread_group_join`),
//! the LV2 host produces a `PendingResponse` describing what to do
//! when the block resolves. The runtime stores it here, keyed by the
//! blocked unit's `UnitId`. When the wake condition fires, the runtime
//! drains the entry, fills the PPU's r3, and (for join) writes the
//! cause/status out-pointers via `SharedWriteIntent` effects.
//!
//! There is exactly one place in the runtime where "what to do when
//! the PPU wakes up" lives. This is that place.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use std::collections::BTreeMap;

/// Pending-response table for blocked syscall callers.
///
/// Keyed by `UnitId` so the runtime can look up a response when a
/// unit is woken. At most one pending response per unit -- a unit
/// cannot be blocked on two syscalls simultaneously.
///
/// Participates in the runtime's `sync_state_hash` so replay is
/// sensitive to its contents.
#[derive(Debug, Clone, Default)]
pub struct SyscallResponseTable {
    pending: BTreeMap<UnitId, PendingResponse>,
}

impl SyscallResponseTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a pending response for `unit`. Overwrites any existing
    /// entry (a unit can only block on one syscall at a time).
    pub fn insert(&mut self, unit: UnitId, response: PendingResponse) {
        self.pending.insert(unit, response);
    }

    /// Remove and return the pending response for `unit`, if any.
    pub fn take(&mut self, unit: UnitId) -> Option<PendingResponse> {
        self.pending.remove(&unit)
    }

    /// Borrow the pending response for `unit` without removing it.
    pub fn peek(&self, unit: UnitId) -> Option<&PendingResponse> {
        self.pending.get(&unit)
    }

    /// Check whether `unit` has a pending response.
    pub fn contains(&self, unit: UnitId) -> bool {
        self.pending.contains_key(&unit)
    }

    /// Number of pending responses.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Iterate over the unit ids that have pending responses.
    pub fn pending_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.pending.keys().copied()
    }

    /// FNV-1a hash of the table contents for determinism checking.
    ///
    /// The hash covers every `(UnitId, PendingResponse)` pair in key
    /// order. An empty table returns the FNV offset basis.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (unit, response) in &self.pending {
            hasher.write(&unit.raw().to_le_bytes());
            match response {
                PendingResponse::ReturnCode { code } => {
                    hasher.write(&[0u8]);
                    hasher.write(&code.to_le_bytes());
                }
                PendingResponse::ThreadGroupJoin {
                    group_id,
                    code,
                    cause_ptr,
                    status_ptr,
                    cause,
                    status,
                } => {
                    hasher.write(&[1u8]);
                    hasher.write(&group_id.to_le_bytes());
                    hasher.write(&code.to_le_bytes());
                    hasher.write(&cause_ptr.to_le_bytes());
                    hasher.write(&status_ptr.to_le_bytes());
                    hasher.write(&cause.to_le_bytes());
                    hasher.write(&status.to_le_bytes());
                }
                PendingResponse::PpuThreadJoin {
                    target,
                    status_out_ptr,
                } => {
                    hasher.write(&[2u8]);
                    hasher.write(&target.to_le_bytes());
                    hasher.write(&status_out_ptr.to_le_bytes());
                }
                PendingResponse::EventQueueReceive {
                    out_ptr,
                    source,
                    data1,
                    data2,
                    data3,
                } => {
                    hasher.write(&[3u8]);
                    hasher.write(&out_ptr.to_le_bytes());
                    hasher.write(&source.to_le_bytes());
                    hasher.write(&data1.to_le_bytes());
                    hasher.write(&data2.to_le_bytes());
                    hasher.write(&data3.to_le_bytes());
                }
                PendingResponse::CondWakeReacquire {
                    mutex_id,
                    mutex_kind,
                } => {
                    hasher.write(&[4u8]);
                    hasher.write(&mutex_id.to_le_bytes());
                    hasher.write(&[*mutex_kind as u8]);
                }
                PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                } => {
                    hasher.write(&[5u8]);
                    hasher.write(&result_ptr.to_le_bytes());
                    hasher.write(&observed.to_le_bytes());
                }
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_table_is_empty() {
        let t = SyscallResponseTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn insert_and_take() {
        let mut t = SyscallResponseTable::new();
        let id = UnitId::new(5);
        let resp = PendingResponse::ReturnCode { code: 0 };
        t.insert(id, resp);
        assert!(t.contains(id));
        assert_eq!(t.len(), 1);
        let taken = t.take(id).unwrap();
        assert_eq!(taken, PendingResponse::ReturnCode { code: 0 });
        assert!(t.is_empty());
    }

    #[test]
    fn take_from_empty_returns_none() {
        let mut t = SyscallResponseTable::new();
        assert!(t.take(UnitId::new(0)).is_none());
    }

    #[test]
    fn peek_borrows_without_removing() {
        let mut t = SyscallResponseTable::new();
        let id = UnitId::new(5);
        t.insert(id, PendingResponse::ReturnCode { code: 42 });
        assert_eq!(t.peek(id), Some(&PendingResponse::ReturnCode { code: 42 }));
        assert!(t.contains(id));
    }

    #[test]
    fn insert_overwrites_previous() {
        let mut t = SyscallResponseTable::new();
        let id = UnitId::new(1);
        t.insert(id, PendingResponse::ReturnCode { code: 10 });
        t.insert(id, PendingResponse::ReturnCode { code: 20 });
        assert_eq!(t.len(), 1);
        let taken = t.take(id).unwrap();
        assert_eq!(taken, PendingResponse::ReturnCode { code: 20 });
    }

    #[test]
    fn multiple_units_independent() {
        let mut t = SyscallResponseTable::new();
        let a = UnitId::new(0);
        let b = UnitId::new(1);
        t.insert(a, PendingResponse::ReturnCode { code: 100 });
        t.insert(b, PendingResponse::ReturnCode { code: 200 });
        assert_eq!(t.len(), 2);
        assert_eq!(
            t.take(a).unwrap(),
            PendingResponse::ReturnCode { code: 100 }
        );
        assert_eq!(
            t.take(b).unwrap(),
            PendingResponse::ReturnCode { code: 200 }
        );
    }

    #[test]
    fn contains_returns_false_after_take() {
        let mut t = SyscallResponseTable::new();
        let id = UnitId::new(3);
        t.insert(id, PendingResponse::ReturnCode { code: 0 });
        assert!(t.contains(id));
        t.take(id);
        assert!(!t.contains(id));
    }

    #[test]
    fn state_hash_is_deterministic() {
        let mut a = SyscallResponseTable::new();
        let mut b = SyscallResponseTable::new();
        a.insert(UnitId::new(1), PendingResponse::ReturnCode { code: 42 });
        b.insert(UnitId::new(1), PendingResponse::ReturnCode { code: 42 });
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_content() {
        let mut a = SyscallResponseTable::new();
        let mut b = SyscallResponseTable::new();
        a.insert(UnitId::new(1), PendingResponse::ReturnCode { code: 1 });
        b.insert(UnitId::new(1), PendingResponse::ReturnCode { code: 2 });
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_empty_vs_populated_differ() {
        let empty = SyscallResponseTable::new();
        let mut populated = SyscallResponseTable::new();
        populated.insert(UnitId::new(0), PendingResponse::ReturnCode { code: 0 });
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn state_hash_covers_join_response() {
        let mut a = SyscallResponseTable::new();
        let mut b = SyscallResponseTable::new();
        a.insert(
            UnitId::new(0),
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 0,
                cause_ptr: 0x1000,
                status_ptr: 0x1004,
                cause: 1,
                status: 0,
            },
        );
        b.insert(
            UnitId::new(0),
            PendingResponse::ThreadGroupJoin {
                group_id: 1,
                code: 0,
                cause_ptr: 0x1000,
                status_ptr: 0x1004,
                cause: 2,
                status: 0,
            },
        );
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_stable_after_take() {
        let mut t = SyscallResponseTable::new();
        let empty_hash = t.state_hash();
        t.insert(UnitId::new(0), PendingResponse::ReturnCode { code: 0 });
        assert_ne!(t.state_hash(), empty_hash);
        t.take(UnitId::new(0));
        assert_eq!(t.state_hash(), empty_hash);
    }
}
