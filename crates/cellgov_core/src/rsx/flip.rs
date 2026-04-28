//! RSX flip-status state machine.
//!
//! Two terminal status values (`DONE = 0`, `WAITING = 1`) driven at
//! commit boundaries:
//!
//! - On `RsxFlipRequest` commit: `status = WAITING`, `pending = true`.
//! - On the next commit boundary: `status = DONE`, `pending = false`.
//!
//! Any PPU step between those two boundaries observes WAITING.
//! `handler` records the `cellGcmSetFlipHandler` address but PPU
//! dispatch into it is not modelled.

pub use cellgov_ps3_abi::cell_gcm::{
    CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
};

/// Hash-input shape version. Bump when [`RsxFlipState::state_hash`]
/// changes field order, endianness, or hasher family.
pub const STATE_HASH_FORMAT_VERSION: u8 = 1;

/// RSX flip-status state tracked across commit boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxFlipState {
    /// Current status byte; starts at `DONE`.
    status: u8,
    /// Callback address from `cellGcmSetFlipHandler`. Zero if none.
    /// Recorded only; PPU dispatch into it is not modelled.
    handler: u32,
    /// True between an `RsxFlipRequest` commit and the next commit
    /// boundary's DONE transition.
    pending: bool,
    /// Back-buffer index from the most-recent flip request.
    /// Last-writer-wins when multiple requests arrive before the
    /// next commit boundary.
    buffer_index: u8,
}

impl RsxFlipState {
    /// Pristine state: `status = DONE`, no handler, nothing pending.
    #[inline]
    pub const fn new() -> Self {
        Self {
            status: CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
            handler: 0,
            pending: false,
            buffer_index: 0,
        }
    }

    /// Current flip-status byte.
    #[inline]
    pub const fn status(&self) -> u8 {
        self.status
    }

    /// Registered flip-handler address; zero if none.
    #[inline]
    pub const fn handler(&self) -> u32 {
        self.handler
    }

    /// Whether a flip is pending the WAITING -> DONE transition.
    #[inline]
    pub const fn pending(&self) -> bool {
        self.pending
    }

    /// Back-buffer index from the most-recent flip request.
    /// Meaningful only while `pending()` is true.
    #[inline]
    pub const fn buffer_index(&self) -> u8 {
        self.buffer_index
    }

    /// Overwrite the full state.
    ///
    /// Legitimate callers: savestate restore and tests. Normal
    /// operation uses the typed mutators.
    #[inline]
    pub fn restore(&mut self, status: u8, handler: u32, pending: bool, buffer_index: u8) {
        self.status = status;
        self.handler = handler;
        self.pending = pending;
        self.buffer_index = buffer_index;
    }

    /// Record the flip-handler callback address.
    #[inline]
    pub fn set_handler(&mut self, addr: u32) {
        self.handler = addr;
    }

    /// Transition to WAITING on an `NV4097_FLIP_BUFFER` parse.
    /// A second request before completion overwrites `buffer_index`
    /// but keeps `pending` and `status` as WAITING.
    #[inline]
    pub fn request_flip(&mut self, buffer_index: u8) {
        self.status = CELL_GCM_DISPLAY_FLIP_STATUS_WAITING;
        self.pending = true;
        self.buffer_index = buffer_index;
    }

    /// Complete a pending flip at a commit boundary; no-op when
    /// `pending == false`. Must run one commit after the
    /// `RsxFlipRequest` so a PPU step can observe WAITING.
    /// `buffer_index` is preserved for state-hash stability.
    #[inline]
    pub fn complete_pending_flip(&mut self) -> bool {
        if self.pending {
            self.status = CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
            self.pending = false;
            true
        } else {
            false
        }
    }

    /// FNV-1a hash prefixed with [`STATE_HASH_FORMAT_VERSION`].
    /// Field order: status, handler (LE), pending, buffer_index.
    /// Folds into the runtime's sync-state hash.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&[STATE_HASH_FORMAT_VERSION]);
        hasher.write(&[self.status]);
        hasher.write(&self.handler.to_le_bytes());
        hasher.write(&[u8::from(self.pending)]);
        hasher.write(&[self.buffer_index]);
        hasher.finish()
    }
}

impl Default for RsxFlipState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_done_with_nothing_pending() {
        let s = RsxFlipState::new();
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
        assert_eq!(s.handler(), 0);
        assert!(!s.pending());
        assert_eq!(s.buffer_index(), 0);
    }

    #[test]
    fn request_flip_sets_waiting_and_records_buffer_index() {
        let mut s = RsxFlipState::new();
        s.request_flip(3);
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
        assert!(s.pending());
        assert_eq!(s.buffer_index(), 3);
    }

    #[test]
    fn second_request_overwrites_buffer_index_keeps_waiting() {
        let mut s = RsxFlipState::new();
        s.request_flip(1);
        s.request_flip(2);
        assert!(s.pending());
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
        assert_eq!(s.buffer_index(), 2);
    }

    #[test]
    fn complete_pending_transitions_to_done_exactly_once() {
        let mut s = RsxFlipState::new();
        s.request_flip(1);
        assert!(s.complete_pending_flip(), "first complete fires");
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
        assert!(!s.pending());
        assert!(
            !s.complete_pending_flip(),
            "second complete is a no-op (nothing pending)"
        );
    }

    #[test]
    fn complete_pending_on_fresh_state_is_noop() {
        let mut s = RsxFlipState::new();
        assert!(!s.complete_pending_flip());
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
    }

    #[test]
    fn set_handler_records_address_without_touching_status() {
        let mut s = RsxFlipState::new();
        s.set_handler(0xDEAD_BEEF);
        assert_eq!(s.handler(), 0xDEAD_BEEF);
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
        assert!(!s.pending());
    }

    #[test]
    fn restore_overwrites_all_fields() {
        let mut s = RsxFlipState::new();
        s.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0x1234_5678, true, 7);
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
        assert_eq!(s.handler(), 0x1234_5678);
        assert!(s.pending());
        assert_eq!(s.buffer_index(), 7);
    }

    #[test]
    fn state_hash_is_deterministic() {
        let a = RsxFlipState::new();
        let b = RsxFlipState::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_status() {
        let mut a = RsxFlipState::new();
        let mut b = RsxFlipState::new();
        b.request_flip(0);
        assert_ne!(a.state_hash(), b.state_hash());
        a.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0, true, 0);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_each_field() {
        fn hash_with(status: u8, handler: u32, pending: bool, buffer_index: u8) -> u64 {
            let mut s = RsxFlipState::new();
            s.restore(status, handler, pending, buffer_index);
            s.state_hash()
        }
        let base = hash_with(0, 0, false, 0);
        assert_ne!(base, hash_with(1, 0, false, 0), "status field folds in");
        assert_ne!(base, hash_with(0, 1, false, 0), "handler field folds in");
        assert_ne!(base, hash_with(0, 0, true, 0), "pending field folds in");
        assert_ne!(
            base,
            hash_with(0, 0, false, 1),
            "buffer_index field folds in"
        );
    }

    #[test]
    fn empty_flip_state_hash_golden() {
        let s = RsxFlipState::new();
        let got = s.state_hash();
        let mut h = cellgov_mem::Fnv1aHasher::new();
        h.write(&[STATE_HASH_FORMAT_VERSION]);
        h.write(&[CELL_GCM_DISPLAY_FLIP_STATUS_DONE]);
        h.write(&0u32.to_le_bytes());
        h.write(&[0u8]);
        h.write(&[0u8]);
        assert_eq!(got, h.finish());
    }

    #[test]
    fn complete_pending_returns_false_after_self_sequence() {
        let mut s = RsxFlipState::new();
        for i in 0..5u8 {
            s.request_flip(i);
            assert!(s.pending());
            assert!(s.complete_pending_flip());
            assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
            assert!(!s.pending());
            assert_eq!(
                s.buffer_index(),
                i,
                "buffer_index sticks across DONE; guest must not read it stale"
            );
        }
    }
}
