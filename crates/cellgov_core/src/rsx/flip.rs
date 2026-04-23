//! RSX flip-status state machine.
//!
//! Sony's `cellGcmSetFlip` emits a flip-buffer command into the RSX
//! FIFO. The guest observes completion by polling a flip-status byte.
//! The PS3 libgcm API defines two terminal values:
//!
//! - `CELL_GCM_DISPLAY_FLIP_STATUS_DONE = 0` -- no flip pending OR
//!   the previously-pending flip has completed.
//! - `CELL_GCM_DISPLAY_FLIP_STATUS_WAITING = 1` -- a flip has been
//!   issued but is still in flight.
//!
//! CellGov's oracle model transitions deterministically at commit
//! boundaries:
//!
//! - Initial: `status = DONE`, `pending = false`, `handler = 0`.
//! - On `RsxFlipRequest` commit: `status = WAITING`, `pending =
//!   true`, `buffer_index = <request's buffer>`.
//! - On the NEXT commit boundary after pending became true:
//!   `status = DONE`, `pending = false`. The intermediate WAITING
//!   observation is guaranteed for any PPU step that reads the
//!   status between those two boundaries.
//!
//! The flip-handler callback is not dispatched into PPU execution
//! today -- `handler` records the address; a later phase adds the
//! PPU-frame dispatch mechanism if a title needs it.
//!
//! This module holds the pure-data committed state. The transitions
//! are driven by the commit pipeline's per-boundary hook; the
//! `NV4097_FLIP_BUFFER` handler in `rsx_method` emits the requests.

/// Flip status byte value: no flip pending OR prior flip completed.
/// Source: RPCS3 `Emu/RSX/gcm_enums.h:44`
/// (`CELL_GCM_DISPLAY_FLIP_STATUS_DONE = 0`).
pub const CELL_GCM_DISPLAY_FLIP_STATUS_DONE: u8 = 0;

/// Flip status byte value: flip issued, not yet complete.
/// Source: RPCS3 `Emu/RSX/gcm_enums.h:45`
/// (`CELL_GCM_DISPLAY_FLIP_STATUS_WAITING = 1`).
pub const CELL_GCM_DISPLAY_FLIP_STATUS_WAITING: u8 = 1;

/// Format version byte prefixed into [`RsxFlipState::state_hash`].
/// Bump when the hash input shape changes (field order, endianness,
/// or hasher family). Same discipline as
/// [`crate::rsx::STATE_HASH_FORMAT_VERSION`]; the bump surfaces a
/// shape change at the golden-test boundary rather than letting it
/// masquerade as a "memory state differs" divergence.
pub const STATE_HASH_FORMAT_VERSION: u8 = 1;

/// RSX flip-status state tracked across commit boundaries.
///
/// Pure data. Mutation lives in a later slice (the `NV4097_FLIP_BUFFER`
/// handler sets `pending = true` and bumps status to WAITING; the
/// commit pipeline's per-boundary hook resolves the WAITING ->
/// DONE transition one batch later).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxFlipState {
    /// Current status byte. Starts at `DONE` so the first guest
    /// poll on a pristine runtime sees "nothing pending" -- matches
    /// real PS3 boot-time state.
    status: u8,
    /// Callback address registered via `cellGcmSetFlipHandler`.
    /// Zero means no handler registered. The address is RECORDED
    /// only; the runtime does not dispatch PPU execution into it.
    handler: u32,
    /// True between an `RsxFlipRequest` commit and the next commit
    /// boundary's DONE transition. A second `RsxFlipRequest` that
    /// commits while `pending == true` overwrites `buffer_index`
    /// (last-writer-wins) but leaves `pending`, `status` unchanged;
    /// the next boundary still resolves exactly one WAITING -> DONE.
    pending: bool,
    /// Back-buffer index the most-recent `RsxFlipRequest` asked to
    /// flip to. Recorded for observability and savestate
    /// completeness; the state machine itself only tracks
    /// `pending` / `status`.
    buffer_index: u8,
}

impl RsxFlipState {
    /// Construct the pristine state: `status = DONE`, no handler,
    /// nothing pending.
    #[inline]
    pub const fn new() -> Self {
        Self {
            status: CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
            handler: 0,
            pending: false,
            buffer_index: 0,
        }
    }

    /// Current flip-status byte. Observed by the guest through the
    /// RSX IO region read at the flip-status label address.
    #[inline]
    pub const fn status(&self) -> u8 {
        self.status
    }

    /// Callback address registered via `cellGcmSetFlipHandler`.
    /// Zero if no handler is registered.
    #[inline]
    pub const fn handler(&self) -> u32 {
        self.handler
    }

    /// Whether a flip is pending the WAITING -> DONE transition.
    /// Flipped to `true` by `RsxFlipRequest` and cleared by the
    /// next commit boundary's transition.
    #[inline]
    pub const fn pending(&self) -> bool {
        self.pending
    }

    /// Back-buffer index the most-recent flip request targeted.
    /// Meaningful only while `pending()` is true; otherwise stale.
    #[inline]
    pub const fn buffer_index(&self) -> u8 {
        self.buffer_index
    }

    /// Overwrite the full state. Exposed for the savestate-restore
    /// path and for tests that script specific shapes. The
    /// `NV4097_FLIP_BUFFER` handler and the per-boundary transition
    /// use the typed mutators below rather than clobbering via this
    /// method.
    #[inline]
    pub fn restore(&mut self, status: u8, handler: u32, pending: bool, buffer_index: u8) {
        self.status = status;
        self.handler = handler;
        self.pending = pending;
        self.buffer_index = buffer_index;
    }

    /// Record the flip-handler callback address. Called by
    /// `cellGcmSetFlipHandler`. The runtime only records; PPU
    /// dispatch into the handler is not modelled.
    #[inline]
    pub fn set_handler(&mut self, addr: u32) {
        self.handler = addr;
    }

    /// Transition to the WAITING state on an incoming
    /// `NV4097_FLIP_BUFFER` parse. If a flip was already pending,
    /// `buffer_index` overwrites (last-writer-wins when multiple
    /// requests arrive before the next commit boundary); `pending`
    /// stays `true` and `status` stays `WAITING` across the
    /// overwrite.
    #[inline]
    pub fn request_flip(&mut self, buffer_index: u8) {
        self.status = CELL_GCM_DISPLAY_FLIP_STATUS_WAITING;
        self.pending = true;
        self.buffer_index = buffer_index;
    }

    /// Complete a pending flip at a commit boundary. No-op when
    /// `pending == false`. Called by the commit pipeline's per-
    /// boundary hook one commit AFTER the `RsxFlipRequest` was
    /// applied, guaranteeing a PPU step between the two boundaries
    /// can observe the WAITING state. After the call: `status =
    /// DONE`, `pending = false`. `buffer_index` stays put for
    /// observability (state-hash-stable; the guest should not poll
    /// it after DONE).
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

    /// FNV-1a hash of the flip state, prefixed with
    /// [`STATE_HASH_FORMAT_VERSION`]. Field order: status (1 byte),
    /// handler (4 bytes LE), pending (1 byte: 0 / 1), buffer_index
    /// (1 byte). Folds into the runtime's sync-state hash alongside
    /// the FIFO cursor.
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
        // Corner case: a second request that arrives while
        // pending==true overwrites buffer_index, keeps pending and
        // WAITING. The next commit boundary still resolves one
        // WAITING -> DONE transition, not two.
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
        // Reset a matching way; hashes re-converge.
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
        // Pins the FNV-1a hash of an empty flip state. Any shape
        // change that does not bump STATE_HASH_FORMAT_VERSION
        // breaks this test first, localising the failure to the
        // hash input shape rather than a downstream memory-diff
        // divergence.
        //
        // Input bytes: [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        // 0x00] (version, status, handler LE x4, pending,
        // buffer_index). Compute via cellgov_mem::Fnv1aHasher's
        // canonical 64-bit constants.
        let s = RsxFlipState::new();
        let got = s.state_hash();
        // Recompute via the hasher directly so the golden is
        // derived, not hard-coded -- the only way this test breaks
        // is if state_hash's input bytes change. That is exactly
        // what we want the version bump to catch.
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
        // Scripted determinism: request + complete repeats
        // N times, each cycle returns state to identical values.
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
