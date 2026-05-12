//! Per-unit pending message / syscall-return / register-write buffers.
//!
//! Producers (the commit pipeline and HLE dispatch) deposit values here
//! addressed by `UnitId`; the runtime drains the relevant entries into
//! the unit's `ExecutionContext` at the start of its next step.

use cellgov_event::UnitId;

use super::UnitRegistry;

impl UnitRegistry {
    /// Push a received mailbox message into the unit's inbox.
    ///
    /// Silently drops writes to unknown ids (debug-assert first) to
    /// keep the pending map registry-consistent.
    pub fn push_receive(&mut self, id: UnitId, message: u32) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "push_receive for unknown UnitId {id:?} (would leak into pending_receives)"
            );
            return;
        }
        self.pending_receives.entry(id).or_default().push(message);
    }

    /// Drain all pending receives for `id` in push order.
    ///
    /// The `is_empty()` guard short-circuits the common no-pending case
    /// and assumes single-threaded access (guaranteed by `&mut self`
    /// today); remove the guard if this method moves behind a lock.
    #[inline]
    pub fn drain_receives(&mut self, id: UnitId) -> Vec<u32> {
        if self.pending_receives.is_empty() {
            return Vec::new();
        }
        self.pending_receives.remove(&id).unwrap_or_default()
    }

    /// Store a syscall return code for `id`. Unknown-id policy matches
    /// [`Self::push_receive`].
    pub fn set_syscall_return(&mut self, id: UnitId, code: u64) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "set_syscall_return for unknown UnitId {id:?} \
                 (would leak into pending_syscall_returns)"
            );
            return;
        }
        self.pending_syscall_returns.insert(id, code);
    }

    /// Drain the pending syscall return for `id`. Guard behaviour per
    /// [`Self::drain_receives`].
    #[inline]
    pub fn drain_syscall_return(&mut self, id: UnitId) -> Option<u64> {
        if self.pending_syscall_returns.is_empty() {
            return None;
        }
        self.pending_syscall_returns.remove(&id)
    }

    /// Queue a register write for the next step of `id`. Unknown-id
    /// policy matches [`Self::push_receive`].
    pub fn push_register_write(&mut self, id: UnitId, reg: u8, value: u64) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "push_register_write for unknown UnitId {id:?} \
                 (would leak into pending_register_writes)"
            );
            return;
        }
        self.pending_register_writes
            .entry(id)
            .or_default()
            .push((reg, value));
    }

    /// Drain pending register writes for `id`. Guard behaviour per
    /// [`Self::drain_receives`].
    #[inline]
    pub fn drain_register_writes(&mut self, id: UnitId) -> Vec<(u8, u64)> {
        if self.pending_register_writes.is_empty() {
            return Vec::new();
        }
        self.pending_register_writes.remove(&id).unwrap_or_default()
    }
}
